//! Unit coverage for the parts of compaction that the behavioural
//! battery in `tests/compaction.rs` reaches only indirectly.

use super::*;
use crate::fractional::Fractional;
use crate::hlc::Hlc;
use crate::id::NodeId;
use crate::op::{LogOp, Op};
use std::collections::BTreeMap;
use std::path::Path;
use tempfile::TempDir;

fn ops_dir() -> TempDir {
    let tmp = TempDir::new().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("ops")).expect("mkdir");
    tmp
}

// ------------------------------------------------------------- fixtures

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

fn now_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

fn pos(s: &str) -> Fractional {
    Fractional::parse(s).expect("valid fractional")
}

fn create(ms: u64, actor: ActorId, node: NodeId, parent: NodeId, position: &str) -> LogOp {
    LogOp {
        ts: Hlc::new(ms, 0, actor),
        actor,
        op: Op::Create {
            node,
            parent,
            position: pos(position),
        },
    }
}

fn mv(ms: u64, actor: ActorId, node: NodeId, new_parent: NodeId, position: &str) -> LogOp {
    LogOp {
        ts: Hlc::new(ms, 0, actor),
        actor,
        op: Op::Move {
            node,
            new_parent,
            position: pos(position),
            old_parent: NodeId::root(),
            old_position: Fractional::first(),
        },
    }
}

/// A log holding exactly one droppable pair, in a caller-chosen window.
///
/// Two actors need **disjoint** windows: the pair must be adjacent in the
/// *merged* HLC order, and overlapping windows dissolve it.
fn compactable_log_at(actor: ActorId, base_ms: u64) -> Vec<LogOp> {
    let parent = NodeId::new();
    let child = NodeId::new();
    vec![
        create(base_ms, actor, parent, NodeId::root(), "a"),
        create(base_ms + 1, actor, child, parent, "a"),
        // Restates the placement its own `Create` just made.
        mv(base_ms + 2, actor, child, parent, "a"),
    ]
}

fn workspace(files: &[(ActorId, Vec<LogOp>)]) -> TempDir {
    let tmp = TempDir::new().expect("tempdir");
    let ops = tmp.path().join("ops");
    std::fs::create_dir_all(&ops).expect("mkdir ops");
    std::fs::create_dir_all(tmp.path().join(".outl")).expect("mkdir .outl");
    for (actor, log) in files {
        let body: String = log
            .iter()
            .map(|op| serde_json::to_string(op).expect("serialize") + "\n")
            .collect();
        std::fs::write(ops.join(format!("ops-{actor}.jsonl")), body).expect("write ops file");
    }
    tmp
}

/// Every op log under `ops/`, by name and bytes. Only the `.jsonl` files:
/// taking a lock leaves a `.lock-<actor>` behind that is not damage.
fn ops_snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(root.join("ops")) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        out.insert(name, std::fs::read(&path).unwrap_or_default());
    }
    out
}

fn no_horizon() -> CompactOptions {
    CompactOptions { horizon_ms: 0 }
}

#[test]
fn a_damaged_record_refuses_the_whole_pass() {
    // Invariant 5: a damaged log costs you the damaged bytes, never the
    // healthy bytes after them — and never quietly. Rewriting a file we
    // could not fully read would make the loss permanent.
    let tmp = ops_dir();
    let actor = ActorId::new();
    std::fs::write(
        tmp.path().join("ops").join(format!("ops-{actor}.jsonl")),
        "{\"ts\":  not json\n",
    )
    .expect("write");

    let err = plan_compaction(tmp.path(), &CompactOptions::default()).expect_err("must refuse");
    assert!(
        matches!(err, CompactError::DamagedLog { line: 1, .. }),
        "got {err:?}"
    );
}

#[test]
fn a_per_page_layout_refuses_rather_than_compacting_half_a_log() {
    let tmp = ops_dir();
    let actor = ActorId::new();
    let shard = tmp.path().join("ops").join(actor.to_string());
    std::fs::create_dir_all(&shard).expect("mkdir");
    std::fs::write(shard.join("ideas.jsonl"), "").expect("write");

    let err = plan_compaction(tmp.path(), &CompactOptions::default()).expect_err("must refuse");
    assert!(matches!(err, CompactError::PerPageLayout(_)), "got {err:?}");
}

#[test]
fn an_empty_ops_dir_plans_nothing() {
    let tmp = ops_dir();
    let plan = plan_compaction(tmp.path(), &CompactOptions::default()).expect("plan");
    assert!(plan.is_empty());
    assert_eq!(plan.report().ops_total, 0);
    assert_eq!(plan.report().percent(), 0.0);
}

