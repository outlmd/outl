//! Opening a workspace raises this process's clock above the op log.
//!
//! `HlcGenerator::new` starts at `physical_ms: 0` and tracks the wall
//! clock from there, so nothing connects a freshly built generator to the
//! ops already on disk. After the wall clock moves *backwards* between two
//! runs — an NTP correction, a VM resuming stale, a restored backup, a
//! dual-boot — `next()` issues timestamps that sort **below** what the log
//! already holds.
//!
//! That is a **cost**, not a divergence: `apply_op` reorders and every
//! replica still converges. The cost is that each such op forces the
//! paper's undo/redo window over every newer log entry, synchronously, on
//! a foreground edit.
//!
//! A log stamped into the far future is the reproducible stand-in for the
//! backwards clock: the two are indistinguishable to the generator, and
//! only one of them is testable without touching the system clock.
//!
//! `outl_ws::open` is the chokepoint — every CLI command, the MCP server
//! and every embedder boots through it — so these pin the property there.

use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::NodeId;
use outl_core::op::{LogOp, Op};
use outl_ws::layout::{init, Paths};
use tempfile::TempDir;

/// Milliseconds since the epoch, the same reading `HlcGenerator` takes.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_millis() as u64
}

/// A workspace directory that has been through `outl init`.
fn workspace() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    init(&Paths::at(dir.path().to_path_buf())).expect("init");
    dir
}

/// Put one op stamped `ms` into the log, under the actor this process
/// resolved to (a foreign actor would be refused by `JsonlStorage`).
fn log_op_at(ms: u64, root: &std::path::Path, seed: &str) {
    let mut ctx = outl_ws::open(root).expect("open");
    let ts = Hlc::new(ms, 0, ctx.actor);
    ctx.workspace
        .apply(LogOp {
            ts,
            actor: ctx.actor,
            op: Op::Create {
                node: NodeId::from_seed(b"clock-seed-test:", seed),
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        })
        .expect("apply");
}

/// The headline case. A log written ahead of this machine's wall clock
/// must not be undercut by the next process to open the workspace.
#[test]
fn opening_issues_timestamps_above_a_log_written_ahead_of_the_wall_clock() {
    let dir = workspace();
    let ahead = now_ms() + outl_core::hlc::MAX_CLOCK_SKEW_MS / 2;
    log_op_at(ahead, dir.path(), "ahead");

    let ctx = outl_ws::open(dir.path()).expect("reopen");
    let issued = ctx.hlc.next();
    assert!(
        issued.physical_ms >= ahead,
        "open issued {issued:?}, which sorts below the {ahead} already in the log — every op \
         stamped like this forces an undo/redo window over the whole newer log"
    );
}

/// The maximum is taken across **every** actor, not just the one this
/// process writes under. A peer's ops arrive in their own
/// `ops-<actor>.jsonl`, and they are exactly the ops a late local stamp
/// would have to be re-ordered against.
#[test]
fn the_seed_covers_ops_written_by_another_actor() {
    let dir = workspace();
    let ahead = now_ms() + outl_core::hlc::MAX_CLOCK_SKEW_MS / 2;

    // Written straight to disk under a foreign actor, the way sync ingest
    // delivers a peer's ops.
    let peer = outl_core::id::ActorId::new();
    let op = LogOp {
        ts: Hlc::new(ahead, 0, peer),
        actor: peer,
        op: Op::Create {
            node: NodeId::from_seed(b"clock-seed-test:", "peer"),
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    };
    let ops_dir = dir.path().join("ops");
    std::fs::create_dir_all(&ops_dir).expect("mkdir ops");
    let line = serde_json::to_string(&op).expect("serialize");
    std::fs::write(
        ops_dir.join(format!("ops-{peer}.jsonl")),
        format!("{line}\n"),
    )
    .expect("write peer log");

    let ctx = outl_ws::open(dir.path()).expect("open");
    let issued = ctx.hlc.next();
    assert!(
        issued.physical_ms >= ahead,
        "open issued {issued:?}, below the peer's {ahead} — seeding must span every actor"
    );
}

/// Reopening never rewinds: the clock a second process starts with must
/// sit above every op the first one wrote.
///
/// Stated over a log that is **ahead of the wall clock**, which is what
/// makes it a real assertion instead of a restatement of "time passes".
/// Compared against the log's own tail, never against the previous
/// generator's last reading: two opens inside one millisecond get
/// separate generators and legitimately read the same timestamp, so that
/// comparison is a race, not a property. An earlier version of this test
/// made exactly that mistake and went order-dependent under load.
///
/// Without the seeding the first open's writes land at the wall clock,
/// below the far-future op, so the log's tail stays that far-future op —
/// and the reopen, also at the wall clock, sorts below it.
#[test]
fn reopening_a_workspace_never_rewinds_the_clock() {
    let dir = workspace();
    let ahead = now_ms() + outl_core::hlc::MAX_CLOCK_SKEW_MS / 2;
    log_op_at(ahead, dir.path(), "ahead");

    // The tail of the log after a full open → write cycle.
    let written = {
        let mut ctx = outl_ws::open(dir.path()).expect("open");
        let page = outl_actions::open_today(&mut ctx.workspace, &ctx.hlc).expect("open today");
        let hlc = ctx.hlc.clone();
        outl_actions::append_block(&mut ctx.workspace, &hlc, Some(page), Some("first"))
            .expect("append block");
        ctx.workspace
            .log()
            .last()
            .expect("the first open wrote something")
            .ts
    };
    assert!(
        written.physical_ms >= ahead,
        "the first open stamped {written:?} below the {ahead} already logged, so the reopen \
         assertion below would pin nothing"
    );

    let reopened = outl_ws::open(dir.path()).expect("reopen").hlc.next();
    assert!(
        reopened > written,
        "reopen issued {reopened:?}, at or below the {written:?} already in the log — the next \
         op it stamps would be re-ordered against the whole newer log"
    );
}

/// `OpLog::append`'s ordering `debug_assert` must not fire on a plain
/// open → edit → reopen cycle, and the resident log must come back
/// strictly ascending.
///
/// This is a guard, not a demonstration: an unseeded late stamp takes
/// `apply_op`'s *reorder* branch (undo down, insert, redo) rather than
/// appending out of order, so this test passes with the seeding removed
/// too. What it pins is that the seeding did not introduce the opposite
/// problem — a stamp that arrives out of order at the log's tail.
#[test]
fn an_open_edit_reopen_cycle_keeps_the_log_in_order() {
    let dir = workspace();
    // Start from a log stamped ahead of the wall clock, which is what
    // makes the next process's stamps late without the seed.
    log_op_at(
        now_ms() + outl_core::hlc::MAX_CLOCK_SKEW_MS,
        dir.path(),
        "ahead",
    );

    for round in 0..3 {
        let mut ctx = outl_ws::open(dir.path()).expect("open");
        let page = outl_actions::open_today(&mut ctx.workspace, &ctx.hlc).expect("open today");
        let hlc = ctx.hlc.clone();
        outl_actions::append_block(
            &mut ctx.workspace,
            &hlc,
            Some(page),
            Some(&format!("round {round}")),
        )
        .expect("append block");

        let mut previous: Option<outl_core::hlc::Hlc> = None;
        for op in ctx.workspace.log().iter() {
            if let Some(prev) = previous {
                assert!(
                    prev < op.ts,
                    "round {round} left the log out of order: {prev:?} then {:?}",
                    op.ts
                );
            }
            previous = Some(op.ts);
        }
    }
}
