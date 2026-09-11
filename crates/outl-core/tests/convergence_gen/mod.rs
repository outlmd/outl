//! Generator and comparison helpers for the convergence property battery.
//!
//! **Single owner.** Three test binaries pull this in via
//! `mod convergence_gen;`: `convergence_property.rs` (the proptest
//! properties), `create_tree_invariants.rs` (the deterministic `Op::Create`
//! cycle regressions) and `create_undo_symmetry.rs` (the `do_op`/`undo_op`
//! inverse property, which uses `snapshot` only). A second copy of `lower()`,
//! `step_strategy()` or `snapshot()` would let them disagree about what a
//! "generated program" is, or about how much of the materialized state a
//! comparison looks at — and comparing too little is exactly how the defect
//! in [RFC 0263] stayed invisible. See `docs/crdt.md` → "Convergence property
//! suite" for why each generator shape exists.
//!
//! [RFC 0263]: ../../../docs/rfcs/0263-create-is-invertible.md
//!
//! Split out of `convergence_property.rs` at 838 lines, against a 900-line
//! hard stop. Behaviour is unchanged: the code moved, nothing was
//! renamed, and no test moved between binaries except the two deterministic
//! `Op::Create` regressions, which kept their names.

#![allow(dead_code)]

use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::property::PropValue;
use outl_core::tree::Tree;
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

#[path = "../common/mod.rs"]
mod common;
pub use common::Replica;

// --------------------------------------------------------------------------
// Full-state canonical key (stronger than common::assert_trees_equal, which
// compares only node parent+position). We need properties + collapsed too,
// because this suite generates SetProp / SetCollapsed / SnoozeRemind ops.
// --------------------------------------------------------------------------

/// A deterministic, total-ordered snapshot of *everything* the tree
/// materializes: node→(parent, position), every property binding, and the
/// collapsed set, and the snooze table. `BTree*` give a canonical order
/// so two snapshots are directly comparable with `==` (byte-identical
/// materialization).
#[derive(Debug, PartialEq, Eq)]
pub struct TreeSnapshot {
    pub nodes: BTreeMap<String, (String, String)>,
    pub properties: BTreeMap<(String, String), String>,
    pub collapsed: BTreeSet<String>,
    pub snoozed: BTreeMap<String, u64>,
}

pub fn snapshot(tree: &Tree) -> TreeSnapshot {
    let nodes = tree
        .iter_nodes()
        .map(|(n, p, pos)| (n.to_string(), (p.to_string(), pos.as_str().to_string())))
        .collect();

    // Every property binding, via `iter_properties` rather than a
    // `properties_of` call per node. That is not just the cheaper shape — it
    // is the **stronger** one. The per-node loop could only see properties
    // whose node is still in `nodes`, so a binding left behind on a node that
    // is no longer in the tree was invisible to every comparison in this
    // battery. A trashed node is still in `nodes` (trash is its parent) and
    // was always covered; a node removed by `undo_op(Create)` is not.
    let properties = tree
        .iter_properties()
        .map(|(node, key, value)| ((node.to_string(), key.to_string()), format!("{value:?}")))
        .collect();

    let collapsed = tree.collapsed_ids().map(|n| n.to_string()).collect();
    let snoozed = tree
        .snoozed_ids()
        .map(|(n, ms)| (n.to_string(), ms))
        .collect();

    TreeSnapshot {
        nodes,
        properties,
        collapsed,
        snoozed,
    }
}

/// Apply a full op set to a fresh replica and return its snapshot + log len.
pub fn materialize(ops: &[LogOp]) -> (TreeSnapshot, usize) {
    let mut r = Replica::new(ActorId::new());
    for op in ops {
        r.apply(op.clone());
    }
    (snapshot(&r.tree), r.log.len())
}