#[test]
fn the_default_horizon_is_thirty_days() {
    assert_eq!(
        CompactOptions::default().horizon_ms,
        30 * 24 * 60 * 60 * 1000
    );
    assert_eq!(DEFAULT_HORIZON_MS, CompactOptions::default().horizon_ms);
}

#[test]
fn page_roots_are_the_derivable_family_this_excludes() {
    // Pins the reason condition 4 is spelled `parent == NodeId::root()`:
    // a page root's id is a pure function of its slug, so a device that
    // has never seen one of our ops can name it.
    assert_eq!(NodeId::from_slug("ideas"), NodeId::from_slug("ideas"));
    assert_ne!(NodeId::from_slug("ideas"), NodeId::from_slug("plans"));
}

// ----------------------------------------------- the settling horizon

/// The horizon is a **wall-clock** margin, so it cannot be anchored on a
/// number a peer chose.
///
/// `hlc.rs` is hand-rolled and has no drift clamp — `observe` adopts any
/// peer's higher `physical_ms` unconditionally. One device with a wrong
/// clock stamps an op years ahead; a cutoff derived from "the newest op
/// in the log" then sits years in the past, every real op falls below it,
/// and condition 6 stops rejecting anything. The user who deliberately
/// did *not* pass `--no-horizon` would silently get `--no-horizon`.
#[test]
fn a_future_stamped_op_cannot_disarm_the_settling_horizon() {
    let a = ActorId::new();
    let mut log = compactable_log_at(a, now_ms() - DAY_MS);
    // A peer whose clock says 2036. Nothing in the CRDT rejects it.
    log.push(create(
        now_ms() + 3650 * DAY_MS,
        a,
        NodeId::new(),
        NodeId::root(),
        "b",
    ));
    let tmp = workspace(&[(a, log)]);

    let plan = plan_compaction(tmp.path(), &CompactOptions::default()).expect("plan");
    assert!(
        plan.is_empty(),
        "a pair written yesterday is inside any 30-day horizon; a future-stamped op \
         must not move the cutoff past it, got {:?}",
        plan.report()
    );
}

/// The mirror: clamping to the wall clock is *only* a clamp. A log whose
/// newest op is in the past keeps the exact cutoff it had before.
#[test]
fn the_horizon_is_unchanged_when_every_op_is_in_the_past() {
    let a = ActorId::new();
    let mut log = compactable_log_at(a, now_ms() - 200 * DAY_MS);
    log.push(create(
        now_ms() - DAY_MS,
        a,
        NodeId::new(),
        NodeId::root(),
        "b",
    ));
    let tmp = workspace(&[(a, log)]);

    let plan = plan_compaction(tmp.path(), &CompactOptions::default()).expect("plan");
    assert_eq!(
        plan.report().ops_dropped,
        1,
        "a pair from 200 days ago is well outside the horizon and must still be dropped"
    );
}

// ------------------------------------------- whose file may be rewritten

/// `docs/storage.md`: *"Each device's file is append-only and owned by
/// exactly one writer."* That premise is what makes `transport = "file"`
/// safe — iCloud / Syncthing / a shared FS reconcile **per path**,
/// last-write-wins, and the `flock`s here are machine-local.
///
/// So a device that shortens another device's `ops-<actor>.jsonl`
/// publishes a competing, shorter version of that path. The peer's copy
/// loses, and every op it had not yet shipped dies with it.
#[test]
fn a_rewrite_refuses_to_touch_another_devices_ops_file() {
    let a = ActorId::new();
    let b = ActorId::new();
    let tmp = workspace(&[
        (a, compactable_log_at(a, 10)),
        (b, compactable_log_at(b, 1000)),
    ]);
    let root = tmp.path();
    let plan = plan_compaction(root, &no_horizon()).expect("plan");
    assert_eq!(plan.report().ops_dropped, 2, "both files have a pair");

    let before = ops_snapshot(root);
    let err = apply_compaction_as(root, &plan, a)
        .expect_err("a plan naming another device's file must be refused");
    assert!(
        matches!(err, CompactError::ForeignActorFile { .. }),
        "got {err:?}"
    );
    assert_eq!(
        ops_snapshot(root),
        before,
        "and no file may be rewritten — not even our own"
    );
}

