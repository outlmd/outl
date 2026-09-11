//! Snapshot sync — peer-to-peer materialized-snapshot transfer (Phase 2).
//!
//! A freshly-paired device that would otherwise have to receive (and replay) a
//! huge op log — 76 MB / 200k+ ops on a real vault — instead pulls a peer's
//! materialized *snapshot* (`snap-<actor>.bin`, ~5× smaller: settled state, no
//! `Edit` history) and boots from it via
//! [`outl_core::snapshot::read_best_from_disk`]. This module is the TRANSPORT
//! half only; the boot-adoption half (reading + replaying the delta over the
//! adopted snapshot) is already done in `outl-core` and never touched here.
//!
//! - [`SnapshotProtocolHandler`] — the responder, mounted on the live sync
//!   endpoint's router under [`SNAPSHOT_ALPN`] (the SAME endpoint — one endpoint
//!   per identity). It authorizes the dialer against `peers.json` via
//!   [`crate::authz`] (the same verdict `SYNC_ALPN` uses — this frame is the
//!   whole workspace, so an unauthorized dialer must never reach the disk read),
//!   then ships THIS device's own `snap-<self.actor>.bin` as one length-prefixed
//!   frame (an empty frame when it has no snapshot yet, which the peer skips).
//!   It never holds the workspace lock — it reads a cache file straight off
//!   disk, and the transport has no `Workspace` anyway.
//! - [`pull_snapshot_from_peer`] — the initiator, fired from
//!   [`crate::engine_pairing::drain_pair_completions`] right after the immediate
//!   delta-sync. It dials the peer, reads the frame, and (when non-empty) writes
//!   the snapshot to `<root>/.outl/snapshots/snap-<peer-actor>.bin`, then fires
//!   the reload signal so the next boot/reload adopts it.
//!
//! The snapshot cache lives in the DOTFILE dir (`.outl/snapshots/`) — never on
//! the file-sync surface — so this transfer is the only way it crosses devices.
//! That is correct: the snapshot is a local boot cache, not source of truth (the
//! op log is), so a corrupt / stale / absent snapshot is always safe to skip.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use outl_core::id::ActorId;
use outl_core::snapshot::{self, SnapshotBody};
use tracing::{debug, info, warn};

use outl_actions::SyncProgress;

use crate::engine_sync::read_frame_reporting;
use crate::protocol::{encode_blob_frame, SNAPSHOT_ALPN};

/// Bound on a single snapshot connect attempt. Mirrors the sync path's
/// `CONNECT_TIMEOUT`: iroh 1.0.0 multipath can stall ~30s on a dead direct addr,
/// so each attempt is capped and the bare-id (relay/discovery) fallback takes
/// over.
const SNAPSHOT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The client's request marker is an empty frame (4 bytes); bound the read so a
/// misbehaving peer can't stream forever before we serve.
const MAX_REQUEST_BYTES: usize = 1024;

/// `<root>/.outl/snapshots` — the local (dotfile) snapshot cache dir, matching
/// `outl_core`'s on-disk layout. Never on the file-sync surface.
fn snapshots_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".outl").join("snapshots")
}

/// Router handler that serves THIS device's own materialized snapshot to a
/// dialing peer.
#[derive(Clone)]
pub(crate) struct SnapshotProtocolHandler {
    /// Local workspace root, so the handler can resolve
    /// `<root>/.outl/snapshots/snap-<actor>.bin`.
    pub(crate) workspace_root: PathBuf,
    /// This device's actor id — the snapshot it owns and serves.
    pub(crate) actor: ActorId,
}

impl std::fmt::Debug for SnapshotProtocolHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnapshotProtocolHandler")
            .field("workspace_root", &self.workspace_root)
            .field("actor", &self.actor)
            .finish()
    }
}

impl ProtocolHandler for SnapshotProtocolHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        if let Err(e) = self.serve(conn).await {
            warn!("snapshot serve failed: {e:#}");
            return Err(AcceptError::from_boxed(e.into()));
        }
        Ok(())
    }
}

