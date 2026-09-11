//! Benchmarks the in-memory `OpLog` — the structure `apply_op`'s
//! reorder loop pushes and pops, and the one every boot fills.
//!
//! Separated from `tree.rs` because these are the log's *own* costs,
//! independent of any tree mutation. If a reorder turns out superlinear
//! in `tree.rs`, this is where the cause would show up.
//!
//! Three things are measured:
//!
//! - **`append`** — the tail push, plus maintenance of the
//!   `edits_by_node` side index.
//! - **`contains_ts` / `get_by_ts`** — a binary search over the
//!   HLC-sorted `ops`, called once per `apply_op` for idempotency, and
//!   again by `Workspace::apply` to read back what `do_op` derived.
//! - **`edit_updates`** — the per-node `Edit` index behind
//!   `Workspace::block_text`. Documented as O(edits-of-node); the
//!   question is whether it stays that way as the log grows around it.
//!
//! Run:
//! ```text
//! cargo bench -p outl-core --bench oplog
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use outl_core::hlc::Hlc;
use outl_core::id::NodeId;
use outl_core::log::OpLog;
use outl_core::op::Op;
use std::hint::black_box;

#[path = "common.rs"]
mod common;

use common::{actors, log_op, node, TIERS};

/// Op-log sizes. The top tier is the reference workspace's 217,811
/// ops, so a number here is directly comparable to production.
const LOG_SIZES: &[(&str, usize)] = &[("10k", 10_000), ("50k", 50_000), ("218k", 217_811)];

/// Build a log of `n` `Create` ops in strictly increasing HLC order.
fn filled_log(n: usize) -> OpLog {
    let a = actors(1)[0];
    let mut log = OpLog::new();
    for i in 0..n {
        log.append(common::create_op(i as u64 + 1, a, i, NodeId::root()));
    }
    log
}

fn bench_append(c: &mut Criterion) {
    let mut group = c.benchmark_group("oplog/append");
    for &(tier, n) in LOG_SIZES {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_function(BenchmarkId::from_parameter(tier), |b| {
            b.iter(|| black_box(filled_log(n).len()));
        });
    }
    group.finish();
}

/// The idempotency check every `apply_op` pays. Two lookups are
/// measured because they exercise different branches of the same
/// binary search: a hit near the middle, and a miss.
fn bench_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("oplog/lookup");
    let a = actors(1)[0];

    for &(tier, n) in LOG_SIZES {
        let log = filled_log(n);
        let hit = Hlc::new((n / 2) as u64 + 1, 0, a);
        let miss = Hlc::new(n as u64 + 10_000, 0, a);

        group.bench_with_input(BenchmarkId::new("contains_ts_hit", tier), &log, |b, l| {
            b.iter(|| black_box(l.contains_ts(black_box(&hit))));
        });
        group.bench_with_input(BenchmarkId::new("contains_ts_miss", tier), &log, |b, l| {
            b.iter(|| black_box(l.contains_ts(black_box(&miss))));
        });
        group.bench_with_input(BenchmarkId::new("get_by_ts_hit", tier), &log, |b, l| {
            b.iter(|| black_box(l.get_by_ts(black_box(&hit)).is_some()));
        });
    }
    group.finish();
}

/// Pop/append churn: the shape of the reorder loop with the tree taken
/// out of the picture, so a superlinear `apply_op` can be attributed to
/// the log or ruled out.
fn bench_reorder_churn(c: &mut Criterion) {
    let mut group = c.benchmark_group("oplog/churn");
    for &window in &[10usize, 100, 1_000, 10_000] {
        // Popping `window` ops and appending the same ops back in
        // order restores the log exactly, so this iterates in place.
        //
        // The obvious alternative — `iter_batched` over a cloned log —
        // measures the clone and the drop of a 50k-op `Vec<LogOp>`
        // (160 B inline each, plus the heap behind every `Edit`,
        // `SetProp` and `Fractional`), which swamps the churn it is
        // supposed to price: it reported ~432 µs for a window of 10,
        // where the real work is nanoseconds. Restoring state in the
        // routine itself keeps the measured region honest.
        let mut log = filled_log(50_000);
        let mut undone: Vec<outl_core::op::LogOp> = Vec::with_capacity(window);
        group.throughput(Throughput::Elements(window as u64));
        group.bench_function(
            BenchmarkId::from_parameter(format!("pop_then_append_{window}")),
            |b| {
                b.iter(|| {
                    for _ in 0..window {
                        if let Some(op) = log.pop() {
                            undone.push(op);
                        }
                    }
                    while let Some(op) = undone.pop() {
                        log.append(op);
                    }
                    black_box(log.len())
                });
            },
        );
    }
    group.finish();
}

/// `edit_updates` — the per-node `Edit` index behind `block_text`.
///
/// A block with `edits` edits is looked up out of a log holding many
/// other nodes' edits. The documented contract is O(edits-of-node),
/// i.e. flat in the surrounding log size. **A curve that grows with
/// the log is the finding.**
fn bench_edit_updates(c: &mut Criterion) {
    let mut group = c.benchmark_group("oplog/edit_updates");
    let a = actors(1)[0];

    for &(tier, total_nodes) in TIERS {
        let mut log = OpLog::new();
        let mut ms = 1u64;
        // Every node gets 4 edits, spread through the log rather than
        // clustered, which is what real interleaved editing looks like.
        for round in 0..4 {
            for i in 0..total_nodes {
                ms += 1;
                log.append(log_op(
                    ms,
                    0,
                    a,
                    Op::Edit {
                        node: node(i),
                        text_op: vec![round as u8; 32],
                    },
                ));
            }
        }

        group.bench_with_input(BenchmarkId::from_parameter(tier), &log, |b, l| {
            b.iter(|| black_box(l.edit_updates(black_box(node(0))).count()));
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_append,
    bench_lookup,
    bench_reorder_churn,
    bench_edit_updates,
);
criterion_main!(benches);