/// The escape from that refusal is not `--force`, it is doing less: a
/// plan narrowed to this device rewrites this device's file and leaves
/// every peer's mirror byte-identical.
#[test]
fn a_plan_narrowed_to_this_device_rewrites_only_its_own_file() {
    let a = ActorId::new();
    let b = ActorId::new();
    let tmp = workspace(&[
        (a, compactable_log_at(a, 10)),
        (b, compactable_log_at(b, 1000)),
    ]);
    let root = tmp.path();
    let plan = plan_compaction(root, &no_horizon()).expect("plan");
    let before = ops_snapshot(root);

    let narrowed = plan.restricted_to(a);
    assert_eq!(narrowed.report().ops_dropped, 1, "only a's pair survives");
    assert_eq!(
        narrowed.report().ops_total,
        plan.report().ops_total,
        "the totals still describe the whole log, only the drops narrow"
    );

    let done = apply_compaction_as(root, &narrowed, a).expect("apply");
    assert_eq!(done.ops_dropped, 1);

    let after = ops_snapshot(root);
    assert_eq!(
        after.get(&format!("ops-{b}.jsonl")),
        before.get(&format!("ops-{b}.jsonl")),
        "the peer's file is not ours to shorten"
    );
    assert_ne!(
        after.get(&format!("ops-{a}.jsonl")),
        before.get(&format!("ops-{a}.jsonl")),
        "ours is"
    );
}

/// Narrowing to an actor with nothing droppable yields a plan that does
/// nothing, rather than one that quietly falls back to everything.
#[test]
fn narrowing_to_an_actor_with_no_drops_yields_an_empty_plan() {
    let a = ActorId::new();
    let b = ActorId::new();
    let tmp = workspace(&[
        (a, compactable_log_at(a, 10)),
        (b, vec![create(5000, b, NodeId::new(), NodeId::root(), "a")]),
    ]);
    let plan = plan_compaction(tmp.path(), &no_horizon()).expect("plan");
    assert!(!plan.is_empty());

    let narrowed = plan.restricted_to(b);
    assert!(narrowed.is_empty(), "b has nothing to drop");
    assert_eq!(narrowed.report().bytes_dropped, 0);
    // The report and the drop set have to agree: a narrowing that zeroed
    // the totals while keeping `a`'s HLCs would read as empty here and
    // still rewrite `a`'s file.
    assert!(
        narrowed.foreign_actors(b).is_empty(),
        "nothing outside b may survive the narrowing"
    );
}

// ------------------------------------------ the actor set under the lock

/// `lock_every_actor` locks the actors the **plan** saw. A file that
/// appears between plan and apply is therefore unlocked, unmeasured, and
/// — the part that matters — its ops never entered the merged log that
/// inertness was decided against.
#[test]
fn an_actor_file_that_appeared_since_the_plan_is_refused() {
    let a = ActorId::new();
    let tmp = workspace(&[(a, compactable_log_at(a, 10))]);
    let root = tmp.path();
    let plan = plan_compaction(root, &no_horizon()).expect("plan");
    assert!(!plan.is_empty());

    // A peer's log landing over sync, or a second process minting an
    // ephemeral actor.
    let c = ActorId::new();
    let body = serde_json::to_string(&mv(11, c, NodeId::new(), NodeId::new(), "a"))
        .expect("serialize")
        + "\n";
    std::fs::write(root.join("ops").join(format!("ops-{c}.jsonl")), body).expect("write");

    let before = ops_snapshot(root);
    let err = apply_compaction_as(root, &plan, a)
        .expect_err("a log the plan never read must invalidate it");
    assert!(matches!(err, CompactError::PlanStale(_)), "got {err:?}");
    assert_eq!(ops_snapshot(root), before);
}

/// The mirror: a file that *vanished*. Same verdict, and worth its own
/// test because the old code reached it as a bare `NotFound`, which reads
/// like a bug in compaction rather than a log that moved.
#[test]
fn an_actor_file_that_vanished_since_the_plan_is_refused_as_stale() {
    let a = ActorId::new();
    let b = ActorId::new();
    let tmp = workspace(&[
        (a, compactable_log_at(a, 10)),
        (b, compactable_log_at(b, 1000)),
    ]);
    let root = tmp.path();
    let plan = plan_compaction(root, &no_horizon()).expect("plan");

    std::fs::remove_file(root.join("ops").join(format!("ops-{b}.jsonl"))).expect("remove");

    let before = ops_snapshot(root);
    // The unguarded `--force` entry point, so the verdict under test is
    // the actor set and not the foreign-file refusal.
    let err = apply_compaction(root, &plan).expect_err("must refuse");
    assert!(matches!(err, CompactError::PlanStale(_)), "got {err:?}");
    assert_eq!(ops_snapshot(root), before);
}

