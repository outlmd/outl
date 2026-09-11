//! Compaction's refusal arms, each one watched firing.
//!
//! `apply_compaction` physically rewrites `ops-<actor>.jsonl` — the file
//! root `CLAUDE.md` invariant 1 calls the source of truth — which makes
//! it the most destructive code in this repository.
//! Its module doc promises that a damaged log is *reported*, never
//! rewritten, and that a log which moved since the plan was made is
//! refused.
//!
//! Those promises had never been executed.
//! `tests/compaction.rs` covers what compaction *drops*; this file covers
//! what it *declines to touch*, on the principle the rest of this
//! campaign keeps landing on: a guard is worth exactly what it refuses,
//! and an untested refusal is a guard you are hoping works.
//!
//! Every test here asserts two things, not one — the error **and** that
//! `ops/` came through byte-for-byte. A refusal that returns `Err` after
//! writing half a file is not a refusal.

use std::collections::BTreeMap;
use std::path::Path;

use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::storage::compact::{
    apply_compaction, plan_compaction, CompactError, CompactOptions,
};
use tempfile::TempDir;

// --------------------------------------------------------------- fixtures

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

/// A log holding exactly one droppable pair, so any test that reaches the
/// rewrite has something it *would* have rewritten. Without that, "the
/// file is unchanged" proves nothing.
fn compactable_log(actor: ActorId) -> Vec<LogOp> {
    compactable_log_at(actor, 10)
}

/// As [`compactable_log`], but placed in a caller-chosen time window.
///
/// Two actors need **disjoint** windows: the droppable pair must be
/// adjacent in the *merged* HLC order, and overlapping windows interleave
/// the two actors' ops and dissolve every pair.
fn compactable_log_at(actor: ActorId, base_ms: u64) -> Vec<LogOp> {
    let parent = NodeId::new();
    let child = NodeId::new();
    vec![
        create(base_ms, actor, parent, NodeId::root(), "a"),
        create(base_ms + 1, actor, child, parent, "a"),
        // Restates the placement its own `Create` just made: the one
        // shape compaction can prove inert.
        mv(base_ms + 2, actor, child, parent, "a"),
    ]
}

/// Every **op log** under `ops/`, by name and bytes.
///
/// Deliberately only the `.jsonl` files. Taking any of compaction's locks
/// creates a `.lock-<actor>` that outlives the lock (the file's existence
/// is not the lock — `Drop` releases the flock and leaves the path), so a
/// whole-directory diff would report the guard doing its job as damage.
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

// ------------------------------------------------- the damaged-log promise

/// The promise in `rewrite.rs`'s module doc: *"It refuses outright on a
/// log holding a record it could not parse. A damaged log is reported,
/// never rewritten."*
///
/// This is the refusal whose failure mode is unbounded. Rewriting a torn
/// log destroys the surviving records around the damage and takes a
/// backup of the already-torn state, so the backup cannot undo it.
#[test]
fn an_unparseable_record_is_refused_and_the_log_is_left_alone() {
    let a = ActorId::new();
    let tmp = workspace(&[(a, compactable_log(a))]);
    let root = tmp.path();
    let path = root.join("ops").join(format!("ops-{a}.jsonl"));

    // A torn tail: the shape a crash mid-append actually leaves.
    let mut body = std::fs::read_to_string(&path).expect("read");
    body.push_str("{\"ts\":{\"physical_ms\":40,\"logi\n");
    std::fs::write(&path, &body).expect("write torn log");

    let before = ops_snapshot(root);

    let err = plan_compaction(root, &no_horizon()).expect_err("a damaged log must not plan");
    assert!(
        matches!(err, CompactError::DamagedLog { line: 4, .. }),
        "the refusal must name the line that could not be parsed, got {err:?}"
    );
    assert_eq!(
        ops_snapshot(root),
        before,
        "planning writes nothing, damaged or not"
    );
}

