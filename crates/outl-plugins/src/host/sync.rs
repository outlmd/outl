//! The `sync-transport` plugin path: ops out, ops in.
//!
//! A transport plugin is trusted to **move bytes** and for nothing else. It
//! never sees a `Workspace`: push hands it JSONL, pull hands back JSONL, and
//! everything that decides what those bytes are allowed to become happens on
//! this side of the seam. That is why the two directions sit in one module —
//! they are one trust boundary, and reading either half without the other
//! hides which checks are actually load-bearing.
//!
//! Three of them are:
//!
//! - **Ops still go through the CRDT.** `sync_pull` calls `Workspace::apply`
//!   per line, so a malformed line is skipped rather than trusted into the tree
//!   raw, and a replayed line is idempotent.
//! - **Actors are not trusted.** `sync_push` ships only locally-authored ops,
//!   so an op injected from a peer never echoes back to the backend that sent
//!   it.
//! - **Clocks are not trusted.** An op past [`outl_core::hlc::MAX_CLOCK_SKEW_MS`]
//!   is dropped *before* `observe` — see [`PluginHost::sync_pull`] for why after
//!   is permanently too late. This is the crate's non-negotiable 7, and the
//!   gate itself lives in `outl-core`: a second copy of "how far ahead is too
//!   far" is the thing to refuse in review.

use outl_core::hlc::HlcGenerator;
use outl_core::op::LogOp;
use outl_core::workspace::Workspace;

use super::PluginHost;
use crate::capability::Capability;
use crate::error::{PluginError, Result};

impl PluginHost {
    /// Index of the first plugin granted the `sync-transport` capability, if any.
    fn sync_plugin(&self) -> Option<usize> {
        self.plugins
            .iter()
            .position(|p| p.has(Capability::SyncTransport))
    }

    /// Hand the sync-transport plugin the JSONL of **locally-authored** ops
    /// produced since the last push, so it can ship them to its backend.
    /// Returns how many ops were shipped. Ops injected from peers (via
    /// [`PluginHost::sync_pull`]) carry a foreign actor and are filtered out, so
    /// they never echo back.
    pub fn sync_push(&mut self, workspace: &Workspace) -> Result<usize> {
        let Some(idx) = self.sync_plugin() else {
            return Ok(0);
        };
        let local = workspace.actor;
        let lines: Vec<String> = workspace
            .log()
            .iter()
            .skip(self.last_pushed)
            .filter(|lo| lo.actor == local)
            .map(serde_json::to_string)
            .collect::<std::result::Result<_, _>>()?;
        self.last_pushed = workspace.log().len();
        if lines.is_empty() {
            return Ok(0);
        }
        let jsonl = lines.join("\n");
        let count = lines.len();
        self.prepare_secrets(idx);
        let config = self.plugins[idx].config.clone();
        self.plugins[idx]
            .engine
            .sync_push(&jsonl, &config)
            .map_err(|e| PluginError::Engine(e.to_string()))?;
        Ok(count)
    }