/// Deterministic permutation of `0..n` driven by a u64 seed (Fisher–Yates
/// with an inline xorshift). Keeps the suite free of an rng dev-dep and makes
/// the chosen order reproducible from the seed proptest shrinks.
pub fn permutation(n: usize, seed: u64) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..n).collect();
    let mut state = seed | 1; // never zero
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for i in (1..n).rev() {
        let j = (next() as usize) % (i + 1);
        idx.swap(i, j);
    }
    idx
}

// --------------------------------------------------------------------------
// Generator: a random op program over a small actor + node pool.
//
// We model the program abstractly (Step), then lower it to concrete `LogOp`s
// with globally unique, monotonic-per-actor HLCs. Lowering tracks which nodes
// already exist so a Create is only emitted once per node and Moves target
// existing nodes — this keeps every generated op *meaningful* (it changes
// state) which is what makes shrinking land on a real minimal counterexample.
// --------------------------------------------------------------------------

pub const N_ACTORS: usize = 4;
pub const N_NODES: usize = 6;

/// One abstract step in the generated program. Indices are into the actor /
/// node pools; the lowering pass resolves them to real ids.
#[derive(Clone, Debug)]
pub enum Step {
    /// Create node[n] under node[parent] (or root if parent==n).
    Create {
        actor: usize,
        n: usize,
        parent: usize,
    },
    /// Move node[n] under node[parent] (or root if parent==n).
    Move {
        actor: usize,
        n: usize,
        parent: usize,
    },
    /// Delete node[n] (Move → trash).
    Delete { actor: usize, n: usize },
    /// Set property `key` on node[n] to a Text value (or clear when None).
    SetProp {
        actor: usize,
        n: usize,
        key: u8,
        set: bool,
    },
    /// Set the collapsed flag of node[n].
    SetCollapsed { actor: usize, n: usize, value: bool },
    /// Snooze node[n]'s reminder until an epoch-ms instant (or clear it).
    SnoozeRemind { actor: usize, n: usize, set: bool },
}

prop_compose! {
    /// A single abstract step. `parent`/`n` overlap is fine — lowering maps
    /// `parent == n` to root, and concurrent self/ancestor moves are exactly
    /// the cycle cases we want to stress.
    pub fn step_strategy()(
        kind in 0u8..6,
        actor in 0usize..N_ACTORS,
        n in 0usize..N_NODES,
        parent in 0usize..N_NODES,
        key in 0u8..3,
        flag in any::<bool>(),
    ) -> Step {
        match kind {
            0 => Step::Create { actor, n, parent },
            1 => Step::Move { actor, n, parent },
            2 => Step::Delete { actor, n },
            3 => Step::SetProp { actor, n, key, set: flag },
            4 => Step::SnoozeRemind { actor, n, set: flag },
            _ => Step::SetCollapsed { actor, n, value: flag },
        }
    }
}

prop_compose! {
    /// A bounded program of steps.
    pub fn program_strategy()(steps in prop::collection::vec(step_strategy(), 1..40)) -> Vec<Step> {
        steps
    }
}

/// Stable per-test pools. Built once per case so ids are consistent across the
/// replicas we compare. Actors get distinct `ActorId`s (HLC tiebreak), nodes
/// distinct `NodeId`s.
pub struct Pools {
    pub actors: Vec<ActorId>,
    pub nodes: Vec<NodeId>,
}

impl Pools {
    pub fn new() -> Self {
        Self {
            actors: (0..N_ACTORS).map(|_| ActorId::new()).collect(),
            nodes: (0..N_NODES).map(|_| NodeId::new()).collect(),
        }
    }
}