/// Same promise through the *writing* entry point.
///
/// The damage is written **in place, same length** — a flipped byte, not
/// an appended line. That is deliberate: an append also moves the file's
/// length, which trips the staleness check first, and then this test
/// would be passing on the wrong guard. In-place corruption (bit rot, a
/// torn write that replaced bytes) leaves `DamagedLog` as the only
/// candidate.
#[test]
fn a_log_damaged_between_plan_and_apply_is_refused_before_any_write() {
    let a = ActorId::new();
    let b = ActorId::new();
    // Two actors: `b`'s file is perfectly compactable, so a refusal that
    // only covered the damaged file would still rewrite this one.
    let tmp = workspace(&[
        (a, compactable_log_at(a, 10)),
        (b, compactable_log_at(b, 100)),
    ]);
    let root = tmp.path();

    let plan = plan_compaction(root, &no_horizon()).expect("plan on a healthy log");
    assert!(!plan.is_empty(), "the fixture must have something to drop");

    let damaged = root.join("ops").join(format!("ops-{a}.jsonl"));
    let mut body = std::fs::read(&damaged).expect("read");
    // Flip the first byte of the last record: still the same number of
    // bytes, no longer parseable.
    let last_line_start = body
        .iter()
        .rposition(|b| *b == b'\n' && *b != body[body.len() - 1])
        .map(|i| i + 1)
        .unwrap_or(0);
    let start = body[..body.len() - 1]
        .iter()
        .rposition(|b| *b == b'\n')
        .map(|i| i + 1)
        .unwrap_or(last_line_start);
    body[start] = b'x';
    std::fs::write(&damaged, &body).expect("damage the log");

    let before = ops_snapshot(root);
    let err = apply_compaction(root, &plan).expect_err("a damaged log must not be rewritten");
    assert!(
        matches!(err, CompactError::DamagedLog { .. }),
        "got {err:?}"
    );
    assert_eq!(
        ops_snapshot(root),
        before,
        "no file may be rewritten once any file in `ops/` is known damaged — not even the \
         healthy one, because compaction decides inertness against the MERGED log"
    );
}

/// Non-UTF-8 bytes are the other damage shape, and they take a different
/// branch (`str::from_utf8`) than a JSON parse failure.
#[test]
fn non_utf8_bytes_in_a_log_are_refused() {
    let a = ActorId::new();
    let tmp = workspace(&[(a, compactable_log(a))]);
    let root = tmp.path();
    let path = root.join("ops").join(format!("ops-{a}.jsonl"));

    let mut body = std::fs::read(&path).expect("read");
    body.extend_from_slice(&[0xff, 0xfe, b'\n']);
    std::fs::write(&path, &body).expect("write");

    let before = ops_snapshot(root);
    let err = plan_compaction(root, &no_horizon()).expect_err("invalid UTF-8 must refuse");
    assert!(
        matches!(err, CompactError::DamagedLog { line: 4, .. }),
        "got {err:?}"
    );
    assert_eq!(ops_snapshot(root), before);
}

/// Damage that also changes the file's length hits the staleness check
/// first. Recorded rather than "fixed": both arms refuse, neither writes,
/// and `PlanStale` tells the user to re-plan — which then surfaces the
/// real cause as `DamagedLog`. The two-step is correct; what would be a
/// bug is either arm letting the rewrite through.
#[test]
fn damage_that_also_resizes_the_file_is_reported_as_a_stale_plan_first() {
    let a = ActorId::new();
    let tmp = workspace(&[(a, compactable_log(a))]);
    let root = tmp.path();
    let plan = plan_compaction(root, &no_horizon()).expect("plan");

    let path = root.join("ops").join(format!("ops-{a}.jsonl"));
    let mut body = std::fs::read_to_string(&path).expect("read");
    body.push_str("not json at all\n");
    std::fs::write(&path, &body).expect("damage");

    let before = ops_snapshot(root);
    let err = apply_compaction(root, &plan).expect_err("must refuse");
    assert!(matches!(err, CompactError::PlanStale(_)), "got {err:?}");
    assert_eq!(ops_snapshot(root), before);

    // Re-planning then names the real problem.
    let err = plan_compaction(root, &no_horizon()).expect_err("must refuse");
    assert!(
        matches!(err, CompactError::DamagedLog { .. }),
        "got {err:?}"
    );
}

// ------------------------------------------------------- the stale plan

