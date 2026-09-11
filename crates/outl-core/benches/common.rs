//! Synthetic trees and op logs for the `outl-core` benchmarks.
//!
//! Every bench binary in `benches/` pulls this in with
//! `#[path = "common.rs"] mod common;` — the same pattern
//! `crates/outl-md/benches/common.rs` established.
//!
//! The shapes here track the maintainer's real workspace, so a number
//! from a bench is comparable to a number from production rather than
//! to a microbenchmark nobody experiences: **67,939 nodes / 217,811 ops
//! / 2,574 pages / 20 actors**. The tiers bracket it — `large` is
//! roughly that workspace, `huge` is the next order of magnitude so a
//! quadratic term shows up as a curve rather than as one slow point.

// Each bench binary uses a different subset of these helpers, and a
// `dead_code` warning per unused-per-binary item isn't actionable.
#![allow(dead_code)]

use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::log::OpLog;
use outl_core::op::{LogOp, Op};
use outl_core::property::PropValue;
use outl_core::tree::Tree;

/// Node-count tiers. `large` ≈ the reference workspace.
pub const TIERS: &[(&str, usize)] = &[("1k", 1_000), ("10k", 10_000), ("68k", 67_939)];

/// Reorder-window depths for the `apply_op` undo/replay loop.
///
/// A window of 0 is the in-order fast path (an append). Everything
/// above it is a late op forcing undo + redo of that many entries —
/// the cost the paper's Fig. 4 buys convergence with.
pub const WINDOWS: &[usize] = &[0, 1, 10, 100, 1_000, 10_000];

/// Deterministic node id for index `i`, so two runs of a bench build
/// byte-identical trees and criterion compares like with like.
pub fn node(i: usize) -> NodeId {
    NodeId::from_seed(b"outl-bench-node:", &i.to_string())
}

/// `n` distinct actor ids.
///
/// `ActorId` has no seeded constructor, so these are freshly generated
/// rather than reproducible across runs. That is fine and deliberate:
/// a bench's timing depends on how *many* actors interleave and on the
/// HLC ordering the caller assigns, never on the id bytes. Everything
/// that does affect timing — node ids, tree shape, op order — is
/// deterministic.
pub fn actors(n: usize) -> Vec<ActorId> {
    (0..n).map(|_| ActorId::new()).collect()
}

/// A `LogOp` at a synthetic HLC. `ms` is the physical component, so
/// callers control ordering directly and never touch a wall clock —
/// a bench that reads the real clock is not reproducible.
pub fn log_op(ms: u64, logical: u32, a: ActorId, op: Op) -> LogOp {
    LogOp {
        ts: Hlc::new(ms, logical, a),
        actor: a,
        op,
    }
}

/// A `Create` op placing `node(i)` under `parent`.
pub fn create_op(ms: u64, a: ActorId, i: usize, parent: NodeId) -> LogOp {
    log_op(
        ms,
        0,
        a,
        Op::Create {
            node: node(i),
            parent,
            position: Fractional::first(),
        },
    )
}

/// Shape of a generated tree.
#[derive(Debug, Clone, Copy)]
pub enum Shape {
    /// Every node hangs directly off the root. Depth 1, maximal
    /// sibling fan-out — the worst case for any "scan siblings"
    /// accessor and the best case for `creates_cycle`.
    Flat,
    /// A single chain root → n1 → n2 → … Depth n, fan-out 1 — the
    /// worst case for `creates_cycle`, which walks to root.
    Chain,
    /// `branching` children per node, filled breadth-first. The
    /// realistic middle: an outline is neither flat nor a list.
    Bushy { branching: usize },
}

/// Build a tree of `count` nodes in `shape`, plus the log that
/// produced it. Returns both because `apply_op` needs the log and the
/// accessor benches need the tree.
pub fn build_tree(count: usize, shape: Shape) -> (Tree, OpLog) {
    let a = ActorId::new();
    let mut tree = Tree::new();
    let mut log = OpLog::new();

    for i in 0..count {
        let parent = match shape {
            Shape::Flat => NodeId::root(),
            Shape::Chain => {
                if i == 0 {
                    NodeId::root()
                } else {
                    node(i - 1)
                }
            }
            Shape::Bushy { branching } => {
                if i == 0 {
                    NodeId::root()
                } else {
                    node((i - 1) / branching.max(1))
                }
            }
        };
        // `ms` strictly increasing keeps every append on the in-order
        // fast path, so building the fixture costs no reorder.
        tree.apply_op(&mut log, create_op(i as u64 + 1, a, i, parent));
    }

    (tree, log)
}

/// Attach `per_node` properties to the first `nodes` nodes of `tree`.
///
/// Properties live in a `(NodeId, String)`-keyed map, so this is what
/// makes `properties_of` / `nodes_with_property` cost anything.
pub fn add_properties(tree: &mut Tree, log: &mut OpLog, nodes: usize, per_node: usize) {
    let a = ActorId::new();
    let mut ms = 1_000_000u64;
    for i in 0..nodes {
        for k in 0..per_node {
            ms += 1;
            tree.apply_op(
                log,
                log_op(
                    ms,
                    0,
                    a,
                    Op::SetProp {
                        node: node(i),
                        key: format!("key{k}"),
                        value: Some(PropValue::Text(format!("value-{i}-{k}"))),
                        old_value: None,
                    },
                ),
            );
        }
    }
}

/// The children index every whole-tree caller has to build by hand
/// today, because `Tree` keeps no reverse parent→children map.
///
/// Reproduced here rather than imported so the bench measures the
/// **cost callers actually pay**, and so this file has no opinion
/// about where the real one should live.
pub fn build_children_index(
    tree: &Tree,
) -> std::collections::HashMap<NodeId, Vec<(NodeId, Fractional)>> {
    let mut idx: std::collections::HashMap<NodeId, Vec<(NodeId, Fractional)>> =
        std::collections::HashMap::new();
    for (n, parent, pos) in tree.iter_nodes() {
        idx.entry(parent).or_default().push((n, pos.clone()));
    }
    for children in idx.values_mut() {
        children.sort_by(|a, b| a.1.cmp(&b.1));
    }
    idx
}

/// One node's children by scanning every node — the shape
/// `docs/primitives-actions.md` documents as the cost of calling a
/// `children_of` accessor without a prebuilt index.
pub fn children_by_scan(tree: &Tree, parent: NodeId) -> Vec<NodeId> {
    let mut out: Vec<(NodeId, Fractional)> = tree
        .iter_nodes()
        .filter(|(_, p, _)| *p == parent)
        .map(|(n, _, pos)| (n, pos.clone()))
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out.into_iter().map(|(n, _)| n).collect()
}