/// Lower an abstract program to concrete `LogOp`s.
///
/// HLC assignment: `physical = step_index` (so every op is globally unique and
/// the program's textual order is one valid total order), `logical = 0`,
/// `actor` as the tiebreak. Monotonic-per-actor holds trivially because
/// physical strictly increases with step index. We deliberately do **not**
/// pre-sort; the convergence properties feed these in random orders.
///
/// Both `Create` and `Move` lower their `parent == n` case to `ROOT` (a node
/// can't be its own parent); every other parent is a real pool node, so the
/// full op surface — including a `Create` whose parent is already a descendant
/// of the node — is exercised. That path is what the cycle guard on
/// `Op::Create` exists for (see `create_respects_cycle_guard`); a cycle-forming
/// `Create` is a no-op on the tree but stays in the log, exactly like `Move`.
pub fn lower(program: &[Step], pools: &Pools) -> Vec<LogOp> {
    let root = NodeId::root();
    let trash = NodeId::trash();
    let mut ops = Vec::with_capacity(program.len());

    for (i, step) in program.iter().enumerate() {
        let physical = i as u64;
        // A **distinct** position per step, not a constant.
        //
        // Every op used to lower to `Fractional::parse("m")`, which made
        // sibling position unobservable: a replica could place a node at the
        // wrong step's position and still compare equal, because every step's
        // position was the same string. That blinded the suite to exactly the
        // class of divergence a broken undo produces — the node survives, at
        // the placement of the wrong op. The alphabet is `a..z` (see
        // `Fractional`), and 26 distinct values over a ≤40-step program is
        // enough for the collisions that remain to be rare rather than total.
        let pos = Fractional::parse(((b'a' + (i % 26) as u8) as char).to_string())
            .expect("valid position");

        let op = match step {
            Step::Create { actor, n, parent } => {
                let parent = if parent == n {
                    root
                } else {
                    pools.nodes[*parent]
                };
                // A repeat `Create` for a node that already has one is
                // emitted as-is, and that is the point.
                //
                // This used to be lowered to a `Move`, on the reasoning that
                // a second `Create` "is NOT a well-formed CRDT input" because
                // "the surviving placement would depend on which Create
                // arrived first". The first half is false and the second half
                // is backwards: which Create arrives first is exactly what
                // must NOT matter, and the lowest-HLC one winning is a
                // property of the op *set*, not of delivery order. Excluding
                // the shape did not make it well-formed, it made it untested
                // — and it is the single most common duplicate in production,
                // because page and journal roots are addressed by the
                // deterministic `NodeId::from_slug` so two devices opening the
                // same journal offline each emit a `Create` for one node id.
                // The generator hid a real non-convergence for as long as it
                // stood (RFC 0263, `tests/create_undo_symmetry.rs`).
                (
                    pools.actors[*actor],
                    Op::Create {
                        node: pools.nodes[*n],
                        parent,
                        position: pos,
                    },
                )
            }
            Step::Move { actor, n, parent } => {
                let parent = if parent == n {
                    root
                } else {
                    pools.nodes[*parent]
                };
                (
                    pools.actors[*actor],
                    Op::Move {
                        node: pools.nodes[*n],
                        new_parent: parent,
                        position: pos,
                        old_parent: NodeId::root(),
                        old_position: Fractional::first(),
                    },
                )
            }
            Step::Delete { actor, n } => (
                pools.actors[*actor],
                Op::Move {
                    node: pools.nodes[*n],
                    new_parent: trash,
                    position: pos,
                    old_parent: NodeId::root(),
                    old_position: Fractional::first(),
                },
            ),
            Step::SetProp { actor, n, key, set } => {
                let value = if *set {
                    Some(PropValue::Text(format!("v{key}")))
                } else {
                    None
                };
                (
                    pools.actors[*actor],
                    Op::SetProp {
                        node: pools.nodes[*n],
                        key: format!("k{key}"),
                        value,
                        old_value: None,
                    },
                )
            }
            Step::SetCollapsed { actor, n, value } => (
                pools.actors[*actor],
                Op::SetCollapsed {
                    node: pools.nodes[*n],
                    value: *value,
                    old_value: false,
                },
            ),
            Step::SnoozeRemind { actor, n, set } => (
                pools.actors[*actor],
                Op::SnoozeRemind {
                    node: pools.nodes[*n],
                    // The instant is derived from the step index so two
                    // snoozes of the same node carry different values —
                    // an always-equal value would make a reordering bug
                    // invisible.
                    until_ms: set.then(|| 1_700_000_000_000 + physical),
                    old_until_ms: None,
                },
            ),
        };

        ops.push(LogOp {
            ts: Hlc::new(physical, 0, op.0),
            actor: op.0,
            op: op.1,
        });
    }

    ops
}

