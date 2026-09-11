//! Benchmarks the tree CRDT itself and the `Tree` read accessors.
//!
//! Two things live here because they are two halves of one question —
//! *what does a tree operation cost at workspace scale*:
//!
//! - **The algorithm.** `do_op` / `undo_op` / `creates_cycle`, plus the
//!   `apply_op` reorder loop at increasing undo-window depths. These
//!   are the four functions root `CLAUDE.md` invariant 3 pins to the
//!   paper, so their cost is a *constraint*, not a free variable: a
//!   faster `apply_op` that diverges is a defect, not an optimization.
//! - **The accessors.** `Tree` keeps `nodes` as a
//!   `HashMap<NodeId, (NodeId, Fractional)>` with **no reverse
//!   parent→children index**, and properties in a `(NodeId, String)`
//!   map. `docs/primitives-actions.md` documents the consequence: a
//!   `children_of`-shaped read rescans every node, and callers walking
//!   the whole tree are told to build an index by hand. The
//!   `children/*` group measures what that costs and what a caller who
//!   skipped the advice pays.
//!
//! Run:
//! ```text
//! cargo bench -p outl-core --bench tree
//! cargo bench -p outl-core --bench tree -- children   # one group
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use outl_core::fractional::Fractional;
use outl_core::id::NodeId;
use outl_core::log::OpLog;
use outl_core::op::Op;
use outl_core::tree::Tree;
use std::hint::black_box;

#[path = "common.rs"]
mod common;

use common::{
    actors, build_children_index, build_tree, children_by_scan, create_op, log_op, node, Shape,
    TIERS, WINDOWS,
};

/// `apply_op` on the in-order fast path: every op's HLC is above the
/// log tail, so the undo window is empty and this is an append plus a
/// `contains_ts` binary search. This is the common case — a local
/// edit — and the number to protect.
fn bench_apply_in_order(c: &mut Criterion) {
    let mut group = c.benchmark_group("apply_op/in_order");
    for &(tier, count) in TIERS {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_function(BenchmarkId::from_parameter(tier), |b| {
            b.iter(|| {
                let a = actors(1)[0];
                let mut tree = Tree::new();
                let mut log = OpLog::new();
                for i in 0..count {
                    tree.apply_op(&mut log, create_op(i as u64 + 1, a, i, NodeId::root()));
                }
                black_box(tree.node_count())
            });
        });
    }
    group.finish();
}