/// A plan records each file's byte length. If the file moved since, the
/// plan's line offsets describe a file that no longer exists — applying
/// it would drop the wrong records.
#[test]
fn a_log_that_grew_between_plan_and_apply_is_refused() {
    let a = ActorId::new();
    let tmp = workspace(&[(a, compactable_log(a))]);
    let root = tmp.path();

    let plan = plan_compaction(root, &no_horizon()).expect("plan");
    assert!(!plan.is_empty());

    // A concurrent append: another client, or a peer's ops landing.
    let path = root.join("ops").join(format!("ops-{a}.jsonl"));
    let extra = serde_json::to_string(&create(40, a, NodeId::new(), NodeId::root(), "b"))
        .expect("serialize")
        + "\n";
    let mut body = std::fs::read_to_string(&path).expect("read");
    body.push_str(&extra);
    std::fs::write(&path, &body).expect("append");

    let before = ops_snapshot(root);
    let err = apply_compaction(root, &plan).expect_err("a moved file must invalidate the plan");
    assert!(matches!(err, CompactError::PlanStale(_)), "got {err:?}");
    assert_eq!(
        ops_snapshot(root),
        before,
        "the staleness check must fire before the first byte is written"
    );
}

/// The mirror case: a file that *shrank*. Same verdict, and worth its own
/// test because a length check written as `>` rather than `!=` would pass
/// the test above and silently accept this one.
#[test]
fn a_log_that_shrank_between_plan_and_apply_is_refused() {
    let a = ActorId::new();
    let tmp = workspace(&[(a, compactable_log(a))]);
    let root = tmp.path();

    let plan = plan_compaction(root, &no_horizon()).expect("plan");
    let path = root.join("ops").join(format!("ops-{a}.jsonl"));
    let body = std::fs::read_to_string(&path).expect("read");
    let truncated: String = body.lines().take(2).map(|l| format!("{l}\n")).collect();
    std::fs::write(&path, &truncated).expect("truncate");

    let before = ops_snapshot(root);
    let err = apply_compaction(root, &plan).expect_err("a shrunk file must invalidate the plan");
    assert!(matches!(err, CompactError::PlanStale(_)), "got {err:?}");
    assert_eq!(ops_snapshot(root), before);
}

// ------------------------------------------------------------ the locks

/// Compaction renumbers every byte offset, and a running client caches
/// those offsets. The exclusive `flock` on `.outl/.lock` is the one
/// question worth asking before a rewrite: *is anyone else in here?*
///
/// This matters more than when it was written: the GUI now takes the
/// workspace lock for the lifetime of its session, so this arm is what
/// stands between `outl compact --apply` and a running desktop app.
#[test]
fn a_held_workspace_lock_refuses_the_rewrite() {
    let a = ActorId::new();
    let tmp = workspace(&[(a, compactable_log(a))]);
    let root = tmp.path();
    let plan = plan_compaction(root, &no_horizon()).expect("plan");
    assert!(!plan.is_empty());

    // Exactly what a live client holds: the shared workspace lock.
    let _held = outl_core::WorkspaceLock::acquire(root).expect("acquire");

    let before = ops_snapshot(root);
    let err = apply_compaction(root, &plan).expect_err("must refuse while a client is attached");
    assert!(matches!(err, CompactError::Busy(_)), "got {err:?}");
    assert_eq!(ops_snapshot(root), before);
}

/// A writer holds exactly one `ActorWriteLock`, and it may be for an
/// actor whose file this plan does not touch — so compaction takes the
/// lock for **every** actor in `ops/`, not just the ones being rewritten.
#[test]
fn a_held_write_lock_on_an_untouched_actor_still_refuses() {
    let a = ActorId::new();
    let b = ActorId::new();
    // Only `a` has anything droppable; `b` is inert.
    let tmp = workspace(&[
        (a, compactable_log_at(a, 10)),
        (b, vec![create(500, b, NodeId::new(), NodeId::root(), "a")]),
    ]);
    let root = tmp.path();
    let plan = plan_compaction(root, &no_horizon()).expect("plan");
    assert!(!plan.is_empty());

    let _held = outl_core::lock::ActorWriteLock::try_acquire(&root.join("ops"), b)
        .expect("acquire b's write lock");

    let before = ops_snapshot(root);
    let err = apply_compaction(root, &plan)
        .expect_err("a writer on ANY actor must block the rewrite, touched or not");
    assert!(matches!(err, CompactError::Busy(_)), "got {err:?}");
    assert_eq!(ops_snapshot(root), before);
}