// --------------------------------------------------------------------------
// Generator: the same op mix, over a tree built to make the cycle guard fire.
//
// `program_strategy()` above is broad and structurally thin. Measured by
// replaying its programs in HLC order and asking `Tree::creates_cycle` before
// each apply — which is what separates a genuine rejection from an op merely
// superseded by a later move — 1.2-1.5% of its structural ops are rejected,
// and only 10-13% of its programs contain a single rejection.
//
// The cause is not the node pool size, and it is not the op mix: half its
// steps are already `Create` / `Move` / `Delete`. It is that a cycle needs
// `node` to be an **ancestor** of `new_parent`, and these programs never
// build a tree tall enough to have ancestors — a mean of 2.39 live nodes at
// a mean depth of 1.33. A pool of six nodes spread over four actors, with a
// sixth of the steps trashing one, spends itself on breadth instead of depth,
// and `lower()` sends the one guaranteed cycle (`parent == n`) to ROOT.
//
// So the broad convergence assertions — all-pairs equality across N
// permutations, duplicated delivery, the late-op undo/redo round trip — are
// almost never made about a program in which the guard fired. The one
// generator that does fire it (`concurrent_moves_never_cycle`, 8.45%) is
// Move-only over a flat 4-node pool and compares two delivery orders.
// That is the gap this generator closes: a program dense in rejections that
// still carries the full op mix, so the *same* broad assertions can be made
// about it.
//
// Two structural choices do the work, and neither one narrows the op mix:
//
// 1. A deterministic chain prelude. Nodes 0..N_CHAIN are created as
//    node[i] under node[i-1], so node[i] is an ancestor of node[j] for
//    every i < j — ancestry the guard can actually walk.
// 2. A move's parent is `(n + offset) % N_CHAIN` with `offset >= 1`, never
//    `n` itself. While the chain stands, `parent > n` is an ancestor moved
//    under its own descendant (a cycle, at depth `parent - n`, so the
//    transitive walk is exercised and not just the immediate-parent case)
//    and `parent < n` is legal and re-forms a chain for the next move to
//    collide with.
//
// The lowering is shared with `program_strategy()` on purpose: two copies of
// `lower()` would let the two generators disagree about what a generated
// program *is*, which is the failure this module's header warns about.
// --------------------------------------------------------------------------

/// Length of the chain prelude. Five nodes give ancestry distances of 1 to 4,
/// so a guard that only compared `new_parent == node` or one level above it
/// fails here rather than passing on a two-node tree.
pub const N_CHAIN: usize = 5;

prop_compose! {
    /// One step of a cycle-dense program. Same six `Op` variants as
    /// [`step_strategy`], reweighted toward `Move` because `Move` is the op
    /// the guard is consulted for, and away from `Delete`, which both flattens
    /// the chain the next move needs and lands in the denominator without ever
    /// being rejectable (`TRASH_ROOT` is a sentinel, so the ancestor walk
    /// stops there). Every variant is still generated: this trades the broad
    /// generator's even sixths for density, it does not drop an op kind.
    pub fn cycle_dense_step()(
        kind in 0u8..16,
        actor in 0usize..N_ACTORS,
        n in 0usize..N_CHAIN,
        offset in 1usize..N_CHAIN,
        key in 0u8..3,
        flag in any::<bool>(),
    ) -> Step {
        // `offset >= 1` and the modulus keep `parent != n`, so a move is
        // never the degenerate self-cycle that `lower()` would send to ROOT
        // anyway. Every cycle this generator produces is a real ancestry
        // collision.
        let parent = (n + offset) % N_CHAIN;
        match kind {
            0 => Step::Create { actor, n, parent },
            1..=9 => Step::Move { actor, n, parent },
            10 => Step::Delete { actor, n },
            11 | 12 => Step::SetProp { actor, n, key, set: flag },
            13 | 14 => Step::SnoozeRemind { actor, n, set: flag },
            _ => Step::SetCollapsed { actor, n, value: flag },
        }
    }
}