/// `apply_op` when a late op lands `window` entries below the tail.
///
/// This is the paper's Fig. 4 undo/replay loop: pop and `undo_op`
/// every newer entry, `do_op` the arrival, then `do_op` each undone op
/// again. Cost is O(window) with two tree mutations per entry, so this
/// group should come out linear in `window`. **A superlinear curve
/// here is the finding** — it would mean something inside the loop is
/// itself scanning.
///
/// Each iteration re-clones the prepared state so the measured region
/// is one `apply_op`, not a growing log.
fn bench_apply_reorder(c: &mut Criterion) {
    let mut group = c.benchmark_group("apply_op/reorder");
    let a = actors(1)[0];

    for &window in WINDOWS {
        // Build a log whose tail is `window` ops above the arrival's
        // HLC. Physical times start at 1000 so the late op can sit
        // below every one of them and still be positive.
        let mut tree = Tree::new();
        let mut log = OpLog::new();
        for i in 0..window {
            tree.apply_op(
                &mut log,
                create_op(1_000 + i as u64 + 1, a, i, NodeId::root()),
            );
        }

        // The late arrival: a `Move` of an existing node, so `do_op`
        // does real work (cycle check + reparent) rather than hitting
        // `Create`'s idempotent early return.
        let late = log_op(
            500,
            0,
            a,
            Op::Move {
                node: node(0),
                new_parent: NodeId::root(),
                position: Fractional::last(),
                old_parent: NodeId::root(),
                old_position: Fractional::first(),
            },
        );

        group.throughput(Throughput::Elements(window.max(1) as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("window_{window}")),
            &(tree, log, late),
            |b, (t, l, op)| {
                b.iter_batched(
                    || (t.clone(), l.clone(), op.clone()),
                    |(mut t, mut l, op)| {
                        t.apply_op(&mut l, op);
                        black_box(l.len())
                    },
                    criterion::BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

/// `creates_cycle` walks from the prospective new parent up to root.
/// Cost is therefore O(depth), and the question is what real depth is.
/// `Chain` is the pathological shape; `Bushy` is the realistic one
/// (depth ≈ log_branching(n)).
fn bench_creates_cycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("creates_cycle");

    for &depth in &[10usize, 100, 1_000, 10_000] {
        let (tree, _log) = build_tree(depth, Shape::Chain);
        let deepest = node(depth - 1);
        group.bench_with_input(
            BenchmarkId::new("chain_depth", depth),
            &(tree, deepest),
            |b, (t, d)| {
                // Ask whether root's subtree contains the deepest node
                // — the full walk, never short-circuited.
                b.iter(|| black_box(t.creates_cycle(black_box(node(0)), black_box(*d))));
            },
        );
    }

    for &(tier, count) in TIERS {
        let (tree, _log) = build_tree(count, Shape::Bushy { branching: 8 });
        let deepest = node(count - 1);
        group.bench_with_input(
            BenchmarkId::new("bushy_b8", tier),
            &(tree, deepest),
            |b, (t, d)| {
                b.iter(|| black_box(t.creates_cycle(black_box(node(0)), black_box(*d))));
            },
        );
    }
    group.finish();
}

/// The accessor gap: one `children_of`-shaped read costs a full scan
/// of every node, because `Tree` holds no reverse index.
///
/// Three measurements, and the comparison between them is the point:
///
/// - `scan_one` — a single lookup. O(n).
/// - `build_index` — the whole-tree index `docs/primitives-actions.md`
///   tells callers to build. Also O(n), paid once.
/// - `scan_all_nodes` — a caller that walks the tree calling the
///   scanning accessor per node. **O(n²)**, and the shape this bench
///   exists to price. Capped at the small tiers because the large one
///   would not finish.
fn bench_children(c: &mut Criterion) {
    let mut group = c.benchmark_group("children");

    for &(tier, count) in TIERS {
        let (tree, _log) = build_tree(count, Shape::Bushy { branching: 8 });

        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::new("scan_one", tier), &tree, |b, t| {
            b.iter(|| black_box(children_by_scan(t, black_box(node(0))).len()));
        });

        group.bench_with_input(BenchmarkId::new("build_index", tier), &tree, |b, t| {
            b.iter(|| black_box(build_children_index(t).len()));
        });
    }

    // The quadratic caller. Small tiers only — at 68k nodes this is
    // ~4.6 billion node visits and would dominate the whole suite,
    // which is itself the finding.
    for &count in &[500usize, 1_000, 2_000, 4_000] {
        let (tree, _log) = build_tree(count, Shape::Bushy { branching: 8 });
        group.throughput(Throughput::Elements((count * count) as u64));
        group.bench_with_input(
            BenchmarkId::new("scan_all_nodes_quadratic", count),
            &tree,
            |b, t| {
                b.iter(|| {
                    let mut total = 0usize;
                    for i in 0..count {
                        total += children_by_scan(t, node(i)).len();
                    }
                    black_box(total)
                });
            },
        );
    }
    group.finish();
}

/// Property reads over the `(NodeId, String)` map: one node's
/// properties, and the transpose (`nodes_with_property`) the docs
/// describe as O(total properties).
fn bench_properties(c: &mut Criterion) {
    let mut group = c.benchmark_group("properties");

    for &(tier, count) in TIERS {
        let (mut tree, mut log) = build_tree(count, Shape::Bushy { branching: 8 });
        // 3 properties on every node — `title::`, `tags::`, and one
        // more is a fair read of a real page.
        common::add_properties(&mut tree, &mut log, count, 3);

        group.throughput(Throughput::Elements(tree.property_count() as u64));
        group.bench_with_input(BenchmarkId::new("properties_of", tier), &tree, |b, t| {
            b.iter(|| black_box(t.properties_of(black_box(node(0))).count()));
        });

        group.bench_with_input(
            BenchmarkId::new("nodes_with_property", tier),
            &tree,
            |b, t| {
                b.iter(|| black_box(t.nodes_with_property(black_box("key0")).count()));
            },
        );

        group.bench_with_input(BenchmarkId::new("iter_properties", tier), &tree, |b, t| {
            b.iter(|| black_box(t.iter_properties().count()));
        });
    }

    // The pathology `properties_of`'s own doc comment describes:
    // calling it once per node is O(nodes × properties), and on the
    // reference workspace that "made a whole-tree walk slower than
    // re-reading every `.md` off disk". `iter_properties` is the
    // documented fix. Benching both at the same sizes turns that
    // sentence into a ratio, which is what a caller choosing between
    // them actually needs.
    for &count in &[500usize, 1_000, 2_000, 4_000] {
        let (mut tree, mut log) = build_tree(count, Shape::Bushy { branching: 8 });
        common::add_properties(&mut tree, &mut log, count, 3);

        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::new("walk_via_properties_of_quadratic", count),
            &tree,
            |b, t| {
                b.iter(|| {
                    let mut total = 0usize;
                    for i in 0..count {
                        total += t.properties_of(node(i)).count();
                    }
                    black_box(total)
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("walk_via_iter_properties_linear", count),
            &tree,
            |b, t| {
                b.iter(|| {
                    let mut grouped: std::collections::HashMap<NodeId, usize> =
                        std::collections::HashMap::new();
                    for (n, _, _) in t.iter_properties() {
                        *grouped.entry(n).or_default() += 1;
                    }
                    black_box(grouped.len())
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_apply_in_order,
    bench_apply_reorder,
    bench_creates_cycle,
    bench_children,
    bench_properties,
);
criterion_main!(benches);
