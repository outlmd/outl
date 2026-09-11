//! The GUI clients take the same two workspace locks every other `outl`
//! process takes.
//!
//! Before this existed the desktop and mobile clients took **neither**,
//! which made a running GUI invisible to the rest of the machine. The
//! sharpest consequence is compaction: `apply_compaction` asks "is anyone
//! in this workspace?" by taking `<root>/.outl/.lock` exclusively, and a
//! GUI holding nothing let that gate pass. Compaction then renamed a
//! rewritten `ops-<actor>.jsonl` under a live client that still held
//! in-memory byte offsets into the pre-compaction layout — "a silently
//! dropped op on every index-driven read", in compaction's own words.
//!
//! The refusal cases matter as much as the happy one, so they are pinned
//! in both directions: a GUI must block compaction, and it must **not**
//! block another `outl` process from opening the same workspace (the
//! workspace lock is shared by design — the historical exclusive variant
//! was a SQLite-era mistake).

use std::path::Path;

use outl_core::fractional::Fractional;
use outl_core::hlc::{Hlc, HlcGenerator};
use outl_core::id::{ActorId, NodeId};
use outl_core::lock::{ActorWriteLock, WorkspaceLock};
use outl_core::op::{LogOp, Op};
use outl_core::storage::compact::{
    apply_compaction, plan_compaction, CompactError, CompactOptions,
};
use outl_core::workspace::Workspace;
use outl_tauri_shared::workspace_open::{open_workspace_at, WorkspaceGuards};
use parking_lot::Mutex;
use tempfile::TempDir;

/// A client's guard slot. The real `AppState` holds this behind an `Arc`;
/// the sharing is irrelevant to what is under test.
type Guards = Mutex<Option<WorkspaceGuards>>;

fn open(root: &Path, actor: ActorId, held: &Guards) -> anyhow::Result<Workspace> {
    let hlc = HlcGenerator::new(actor);
    open_workspace_at(actor, &hlc, root, 0, held)
}

/// Seed `ops/ops-<actor>.jsonl` with a `Create` + placement-restating
/// `Move` pair — the one shape compaction is allowed to drop — so
/// `plan_compaction` has something to plan.
fn seed_compactable_log(root: &Path) {
    let actor = ActorId::new();
    let parent = NodeId::new();
    let node = NodeId::new();
    let position = Fractional::parse("am").expect("valid fractional");

    let ops = [
        LogOp {
            ts: Hlc::new(1_000, 0, actor),
            actor,
            op: Op::Create {
                node: parent,
                parent: NodeId::root(),
                position: position.clone(),
            },
        },
        LogOp {
            ts: Hlc::new(2_000, 0, actor),
            actor,
            op: Op::Create {
                node,
                parent,
                position: position.clone(),
            },
        },
        LogOp {
            ts: Hlc::new(3_000, 0, actor),
            actor,
            op: Op::Move {
                node,
                new_parent: parent,
                position,
                // Local undo bookkeeping; compaction must never read it.
                old_parent: NodeId::root(),
                old_position: Fractional::first(),
            },
        },
    ];

    let ops_dir = root.join("ops");
    std::fs::create_dir_all(&ops_dir).expect("mkdir ops");
    let body: String = ops
        .iter()
        .map(|op| format!("{}\n", serde_json::to_string(op).expect("serialize")))
        .collect();
    std::fs::write(ops_dir.join(format!("ops-{actor}.jsonl")), body).expect("write seed log");
}

/// `outl compact --no-horizon`: the horizon is about peers still holding
/// undelivered ops, which no fixture has.
fn plan_now(root: &Path) -> outl_core::storage::compact::CompactPlan {
    plan_compaction(root, &CompactOptions { horizon_ms: 0 }).expect("plan")
}

/// What `outl compact --apply` actually runs: plan, then rewrite.
///
/// Both halves take the exclusive workspace lock — the read half because
/// a log being appended to parses as a torn record, which would be
/// reported as a damaged log. So a caller asking "does a GUI stop
/// compaction?" has to ask it of the whole command, not of `apply` alone.
fn compact_now(root: &Path) -> Result<outl_core::storage::compact::CompactReport, CompactError> {
    let plan = plan_compaction(root, &CompactOptions { horizon_ms: 0 })?;
    apply_compaction(root, &plan)
}

// --------------------------------------------------------------------------
// The compaction gate
// --------------------------------------------------------------------------

/// The headline case. A GUI with the workspace open must make
/// `compact --apply` refuse, because compaction renames the very files
/// that client holds byte offsets into.
#[test]
fn a_running_gui_makes_compaction_refuse() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    seed_compactable_log(root);

    // Planned *before* the GUI attaches: `plan_compaction` takes the same
    // lock (reading a log that is being appended to parses as damage), so
    // planning under the GUI would refuse before `apply` could be asked.
    // This test is about the writing half.
    let plan = plan_now(root);
    assert!(
        !plan.is_empty(),
        "fixture must give compaction something to drop, or this pins nothing"
    );

    let held: Guards = Mutex::new(None);
    let _ws = open(root, ActorId::new(), &held).expect("gui opens the workspace");
    assert!(
        held.lock().is_some(),
        "opening through the GUI path must install the workspace guards"
    );

    match apply_compaction(root, &plan) {
        Err(CompactError::Busy(path)) => {
            assert!(
                path.ends_with(".lock"),
                "the refusal must name the workspace lock, got {}",
                path.display()
            );
        }
        other => panic!("compaction ran with a GUI holding the workspace: {other:?}"),
    }
}

