//! Guards against the third recurrence of one bug.
//!
//! Rebuilding a single block's text means replaying *that block's* `Edit`
//! ops. `OpLog::edit_updates` answers that from the `edits_by_node`
//! index in `O(edits-of-node)`. The tempting alternative —
//! `log.iter().filter_map(..)` — returns **exactly the same bytes** in
//! `O(total ops)`.
//!
//! That identical-output property is the whole problem: no correctness
//! test can tell the two apart, so the scan has been reintroduced twice.
//!
//! 1. `block_text` scanned the log per block. Fixed in #179.
//! 2. `ContentStore::ensure_doc` had the identical defect, one function
//!    away, and was not fixed with it — so the first keystroke on any
//!    cold block after a full-replay boot scanned all 217,811 ops on the
//!    maintainer's workspace, on the foreground thread.
//! 3. `materialize_text_from_log` carried a third copy, latent: it was
//!    reachable only when an earlier pass failed to drain `pending`.
//!
//! Both now route through `content::doc_from_log`, the single owner.
//!
//! # Why this test is shaped like this
//!
//! Because the outputs are identical, the *only* observable difference
//! is cost, so this measures cost — which normally means a flaky test.
//! What keeps it honest is that it asserts a **ratio across log sizes**
//! rather than an absolute duration, so it does not care how fast the
//! machine is. A test that merely asserted "this is fast" would fail on
//! a loaded CI box and teach everyone to ignore it.
//!
//! # The numbers, measured rather than assumed
//!
//! This file used to claim the indexed ratio was ~1 against ~`LOG_GROWTH`
//! for a scan, and called that margin "enormous". Neither half held, and
//! the bound they justified went red on CI at 5.5x with the index in
//! place and nothing wrong.
//!
//! Measured on an idle macOS laptop, by injecting the scan into
//! `Workspace::block_text` — the path this test actually times:
//!
//! | `LOG_GROWTH` | indexed   | scanned     |
//! |--------------|-----------|-------------|
//! | 20 (old)     | 2.2–4.2x  | 6.5–6.7x    |
//! | 50 (now)     | 2.3–4.0x  | 13.1–25.6x  |
//!
//! The indexed ratio is not ~1 because only the *replayed work* is
//! constant. The read also pays a lookup in `text`, in `pending` and in
//! `edits_by_node`, and all three grow with the log — at the ~16µs the
//! fast case takes, those cache misses dominate three `Edit` replays.
//! That floor is real, so the ratio can only be pushed apart from above:
//! growing `LOG_GROWTH` costs a scan proportionally and the index almost
//! nothing, which is what the table shows.
//!
//! At `LOG_GROWTH` 20 the two bands nearly touch (4.2 vs 6.5) and a bound
//! between them is a coin flip on a loaded runner — that is the 5.5x
//! failure. At 50 they are a factor of three apart, and `MAX_RATIO` sits
//! at 8: twice the worst honest run, comfortably under the cheapest
//! scan.
//!
//! If this goes flaky again, re-measure both columns the same way and
//! move the bound — do not delete it, or the bug comes back a fourth
//! time.

use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::storage::MemoryStorage;
use outl_core::workspace::Workspace;
use std::time::{Duration, Instant};

/// Ops in the small workspace.
const SMALL_LOG: usize = 1_000;
/// How much bigger the large workspace is. A scan costs this much more;
/// the index costs the same. 50 rather than 20 so the two bands stay a
/// factor of three apart — see the table in the module docs.
const LOG_GROWTH: usize = 50;
/// Edits on the node under test. Identical in both workspaces, so the
/// indexed work is identical and only the *surrounding* log grows.
const EDITS_ON_TARGET: usize = 3;
/// Halfway between the measured bands, on a log scale: indexed runs
/// reach 4.0x, the cheapest scan costs 13.1x. See the module docs.
const MAX_RATIO: f64 = 8.0;
/// Best-of-N, to shrug off a scheduler hiccup.
const REPS: usize = 5;

fn target() -> NodeId {
    NodeId::from_seed(b"scan-guard:", "target")
}

fn filler(i: usize) -> NodeId {
    NodeId::from_seed(b"scan-guard:", &format!("filler-{i}"))
}