// ------------------------------------------------------- the empty cases

/// A directory with no `.jsonl` at all is not an error and not a rewrite.
/// `holds_op_log` is what keeps compaction from mistaking, say, a
/// per-page shard directory it has not looked inside for an empty
/// workspace.
#[test]
fn an_ops_directory_holding_no_op_log_plans_nothing() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("ops")).expect("mkdir");
    std::fs::create_dir_all(root.join(".outl")).expect("mkdir");
    std::fs::write(root.join("ops").join("README"), "not an op log").expect("write");

    let plan = plan_compaction(root, &no_horizon()).expect("an empty ops dir is not an error");
    assert!(plan.is_empty());
}

/// Blank lines inside a log are carried through rather than treated as
/// damage — a torn-tail heal leaves one, and refusing on it would lock a
/// user out of compaction forever.
#[test]
fn blank_lines_in_a_log_are_not_damage() {
    let a = ActorId::new();
    let tmp = workspace(&[(a, compactable_log(a))]);
    let root = tmp.path();
    let path = root.join("ops").join(format!("ops-{a}.jsonl"));

    let body = std::fs::read_to_string(&path).expect("read");
    let mut lines: Vec<&str> = body.lines().collect();
    lines.insert(1, "");
    std::fs::write(&path, lines.join("\n") + "\n").expect("write");

    let plan = plan_compaction(root, &no_horizon()).expect("a blank line is not damage");
    assert!(
        !plan.is_empty(),
        "and it must not stop the real pair from being found"
    );
    apply_compaction(root, &plan).expect("apply");
}

/// A subdirectory in `ops/` that holds no `.jsonl` is not the per-page
/// layout — it is a stray directory (an editor's scratch, a sync tool's
/// metadata) and must be stepped over, not treated as a reason to abort.
#[test]
fn a_stray_subdirectory_in_ops_does_not_abort_compaction() {
    let a = ActorId::new();
    let tmp = workspace(&[(a, compactable_log(a))]);
    let root = tmp.path();
    std::fs::create_dir_all(root.join("ops").join(".stfolder")).expect("mkdir");
    std::fs::write(root.join("ops").join(".stfolder").join("note"), "x").expect("write");

    let plan =
        plan_compaction(root, &no_horizon()).expect("a stray dir is not the per-page layout");
    assert!(!plan.is_empty());
}

/// A subdirectory we cannot read is the per-page question we **cannot
/// answer**, and the safe answer is the refusing one.
///
/// Compaction decides inertness against the merged log. If that directory
/// is a per-page shard, its ops are part of the merge and proceeding
/// without them can drop a `Move` some shard made meaningful. RFC 0211's
/// rule applies unchanged: an unreadable thing counts as present, because
/// the cost of guessing wrong in the other direction is the op log.
#[cfg(unix)]
#[test]
fn an_unreadable_subdirectory_in_ops_is_refused_rather_than_assumed_empty() {
    use std::os::unix::fs::PermissionsExt;

    let a = ActorId::new();
    let tmp = workspace(&[(a, compactable_log(a))]);
    let root = tmp.path();
    let opaque = root.join("ops").join("maybe-a-shard");
    std::fs::create_dir_all(&opaque).expect("mkdir");
    std::fs::set_permissions(&opaque, std::fs::Permissions::from_mode(0o000)).expect("chmod");

    let result = plan_compaction(root, &no_horizon());

    // Restore before asserting, so a failure does not leave an
    // undeletable TempDir behind.
    std::fs::set_permissions(&opaque, std::fs::Permissions::from_mode(0o755)).expect("chmod back");

    let err = result.expect_err("a directory we cannot read may be a per-page shard");
    assert!(matches!(err, CompactError::PerPageLayout(_)), "got {err:?}");
}