/// The other half: the guards are a lease, not a wall. Once the client
/// closes the workspace, compaction proceeds.
#[test]
fn releasing_the_gui_lets_compaction_run() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    seed_compactable_log(root);

    let held: Guards = Mutex::new(None);
    let ws = open(root, ActorId::new(), &held).expect("gui opens the workspace");
    assert!(matches!(compact_now(root), Err(CompactError::Busy(_))));

    // Close the workspace the way a client does: drop the workspace and
    // clear the guard slot.
    drop(ws);
    *held.lock() = None;

    let report = compact_now(root).expect("compaction runs once nobody holds the workspace");
    assert!(
        report.ops_dropped > 0,
        "expected the seeded inert Move to be dropped, got {report:?}"
    );
}

// --------------------------------------------------------------------------
// The per-actor write lock
// --------------------------------------------------------------------------

/// `append_ops` states that it is the single writer for its actor file,
/// "guarded by `ActorWriteLock`". A GUI that took no such lock made that
/// precondition false. It is now enforced — and enforced by **refusing**,
/// not by silently sharing the file.
///
/// The ephemeral-actor fallback `resolve_write_actor` offers the CLI is
/// deliberately not taken here: a GUI's `HlcGenerator` actor is fixed
/// before a workspace is picked, so writing to a fresh file while still
/// stamping ops with the device actor would leave two live generators on
/// one actor id — identical HLCs, and `contains_ts` drops one of the two
/// ops. See the module docs in `workspace_open.rs`.
#[test]
fn a_second_process_on_the_same_actor_is_refused_not_silently_shared() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    let actor = ActorId::new();

    // Stand in for the other process holding this actor's write lock.
    let _other = ActorWriteLock::try_acquire(&root.join("ops"), actor).expect("first holder");

    let held: Guards = Mutex::new(None);
    let Err(err) = open(root, actor, &held) else {
        panic!("must refuse rather than share the actor file");
    };
    assert!(
        err.to_string()
            .contains("already open in another outl process"),
        "the refusal must say who is in the way, got {err}"
    );
    assert!(
        held.lock().is_none(),
        "a refused open must not install guards"
    );
}

/// A refused open leaves nothing behind. The shared workspace lock is
/// taken before the per-actor one, so a refusal at the second step must
/// not strand the first — otherwise a single failed open would block
/// compaction on that workspace until the process exits.
#[test]
fn a_refused_open_parks_no_lock() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    let actor = ActorId::new();

    let _other = ActorWriteLock::try_acquire(&root.join("ops"), actor).expect("first holder");
    let held: Guards = Mutex::new(None);
    assert!(open(root, actor, &held).is_err(), "must refuse");

    // Nobody holds `.outl/.lock`, so an exclusive acquire (what
    // compaction does) succeeds.
    seed_compactable_log(root);
    let plan = plan_now(root);
    assert!(!plan.is_empty());
    assert!(
        !matches!(apply_compaction(root, &plan), Err(CompactError::Busy(_))),
        "a refused open stranded the shared workspace lock"
    );
}

/// The workspace lock is **shared**. A running GUI must not lock the TUI,
/// the MCP server or a `outl` subcommand out of the same workspace — they
/// each write their own `ops-<actor>.jsonl`, which is what makes
/// concurrent opens safe in the first place.
#[test]
fn a_running_gui_does_not_lock_out_another_outl_process() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();

    let held: Guards = Mutex::new(None);
    let _ws = open(root, ActorId::new(), &held).expect("gui opens the workspace");

    let _peer = WorkspaceLock::acquire(root).expect("a second outl process must still get in");
    // And on its own actor, which is how the CLI and TUI resolve theirs.
    let _peer_write = ActorWriteLock::try_acquire(&root.join("ops"), ActorId::new())
        .expect("a different actor is never contended");
}

// --------------------------------------------------------------------------
// Lifetime: what releases the guards
// --------------------------------------------------------------------------

/// A POSIX `flock` belongs to an open file description, so a second
/// `open` + `LOCK_EX|LOCK_NB` of `ops/.lock-<actor>` fails *inside the
/// process that already holds it*. Re-picking the folder already open
/// (the desktop's `set_workspace` pointed at the current root) must
/// therefore release before it retakes, not refuse itself.
#[test]
fn re_picking_the_open_workspace_does_not_refuse_itself() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    let actor = ActorId::new();

    let held: Guards = Mutex::new(None);
    let first = open(root, actor, &held).expect("first open");
    drop(first);

    let _second = open(root, actor, &held).expect("re-picking the open workspace must succeed");
    assert!(
        held.lock().is_some(),
        "guards reinstalled after the re-pick"
    );
}

/// Switching to another workspace releases the previous root's guards —
/// the slot is replaced, and replacing it is what drops them. A guard
/// that outlives its workspace would block compaction on a folder nobody
/// has open.
#[test]
fn switching_workspaces_releases_the_previous_root() {
    let old_dir = TempDir::new().expect("tempdir");
    let new_dir = TempDir::new().expect("tempdir");
    let (old, new) = (old_dir.path(), new_dir.path());
    seed_compactable_log(old);

    let actor = ActorId::new();
    let held: Guards = Mutex::new(None);
    let first = open(old, actor, &held).expect("open the first workspace");
    assert!(matches!(compact_now(old), Err(CompactError::Busy(_))));

    // The client swaps its `Option<Workspace>` too; `open_workspace_at`
    // swaps the guard slot on its own.
    let _second = open(new, actor, &held).expect("open the second workspace");
    drop(first);
    assert_eq!(
        held.lock().as_ref().map(|g| g.root().to_path_buf()),
        Some(std::fs::canonicalize(new).expect("canonical")),
        "the slot must now guard the new root"
    );

    let report = compact_now(old).expect("the abandoned workspace is compactable again");
    assert!(report.ops_dropped > 0);
}