    /// Ask the sync-transport plugin for remote ops and apply each through
    /// `Workspace::apply` (HLC-observed, idempotent). The plugin only transports
    /// bytes — every op still goes through the CRDT, so a malformed line is
    /// skipped, never trusted into the tree raw. Returns how many applied.
    ///
    /// **An op's timestamp is not trusted either.** One past
    /// [`outl_core::hlc::MAX_CLOCK_SKEW_MS`] is dropped and logged before it can
    /// reach `observe`, because `observe` is monotonic: a timestamp from the
    /// year 584 million raises this device's clock and *keeps* it there, every
    /// op the device writes afterwards lands past its peers' own skew gate, and
    /// the device keeps working locally while silently syncing nothing it
    /// writes. No hostile plugin required — a backend reading from a server with
    /// a wrong clock is enough.
    pub fn sync_pull(&mut self, workspace: &mut Workspace, hlc: &HlcGenerator) -> Result<usize> {
        let Some(idx) = self.sync_plugin() else {
            return Ok(0);
        };
        self.prepare_secrets(idx);
        let config = self.plugins[idx].config.clone();
        let Some(jsonl) = self.plugins[idx]
            .engine
            .sync_pull(&config)
            .map_err(|e| PluginError::Engine(e.to_string()))?
        else {
            return Ok(0);
        };

        // Read once per batch, then judge every op against it — the same shape
        // `outl-sync-iroh`'s `ingest_received_ops` uses, calling the same gate.
        // A clock before the epoch cannot anchor "the future", and `None` there
        // accepts: applying an op the gate cannot judge is recoverable, while
        // refusing all sync is a silent black hole.
        let now_ms = outl_core::hlc::wall_clock_ms_checked();
        if now_ms.is_none() {
            tracing::warn!("local clock is before UNIX_EPOCH; skipping the future-HLC gate");
        }
        let mut applied = 0;
        for line in jsonl.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let op = match serde_json::from_str::<LogOp>(line) {
                Ok(op) => op,
                // Never panic, and never silently. `protocol::delta` logs the
                // same drop on the iroh side; a plugin transport shipping
                // garbage should not be quieter than the built-in one.
                Err(e) => {
                    tracing::warn!("skipping undecodable op ({} bytes): {e}", line.len());
                    continue;
                }
            };
            if let Some(ahead) = outl_core::hlc::skew_ahead_ms(op.ts, now_ms) {
                // The op's HLC + actor, so a dropped op is traceable rather
                // than "something 25h ahead vanished".
                tracing::warn!(
                    ts = ?op.ts,
                    actor = ?op.actor,
                    "skipping op with future HLC ({ahead}ms ahead)"
                );
                continue;
            }
            hlc.observe(op.ts); // advance the local clock so causality holds
            if workspace.apply(op).is_ok() {
                applied += 1;
            }
        }
        // Injected ops advanced the log but are foreign-actor, so sync_push
        // won't re-ship them; keep last_pushed in step so we don't rescan them.
        self.last_pushed = workspace.log().len();
        Ok(applied)
    }
}

#[cfg(all(test, feature = "js"))]
mod tests {
    use super::*;
    use crate::host::tests::ws;
    use crate::manifest::PluginManifest;
    use crate::permission::PermissionSet;
    use outl_actions::block;
    use outl_actions::page::{self, PageKind};
    use serde_json::Value;

    #[test]
    fn sync_transport_carries_ops_between_workspaces() {
        // A loopback transport: push stashes the JSONL in a global, pull returns it.
        // Stands in for "ship to backend / fetch from backend" without a network.
        const SYNC_PLUGIN: &str = r#"
        globalThis.__buf = '';
        globalThis.__outl_register({ activate(ctx) {
            ctx.sync.register({
                push: (jsonl) => { globalThis.__buf = jsonl; },
                pull: () => globalThis.__buf,
            });
        }});
    "#;
        let manifest = PluginManifest::parse(
            br#"{"id":"run.x.sync","name":"Sync","version":"1.0.0","api":"^1.0","main":"i.js",
             "capabilities":["sync-transport"]}"#,
        )
        .unwrap();
        let mut host = PluginHost::new([Capability::SyncTransport].into_iter().collect());
        host.load_plugin(
            manifest,
            SYNC_PLUGIN,
            PermissionSet::new(vec![]),
            Value::Null,
        )
        .unwrap();

        // Device 1 authors a page + block, then pushes its local ops to the transport.
        let (mut ws1, hlc1) = ws();
        let page =
            page::open_or_create(&mut ws1, &hlc1, "shared", "Shared", PageKind::Page).unwrap();
        block::create_under(&mut ws1, &hlc1, page, Some("hello from device 1")).unwrap();
        let pushed = host.sync_push(&ws1).unwrap();
        assert!(pushed >= 2, "pushed page + block ops, got {pushed}");

        // Device 2 starts empty; pulling applies device 1's ops through the CRDT.
        let (mut ws2, hlc2) = ws();
        assert!(page::find_by_slug(&ws2, "shared").is_none());
        let applied = host.sync_pull(&mut ws2, &hlc2).unwrap();
        assert!(applied >= 2, "applied the transported ops, got {applied}");
        assert!(
            page::find_by_slug(&ws2, "shared").is_some(),
            "page converged onto device 2"
        );
    }
}
