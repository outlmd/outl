//! `peer_status` — what the always-visible sync dot is allowed to cost.
//!
//! The dot polls this on the catch-up cadence **and** once per
//! `peer-ops-changed`, so during an active sync it runs several times a
//! second. Reachability is an in-memory read of the transport's own dial
//! outcomes; `pending_ops` is the only field that counts the op log, and
//! `pendingLabel` renders it in exactly one place — the Sync panel's
//! device list, which the dot is not.
//!
//! So `with_pending` is a real gate and not a hint. These tests pin it
//! **structurally**, the way `the_actor_census_neither_reads_nor_writes_the_node_index`
//! does: the census read goes through the op log's offset sidecar, so a
//! sidecar that appears is proof the op log was read. A timing assertion
//! would be flaky and would not say *what* was read.
//!
//! Root `CLAUDE.md` invariant 15, and `docs/performance.md`.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::SyncTransport;
use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::commands::peers::peer_status;
use outl_tauri_shared::host::AppHost;
use parking_lot::Mutex;
use tauri::async_runtime::block_on;
use tempfile::TempDir;

struct TestHost {
    workspace: Arc<Mutex<Option<Workspace>>>,
    hlc: HlcGenerator,
    root: PathBuf,
    registry: Arc<RuntimeRegistry>,
}

impl AppHost for TestHost {
    fn workspace(&self) -> &Mutex<Option<Workspace>> {
        &self.workspace
    }
    fn workspace_arc(&self) -> Arc<Mutex<Option<Workspace>>> {
        self.workspace.clone()
    }
    fn hlc(&self) -> &HlcGenerator {
        &self.hlc
    }
    fn storage_root(&self) -> Result<PathBuf, String> {
        Ok(self.root.clone())
    }
    fn sync_transport(&self) -> Option<Arc<dyn SyncTransport>> {
        None
    }
    fn exec_registry(&self) -> Arc<RuntimeRegistry> {
        self.registry.clone()
    }
}

/// A workspace with an op log on disk and one paired peer carrying a
/// recorded floor — the state in which `pending_ops` has a number to
/// report, so skipping the read is a choice rather than an accident.
fn host_with_a_peer_and_a_log() -> (TempDir, TestHost) {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path().to_path_buf();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);

    // An op log a census would have something to count.
    let ops_dir = root.join("ops");
    std::fs::create_dir_all(&ops_dir).expect("ops dir");
    let gen = HlcGenerator::new(actor);
    let body: String = (0..8)
        .map(|_| {
            let ts = gen.next();
            let op = LogOp {
                ts,
                actor,
                op: Op::Create {
                    node: NodeId::new(),
                    parent: NodeId::root(),
                    position: Fractional::first(),
                },
            };
            format!("{}\n", serde_json::to_string(&op).expect("encode"))
        })
        .collect();
    std::fs::write(ops_dir.join(format!("ops-{actor}.jsonl")), body).expect("write log");

    // One paired peer, and a floor for it — `pending_by_peer` returns
    // early when no peer has ever exchanged, which would make both
    // branches of `with_pending` look identical.
    let outl_dir = root.join(".outl");
    std::fs::create_dir_all(outl_dir.join("state")).expect("state dir");
    std::fs::write(
        outl_dir.join("peers.json"),
        r#"{"peers":[{"node_id":"b75b9083739f4a0c8a1f6f2e5d4c3b2a19087766554433221100ffeeddccbbaa","alias":"laptop","added_at":"2026-01-01T00:00:00Z"}]}"#,
    )
    .expect("peers.json");
    std::fs::write(
        outl_dir.join("state").join("sync-pending.json"),
        format!(
            r#"{{"b75b9083739f4a0c8a1f6f2e5d4c3b2a19087766554433221100ffeeddccbbaa":{{"floor":{{"{actor}":3}},"seen_ms":1700000000000}}}}"#
        ),
    )
    .expect("sync-pending.json");

    let ws = Workspace::open_in_memory(actor).expect("workspace");
    let host = TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root,
        registry: Arc::new(RuntimeRegistry::new()),
    };
    (tmp, host)
}

/// Every `.ops-*.idx` beside the op log. The census reads these, and a
/// cold directory has none — so one appearing is the op log having been
/// read, whatever the timing says.
fn offset_sidecars(root: &std::path::Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root.join("ops")) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(".ops-") && n.ends_with(".idx"))
        })
        .collect()
}

/// The status dot's call. It asks for reachability, which the transport
/// holds in memory, and must not reach the op log for a number it does
/// not render.
#[test]
fn the_status_dot_does_not_read_the_op_log() {
    let (tmp, host) = host_with_a_peer_and_a_log();

    let status = block_on(peer_status(&host, false)).expect("status");

    assert_eq!(status.len(), 1, "the paired peer is still listed");
    assert!(
        status[0].pending_ops.is_none(),
        "a caller that did not ask gets `None`, which already means \
         'nobody measured this' before a peer's first exchange"
    );
    assert!(
        offset_sidecars(tmp.path()).is_empty(),
        "an offset sidecar appeared: the dot's poll read the op log for a \
         number it never renders"
    );
}

/// The Sync panel's call. It renders the column, so it pays for it — and
/// the number has to be real, not a placeholder the cheap path left
/// behind.
#[test]
fn the_sync_panel_gets_the_number_it_renders() {
    let (_tmp, host) = host_with_a_peer_and_a_log();

    let status = block_on(peer_status(&host, true)).expect("status");

    assert_eq!(status.len(), 1);
    assert_eq!(
        status[0].pending_ops,
        Some(5),
        "8 ops held, the peer's floor is 3, so 5 are still owed to it"
    );
}

/// Both answers describe the same peers. The gate may drop a column; it
/// may not drop a device, which is what a client's list is keyed on.
#[test]
fn the_gate_changes_one_column_and_nothing_else() {
    let (_tmp, host) = host_with_a_peer_and_a_log();

    let cheap = block_on(peer_status(&host, false)).expect("cheap");
    let full = block_on(peer_status(&host, true)).expect("full");

    assert_eq!(cheap.len(), full.len());
    for (c, f) in cheap.iter().zip(&full) {
        assert_eq!(c.node_id, f.node_id);
        assert_eq!(c.alias, f.alias);
        assert_eq!(c.online, f.online);
        assert_eq!(c.rtt_ms, f.rtt_ms);
    }
}