impl SnapshotProtocolHandler {
    /// Read our own `snap-<self.actor>.bin` off disk and write it back as one
    /// length-prefixed frame. Absent snapshot → an empty frame (the peer skips).
    async fn serve(&self, conn: Connection) -> Result<()> {
        // `snap-<actor>.bin` IS the materialized workspace — every page, settled.
        // Until this line it went to ANY dialer that spoke the ALPN: no peer
        // check, no workspace check. `outl peer remove` took the revoked device
        // off `SYNC_ALPN` and left this door open, so the device kept a complete,
        // current copy of the graph. Authorize FIRST, before a byte is read off
        // disk. Same owner as the sync handler's check — see `crate::authz`.
        if !crate::authz::authorize_or_close(&conn, &self.workspace_root).await {
            return Ok(());
        }

        let (mut send, mut recv) = conn
            .accept_bi()
            .await
            .context("accept snapshot bi stream")?;

        // Drain the client's request marker so the stream is established. Its
        // CONTENT is irrelevant — the ALPN itself means "send me your snapshot"
        // (a device always serves its own actor's snapshot). Bounded read so a
        // peer can't stream forever.
        let _ = recv.read_to_end(MAX_REQUEST_BYTES).await;

        // Read straight off disk — no workspace lock (this is a cache file, and
        // the transport holds no `Workspace`).
        let path = snapshots_dir(&self.workspace_root).join(format!("snap-{}.bin", self.actor));
        let bytes = match tokio::fs::read(&path).await {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                return Err(anyhow::Error::new(e))
                    .with_context(|| format!("read snapshot {}", path.display()))
            }
        };

        let frame = encode_blob_frame(&bytes)?;
        send.write_all(&frame)
            .await
            .context("send snapshot frame")?;
        send.finish().context("finish snapshot send")?;
        // Wait for the client to finish reading before the endpoint tears the
        // connection down (mirrors the pairing handler's `conn.closed()`).
        conn.closed().await;
        Ok(())
    }
}

/// Connect to `peer` on [`SNAPSHOT_ALPN`], resilient to a stale direct address.
///
/// Mirrors `engine_sync::connect_with_fallback` but for the snapshot ALPN: try
/// the full addr (fast on-LAN), fall back to the bare node id via relay /
/// discovery if that stalls or fails and the addr carried a (possibly dead)
/// direct addr.
async fn connect_snapshot(
    endpoint: &iroh::Endpoint,
    peer_addr: iroh::EndpointAddr,
) -> Result<Connection> {
    let node_id = peer_addr.id;
    let had_direct = peer_addr.ip_addrs().next().is_some();

    match tokio::time::timeout(
        SNAPSHOT_CONNECT_TIMEOUT,
        endpoint.connect(peer_addr, SNAPSHOT_ALPN),
    )
    .await
    {
        Ok(Ok(conn)) => return Ok(conn),
        Ok(Err(e)) if !had_direct => return Err(e).context("snapshot connect"),
        Err(_) if !had_direct => return Err(anyhow::anyhow!("snapshot connect timed out")),
        Ok(Err(e)) => debug!(
            "direct snapshot connect to {} failed ({e}); retrying via relay/discovery",
            node_id.fmt_short()
        ),
        Err(_) => debug!(
            "direct snapshot connect to {} timed out; retrying via relay/discovery",
            node_id.fmt_short()
        ),
    }

    tokio::time::timeout(
        SNAPSHOT_CONNECT_TIMEOUT,
        endpoint.connect(node_id, SNAPSHOT_ALPN),
    )
    .await
    .context("snapshot relay/discovery connect timed out")?
    .context("snapshot connect (relay/discovery)")
}

