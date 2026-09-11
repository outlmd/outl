//! The GUI open path raises the client's clock above the op log.
//!
//! A GUI's `HlcGenerator` is built in `setup()`, **before** a workspace is
//! picked, so it cannot be seeded where it is constructed. The seeding has
//! to happen in [`open_workspace_at`] — and because a GUI can close one
//! workspace and open another, on *every* open rather than once at start.
//!
//! What goes wrong without it is a cost, not a divergence: a generator
//! built by `HlcGenerator::new` knows nothing about the ops on disk, so
//! after the wall clock moves backwards between two runs it issues
//! timestamps that sort below the log, and each one forces the paper's
//! undo/redo window over every newer entry — synchronously, on whatever
//! keystroke produced it.
//!
//! A log stamped into the far future stands in for the backwards clock:
//! indistinguishable to the generator, and the only one of the two that is
//! testable without touching the system clock.

use std::path::Path;

use outl_core::fractional::Fractional;
use outl_core::hlc::{Hlc, HlcGenerator};
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_tauri_shared::workspace_open::{open_workspace_at, WorkspaceGuards};
use parking_lot::Mutex;
use tempfile::TempDir;

/// A client's guard slot, the way `AppState` holds it.
type Guards = Mutex<Option<WorkspaceGuards>>;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_millis() as u64
}

/// Seed `ops/ops-<actor>.jsonl` with one `Create` stamped `ms`.
fn seed_log_at(root: &Path, actor: ActorId, ms: u64, node_seed: &str) {
    let op = LogOp {
        ts: Hlc::new(ms, 0, actor),
        actor,
        op: Op::Create {
            node: NodeId::from_seed(b"gui-clock-seed:", node_seed),
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    };
    let ops_dir = root.join("ops");
    std::fs::create_dir_all(&ops_dir).expect("mkdir ops");
    let line = serde_json::to_string(&op).expect("serialize");
    std::fs::write(
        ops_dir.join(format!("ops-{actor}.jsonl")),
        format!("{line}\n"),
    )
    .expect("write seed log");
}

/// The headline case: the client's generator comes back above a log
/// written ahead of this machine's wall clock.
#[test]
fn the_gui_open_seeds_the_clock_from_the_op_log() {
    let dir = TempDir::new().expect("tempdir");
    let actor = ActorId::new();
    let ahead = now_ms() + outl_core::hlc::MAX_CLOCK_SKEW_MS / 2;
    seed_log_at(dir.path(), actor, ahead, "ahead");

    let held: Guards = Mutex::new(None);
    let hlc = HlcGenerator::new(actor);
    let _ws = open_workspace_at(actor, &hlc, dir.path(), 0, &held).expect("open");

    let issued = hlc.next();
    assert!(
        issued.physical_ms >= ahead,
        "the GUI open issued {issued:?}, below the {ahead} already in the log"
    );
}

/// A GUI can close one workspace and open another, so the seeding is per
/// open — not a one-shot at app start. Opening a second workspace whose
/// log is further ahead must raise the same generator again.
#[test]
fn every_open_seeds_again_not_just_the_first() {
    let first = TempDir::new().expect("tempdir");
    let second = TempDir::new().expect("tempdir");
    let actor = ActorId::new();
    let near = now_ms() + outl_core::hlc::MAX_CLOCK_SKEW_MS / 4;
    let far = now_ms() + outl_core::hlc::MAX_CLOCK_SKEW_MS * 3 / 4;
    seed_log_at(first.path(), actor, near, "near");
    seed_log_at(second.path(), actor, far, "far");

    let held: Guards = Mutex::new(None);
    let hlc = HlcGenerator::new(actor);

    let ws = open_workspace_at(actor, &hlc, first.path(), 0, &held).expect("open first");
    assert!(hlc.next().physical_ms >= near, "first open must seed");
    // Release the first workspace's per-actor write lock before the
    // second open asks for it under the same actor.
    drop(ws);

    let _ws = open_workspace_at(actor, &hlc, second.path(), 0, &held).expect("open second");
    let issued = hlc.next();
    assert!(
        issued.physical_ms >= far,
        "the second open issued {issued:?}, below that workspace's {far} — a GUI that seeds only \
         once is unseeded for every workspace it opens afterwards"
    );
}

/// Reopening the same workspace never rewinds: the clock must come back
/// above every op the previous session wrote.
///
/// Stated over a log that is **ahead of the wall clock**, so it is a real
/// assertion rather than a restatement of "time passes", and compared
/// against the log's own tail rather than a previous generator reading —
/// two readings taken inside one millisecond legitimately tie.
#[test]
fn reopening_never_rewinds_the_clock() {
    let dir = TempDir::new().expect("tempdir");
    let actor = ActorId::new();
    let ahead = now_ms() + outl_core::hlc::MAX_CLOCK_SKEW_MS / 2;
    seed_log_at(dir.path(), actor, ahead, "ahead");

    let held: Guards = Mutex::new(None);

    // First session: its own generator, the way a fresh app launch has one.
    let written = {
        let hlc = HlcGenerator::new(actor);
        let mut ws = open_workspace_at(actor, &hlc, dir.path(), 0, &held).expect("open");
        let page = outl_actions::open_today(&mut ws, &hlc).expect("open today");
        outl_actions::append_block(&mut ws, &hlc, Some(page), Some("first")).expect("append block");
        ws.log().last().expect("the session wrote something").ts
    };
    assert!(
        written.physical_ms >= ahead,
        "the first session stamped {written:?} below the {ahead} already logged, so the reopen \
         assertion below would pin nothing"
    );
    // Release the per-actor write lock before the second session takes it.
    *held.lock() = None;

    let hlc = HlcGenerator::new(actor);
    let _ws = open_workspace_at(actor, &hlc, dir.path(), 0, &held).expect("reopen");
    let reopened = hlc.next();
    assert!(
        reopened > written,
        "reopen issued {reopened:?}, at or below the {written:?} the previous session wrote"
    );
}