// ---------------------------------------- a failure after the backup

/// The backup is the only thing standing between a half-rewritten `ops/`
/// and a restore, and the one moment its path is needed is the one
/// moment the old code did not print it: `report.backup_dir` was filled
/// in on success only.
#[cfg(unix)]
#[test]
fn a_rewrite_that_fails_still_names_the_backup_it_took() {
    use std::os::unix::fs::PermissionsExt;

    let a = ActorId::new();
    let tmp = workspace(&[(a, compactable_log_at(a, 10))]);
    let root = tmp.path();
    let plan = plan_compaction(root, &no_horizon()).expect("plan");

    // ENOSPC is the realistic shape (the log grew until the disk filled);
    // a read-only `ops/` reaches the same branch deterministically. The
    // backup lands under `.outl/`, so it still succeeds.
    //
    // The per-actor lock file has to exist first: `try_acquire` opens it
    // with `O_CREAT`, which needs a writable *directory*, and failing
    // there would test a branch that runs before the backup.
    let ops = root.join("ops");
    std::fs::write(ops.join(format!(".lock-{a}")), "").expect("pre-create the lock file");
    std::fs::set_permissions(&ops, std::fs::Permissions::from_mode(0o555)).expect("chmod");

    let result = apply_compaction_as(root, &plan, a);

    std::fs::set_permissions(&ops, std::fs::Permissions::from_mode(0o755)).expect("chmod back");

    let err = result.expect_err("a read-only ops dir cannot be rewritten");
    let CompactError::RewriteFailed { backup_dir, .. } = &err else {
        panic!("the failure must carry the backup it already took, got {err:?}");
    };
    assert!(
        backup_dir.join(format!("ops-{a}.jsonl")).is_file(),
        "and that directory must actually hold the pre-compaction log"
    );
    assert!(
        err.to_string().contains("compact-backup"),
        "the message the user sees must name it: {err}"
    );
}

// ------------------------------------------- planning against a live log

/// `plan_compaction` reads `ops/` with no lock and turns any unparseable
/// line into `DamagedLog`, whose message sends the user to `outl doctor`.
/// A concurrent append is visible mid-line, so a healthy log gets
/// diagnosed as a corrupt one. Nothing is written either way — the cost
/// is the false alarm, and a false corruption alarm is expensive.
#[test]
fn planning_refuses_while_the_workspace_is_open() {
    let a = ActorId::new();
    let tmp = workspace(&[(a, compactable_log_at(a, 10))]);
    let root = tmp.path();

    // Exactly what a live client holds.
    let _held = crate::lock::WorkspaceLock::acquire(root).expect("acquire");

    let err = plan_compaction(root, &no_horizon())
        .expect_err("a log that can move under the reader must not be read");
    assert!(matches!(err, CompactError::Busy(_)), "got {err:?}");
}

/// And it still plans when nobody is attached — the guard above must not
/// become a wall for the ordinary case.
#[test]
fn planning_succeeds_once_nothing_holds_the_workspace() {
    let a = ActorId::new();
    let tmp = workspace(&[(a, compactable_log_at(a, 10))]);
    let plan = plan_compaction(tmp.path(), &no_horizon()).expect("plan");
    assert_eq!(plan.report().ops_dropped, 1);
}

// ------------------------------------------------------ honest totals

/// Blank lines are dropped by the rewrite (a torn-tail heal leaves one),
/// so the bytes they cost must show up in what the report says the file
/// lost. No data risk; a report that under-declares what it removed is
/// still a report the outcome disagrees with.
#[test]
fn a_dropped_blank_line_is_counted_in_the_bytes_it_reclaims() {
    let a = ActorId::new();
    let tmp = workspace(&[(a, compactable_log_at(a, 10))]);
    let root = tmp.path();
    let path = root.join("ops").join(format!("ops-{a}.jsonl"));

    let body = std::fs::read_to_string(&path).expect("read");
    let mut lines: Vec<&str> = body.lines().collect();
    lines.insert(1, "");
    std::fs::write(&path, lines.join("\n") + "\n").expect("write");

    let before = std::fs::metadata(&path).expect("stat").len();
    let plan = plan_compaction(root, &no_horizon()).expect("plan");
    let declared = plan.report().bytes_dropped;

    let done = apply_compaction_as(root, &plan, a).expect("apply");
    let after = std::fs::metadata(&path).expect("stat").len();

    assert_eq!(
        before - after,
        declared,
        "the plan must declare every byte the rewrite removes, blank line included"
    );
    assert_eq!(done.bytes_dropped, declared);
}