prop_compose! {
    /// A chain prelude followed by a bounded cycle-dense body.
    ///
    /// The prelude is `Step::Create { n: i, parent: i - 1 }` for each i, with
    /// `i == 0` lowering to ROOT (`lower()` maps `parent == n` there). It is
    /// fixed rather than generated so every case starts from a tree with real
    /// ancestry; the body is what proptest varies and shrinks.
    pub fn cycle_dense_program_strategy()(
        // Four is the floor, not one: a shorter body often holds no `Move`
        // at all, and a program that never consults the guard is one this
        // generator exists to not produce. Small enough that shrinking still
        // lands on a readable counterexample.
        body in prop::collection::vec(cycle_dense_step(), 4..32),
    ) -> Vec<Step> {
        let mut program: Vec<Step> = (0..N_CHAIN)
            .map(|i| Step::Create {
                actor: 0,
                n: i,
                parent: i.saturating_sub(1),
            })
            .collect();
        program.extend(body);
        program
    }
}

/// How many of `ops`' structural ops (`Move` / `Create`) the cycle guard
/// would reject, and how many it is consulted for at all.
///
/// Returns `(rejected, consulted)`. The ops are replayed into a fresh tree in
/// HLC order and `Tree::creates_cycle` is asked **before** each apply, which
/// is what separates a genuine rejection from an op merely superseded by a
/// later move. Counting "the op's parent differs from the final tree's"
/// instead conflates the two and reports a number many times too large.
///
/// The guard is only *consulted* for a `Move` whose node already exists and a
/// `Create` whose node does not, which is why that denominator is reported
/// separately: a program of moves against nodes nobody created yet exercises
/// nothing, however many moves it holds.
pub fn cycle_rejections(ops: &[LogOp]) -> (usize, usize) {
    use outl_core::log::OpLog;

    let mut sorted: Vec<LogOp> = ops.to_vec();
    sorted.sort_by_key(|o| o.ts);

    let mut tree = Tree::new();
    let mut log = OpLog::new();
    let (mut rejected, mut consulted) = (0usize, 0usize);

    for op in sorted {
        match &op.op {
            Op::Move {
                node, new_parent, ..
            } if tree.parent(*node).is_some() => {
                consulted += 1;
                if tree.creates_cycle(*node, *new_parent) {
                    rejected += 1;
                }
            }
            Op::Create { node, parent, .. } if tree.parent(*node).is_none() => {
                consulted += 1;
                if tree.creates_cycle(*node, *parent) {
                    rejected += 1;
                }
            }
            _ => {}
        }
        tree.apply_op(&mut log, op);
    }

    (rejected, consulted)
}

// --------------------------------------------------------------------------
// Cycle detection helper: walk parent chains in a materialized snapshot and
// assert no node reaches itself. Root/trash terminate the walk.
// --------------------------------------------------------------------------

/// Returns the id of a node that participates in a cycle, if any.
pub fn find_cycle(tree: &Tree) -> Option<NodeId> {
    for (start, _, _) in tree.iter_nodes() {
        let mut cur = start;
        // Bounded by node_count + slack; a cycle reveals itself well before.
        for _ in 0..(tree.node_count() + 2) {
            match tree.parent(cur) {
                Some(p) => {
                    if p == start {
                        return Some(start);
                    }
                    cur = p;
                }
                None => break, // reached root / trash / detached
            }
        }
    }
    None
}