/// The op list for a workspace whose log holds `total_ops` ops, of which
/// exactly `EDITS_ON_TARGET` touch [`target`]. Every other op is an
/// `Edit` on a different node — the noise a scan has to wade through.
///
/// Returned as ops rather than as a built `Workspace` so each timed
/// iteration can seed a *fresh* storage from the same bytes. The target's
/// `Op::Edit` payloads have to come from a real `build_text_replace_update`
/// (they are Yrs deltas, not arbitrary bytes), which is why this builds a
/// throwaway workspace to produce them.
fn ops_for(total_ops: usize) -> (ActorId, Vec<LogOp>) {
    let actor = ActorId::new();
    let mut ops: Vec<LogOp> = Vec::with_capacity(total_ops);
    let mut ms = 1u64;

    let mut push = |op: Op, ms: &mut u64| {
        *ms += 1;
        ops.push(LogOp {
            ts: Hlc::new(*ms, 0, actor),
            actor,
            op,
        });
    };

    push(
        Op::Create {
            node: target(),
            parent: NodeId::root(),
            position: Fractional::first(),
        },
        &mut ms,
    );

    let filler_count = total_ops.saturating_sub(EDITS_ON_TARGET + 1);
    for i in 0..filler_count {
        // Every filler node needs to exist before it can be edited, but
        // creating one node per op would make the tree, not the log, the
        // thing that grows. A handful of nodes carrying many edits each
        // is what a real workspace looks like anyway.
        let n = filler(i % 64);
        if i < 64 {
            push(
                Op::Create {
                    node: n,
                    parent: NodeId::root(),
                    position: Fractional::first(),
                },
                &mut ms,
            );
        } else {
            push(
                Op::Edit {
                    node: n,
                    text_op: vec![(i % 251) as u8; 24],
                },
                &mut ms,
            );
        }
    }

    // The target's own edits land last so a scan cannot short-circuit.
    // They must be produced by the real encoder — `Op::Edit` carries a
    // Yrs `update_v1` delta, so arbitrary bytes would not replay into
    // readable text — hence this throwaway workspace.
    let mut ws = Workspace::open_with_storage(actor, Box::new(MemoryStorage::default()), None)
        .expect("open workspace");
    for op in &ops {
        ws.apply(op.clone()).expect("apply filler");
    }
    for _ in 0..EDITS_ON_TARGET {
        let update = ws.build_text_replace_update(target(), "hello");
        if update.is_empty() {
            continue;
        }
        ms += 1;
        let edit = LogOp {
            ts: Hlc::new(ms, 0, actor),
            actor,
            op: Op::Edit {
                node: target(),
                text_op: update,
            },
        };
        ws.apply(edit.clone()).expect("apply target edit");
        ops.push(edit);
    }
    (actor, ops)
}

/// Time the cheapest of `REPS` rebuilds of the target's text.
///
/// Each iteration seeds a fresh storage from the same ops and opens a new
/// `Workspace`, so the string for `target()` is not yet materialized and
/// the first `block_text` goes through the replay path under test.
///
/// Everything here is the crate's **public** API — `Storage::append_ops`,
/// `open_with_storage`, `block_text`. A private accessor would have been
/// marginally shorter, and widening `outl-core`'s surface for a test's
/// convenience is a debt that outlives the test.
fn time_cold_rebuild(total_ops: usize) -> Duration {
    let (actor, all) = ops_for(total_ops);

    let mut best = Duration::MAX;
    for _ in 0..REPS {
        let mut storage = MemoryStorage::default();
        <MemoryStorage as outl_core::Storage>::append_ops(&mut storage, &all).expect("seed");
        let ws = Workspace::open_with_storage(actor, Box::new(storage), None).expect("reopen");

        let start = Instant::now();
        let text = ws.block_text(target());
        let elapsed = start.elapsed();

        assert_eq!(
            text.as_deref(),
            Some("hello"),
            "the rebuild must still produce the right text — this test is about cost, \
             but a fast wrong answer is not the goal"
        );
        best = best.min(elapsed);
    }
    best
}

#[test]
fn rebuilding_one_blocks_text_does_not_scale_with_the_rest_of_the_log() {
    let small = time_cold_rebuild(SMALL_LOG);
    let large = time_cold_rebuild(SMALL_LOG * LOG_GROWTH);

    // Guard against a degenerate measurement: if the small case is at
    // clock resolution the ratio is meaningless, so floor it.
    let floor = Duration::from_nanos(200);
    let small = small.max(floor);
    let ratio = large.as_secs_f64() / small.as_secs_f64();

    assert!(
        ratio < MAX_RATIO,
        "rebuilding one block's text scaled {ratio:.1}x when the surrounding log grew {LOG_GROWTH}x \
         ({small:?} -> {large:?}). That is the signature of a whole-log scan. \
         `content::doc_from_log` must go through `OpLog::edit_updates`, which is indexed by node; \
         `log.iter().filter_map(..)` returns identical bytes in O(total ops) and is how this \
         regressed twice before. See this file's module docs."
    );
}