/// Pull `peer`'s materialized snapshot and cache it locally, firing the reload
/// signal so boot adopts it. Best-effort: any failure returns `Ok(false)` or an
/// `Err` the caller logs, never corrupting local state.
///
/// Returns `Ok(true)` when a snapshot was received and written, `Ok(false)` when
/// the peer had none (empty frame) or its snapshot was undecodable (skipped —
/// the boot side falls back to full replay regardless).
pub(crate) async fn pull_snapshot_from_peer(
    endpoint: &iroh::Endpoint,
    peer: iroh::EndpointAddr,
    workspace_root: &Path,
    peer_ready_tx: &std::sync::mpsc::Sender<()>,
    progress: &crate::progress::ProgressSink,
) -> Result<bool> {
    let peer_node_id = peer.id;
    let peer_short = peer_node_id.fmt_short().to_string();
    let conn = connect_snapshot(endpoint, peer).await?;
    let (mut send, mut recv) = conn.open_bi().await.context("open snapshot bi stream")?;

    // Request marker: an empty frame is enough to establish the stream on the
    // responder, which serves its own snapshot regardless of the content.
    send.write_all(&encode_blob_frame(&[])?)
        .await
        .context("send snapshot request")?;
    send.finish().context("finish snapshot request")?;

    // Read the responder's snapshot frame (empty when it has none). `read_frame`
    // returns `[prefix || body]`; strip the 4-byte prefix to get the body. This
    // is the multi-MB transfer that dominates a pair, so report byte progress
    // (throttled to ~256 KiB steps to keep the event stream light) for the UI
    // bar — the only sync phase with an honest total.
    let mut last_emitted = 0u64;
    let frame = read_frame_reporting(&mut recv, |received, total| {
        if total > 0
            && (received == 0 || received >= total || received - last_emitted >= 256 * 1024)
        {
            last_emitted = received;
            progress.emit(SyncProgress::Snapshot {
                peer: peer_short.clone(),
                received,
                total,
            });
        }
    })
    .await
    .context("read snapshot frame")?;
    conn.close(0u32.into(), b"done");

    let body_bytes = &frame[4..];
    if body_bytes.is_empty() {
        debug!(
            "snapshot pull: peer {} has no snapshot",
            peer_node_id.fmt_short()
        );
        return Ok(false);
    }

    // Decode to VALIDATE integrity and learn the snapshot's actor id — the cache
    // filename must be `snap-<actor>.bin` so boot's `read_best_from_disk` adopts
    // it. A corrupt / future-schema snapshot is skipped (never written): the
    // boot side would fall back to full replay anyway, and a bad cache file must
    // never land. Decode is cheap relative to the multi-MB transfer.
    let body = match SnapshotBody::decode(body_bytes) {
        Ok(b) => b,
        Err(e) => {
            warn!(
                "snapshot pull from {}: undecodable snapshot skipped ({e})",
                peer_node_id.fmt_short()
            );
            return Ok(false);
        }
    };
    // A snapshot with NO per-actor cutoff is not merely useless — it is
    // unattributable, and `outl_core`'s adoption guard is written as
    // `if let Some(max) = body.cutoff.values().max()`, so an EMPTY cutoff makes
    // that guard not run at all. The body is an opaque materialized tree the
    // peer chose; adopting it unchecked lets a forged one install its own
    // `nodes` / `block_text` and then replay our entire log on top of it (every
    // actor absent from the cutoff means "we have seen none of your ops"),
    // which is the divergence the guard exists to prevent and, since RFC 0263,
    // also re-arms the duplicate-`Create` defect that RFC fixed.
    //
    // `build_snapshot_body` returns `None` rather than an empty cutoff, so no
    // honest peer can produce this and refusing it costs nothing. `outl-core`
    // owns the authoritative refusal — a snapshot can also arrive by a file
    // transport or a restored backup, paths this function never sees — but a
    // poisoned cache file is cheaper to never write than to fall back from.
    if body.cutoff.is_empty() {
        warn!(
            "snapshot pull from {}: refusing a snapshot with no per-actor cutoff",
            peer_node_id.fmt_short()
        );
        return Ok(false);
    }
    let received_len = body_bytes.len();

    // Atomic write (tmp + fsync + rename) via the format's owner in `outl-core`.
    // Re-encoding the validated body is byte-identical to the received bytes
    // (the encoding is deterministic — the whole `content_hash` integrity
    // design relies on it) and names the file `snap-<body.actor>.bin`. Runs on a
    // blocking thread — it fsyncs a multi-MB file.
    let dir = snapshots_dir(workspace_root);
    let written = tokio::task::spawn_blocking(move || snapshot::write_to_disk(&dir, &body)).await;
    match written {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            warn!("snapshot pull: persist failed: {e}");
            return Ok(false);
        }
        Err(e) => return Err(anyhow::Error::new(e).context("join snapshot write task")),
    }

    info!(
        "snapshot pull: adopted peer {}'s snapshot ({received_len} bytes)",
        peer_node_id.fmt_short()
    );
    // Fire the reload so boot/reload adopts the freshly written snapshot.
    peer_ready_tx.send(()).ok();
    Ok(true)
}
