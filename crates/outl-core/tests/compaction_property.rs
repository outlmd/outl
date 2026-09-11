//! Property-based **convergence** suite for op-log compaction.
//!
//! Sibling of `convergence_property.rs`, aimed at one claim: *a peer
//! replaying the compacted log materializes the same tree as one
//! replaying the original.*
//!
//! The generator deliberately produces the shapes that break a naive
//! predicate, not just the shape compaction is designed to remove:
//!
//! - the redundant `Create` + `Move` pair itself (the thing being
//!   dropped);
//! - `Move(node, TRASH_ROOT)` deletes, followed by a `Create` + `Move`
//!   pair restoring the same node — the case where `Create` is an
//!   idempotent no-op and the `Move` is the op doing the work;
//! - multiple actors writing interleaved HLCs, so a peer's op can land
//!   between a pair;
//! - a second `Create` for an already-created node (a derivable id);
//! - relocations and reorders that must survive untouched.
//!
//! Convergence is asserted twice over: in HLC order (the canonical
//! materialization) against the full generator, and under random
//! delivery permutations through `Tree::apply_op` — which is what a real
//! peer does — against a generator restricted to one `Create` per node.
//!
//! **Why that restriction is not compaction dodging its own test.** A
//! node with two `Create` ops does not converge under reordering in the
//! CRDT as it stands, independently of anything here:
//! `undo_op(Op::Create)` removes the node outright, so a `Move` replayed
//! after that undo finds nothing to move and the redone `Create`
//! reinserts the node at *its* position. Three ops reproduce it —
//! `Create(n,P,"a")@1`, `Move(n,P,"t")@2`, `Create(n,P,"u")@3` — which
//! materialize `"t"` in HLC order and `"u"` delivered `[3,1,2]`. Asking
//! the compacted log to converge under reordering on a program that
//! already does not would be testing that defect, not this module.
//! Compaction never drops an op for a node with more than one `Create`
//! (predicate condition 3), so the restriction removes nothing it is
//! responsible for.

use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::log::OpLog;
use outl_core::op::{LogOp, Op};
use outl_core::storage::compact::{apply_compaction, plan_compaction, CompactOptions};
use outl_core::tree::Tree;
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use tempfile::TempDir;

// --------------------------------------------------------------------------
// Program generator
// --------------------------------------------------------------------------

/// One generated step, resolved into ops against a live model so every
/// `Create` / `Move` names a node that makes sense at that point.
#[derive(Clone, Debug)]
enum Step {
    /// The shape compaction targets: `Create` then an identical `Move`.
    RedundantPair { actor: usize, parent: usize },
    /// A plain `Create` with no follow-up.
    Create { actor: usize, parent: usize },
    /// Relocate an existing node under another one.
    Relocate {
        actor: usize,
        node: usize,
        parent: usize,
    },
    /// `Move(node, TRASH_ROOT)` — how deletion works.
    Trash { actor: usize, node: usize },
    /// Re-`Create` an existing node, then `Move` it to a page. On a live
    /// node the `Create` is a no-op and the `Move` is load-bearing.
    Restore {
        actor: usize,
        node: usize,
        parent: usize,
    },
    /// A second `Create` for a node that already exists — the derivable
    /// id shape.
    DuplicateCreate { actor: usize, node: usize },
    /// An edit, which never touches the tree but does populate the log.
    Edit { actor: usize, node: usize },
}

fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => (0usize..3, 0usize..8).prop_map(|(actor, parent)| Step::RedundantPair { actor, parent }),
        2 => (0usize..3, 0usize..8).prop_map(|(actor, parent)| Step::Create { actor, parent }),
        2 => (0usize..3, 0usize..8, 0usize..8)
            .prop_map(|(actor, node, parent)| Step::Relocate { actor, node, parent }),
        1 => (0usize..3, 0usize..8).prop_map(|(actor, node)| Step::Trash { actor, node }),
        2 => (0usize..3, 0usize..8, 0usize..8)
            .prop_map(|(actor, node, parent)| Step::Restore { actor, node, parent }),
        1 => (0usize..3, 0usize..8).prop_map(|(actor, node)| Step::DuplicateCreate { actor, node }),
        1 => (0usize..3, 0usize..8).prop_map(|(actor, node)| Step::Edit { actor, node }),
    ]
}

fn program_strategy() -> impl Strategy<Value = Vec<Step>> {
    prop::collection::vec(step_strategy(), 1..40)
}

/// The same generator minus the two steps that give one node a second
/// `Create`. See the module doc for why the reordering properties need
/// it.
fn single_create_step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => (0usize..3, 0usize..8).prop_map(|(actor, parent)| Step::RedundantPair { actor, parent }),
        2 => (0usize..3, 0usize..8).prop_map(|(actor, parent)| Step::Create { actor, parent }),
        3 => (0usize..3, 0usize..8, 0usize..8)
            .prop_map(|(actor, node, parent)| Step::Relocate { actor, node, parent }),
        2 => (0usize..3, 0usize..8).prop_map(|(actor, node)| Step::Trash { actor, node }),
        1 => (0usize..3, 0usize..8).prop_map(|(actor, node)| Step::Edit { actor, node }),
    ]
}

fn single_create_program_strategy() -> impl Strategy<Value = Vec<Step>> {
    prop::collection::vec(single_create_step_strategy(), 1..40)
}

/// Deterministic fractional positions, so a generated `Move` can either
/// restate a node's position exactly or genuinely change it.
fn position(i: usize) -> Fractional {
    Fractional::parse(match i % 4 {
        0 => "a",
        1 => "m",
        2 => "t",
        _ => "u",
    })
    .expect("valid fractional")
}

/// Turn a program into a log: per-actor monotonic HLCs interleaved on a
/// shared physical counter, so different actors' ops genuinely interleave
/// in the merged order.
fn build(program: &[Step], actors: &[ActorId], pages: &[NodeId]) -> Vec<LogOp> {
    let mut ops: Vec<LogOp> = Vec::new();
    let mut clock = 1000u64;
    let mut live: Vec<NodeId> = Vec::new();

    // Every page root, created once so `Relocate` has real parents.
    for (i, page) in pages.iter().enumerate() {
        ops.push(LogOp {
            ts: Hlc::new(clock, 0, actors[0]),
            actor: actors[0],
            op: Op::Create {
                node: *page,
                parent: NodeId::root(),
                position: position(i),
            },
        });
        clock += 1;
    }

    let tick = |clock: &mut u64| {
        *clock += 1;
        *clock
    };
    for (idx, step) in program.iter().enumerate() {
        match step {
            Step::RedundantPair { actor, parent } => {
                let a = actors[*actor % actors.len()];
                let p = pages[*parent % pages.len()];
                let node = NodeId::new();
                let pos = position(idx);
                let t1 = tick(&mut clock);
                ops.push(LogOp {
                    ts: Hlc::new(t1, 0, a),
                    actor: a,
                    op: Op::Create {
                        node,
                        parent: p,
                        position: pos.clone(),
                    },
                });
                let t2 = tick(&mut clock);
                ops.push(LogOp {
                    ts: Hlc::new(t2, 0, a),
                    actor: a,
                    op: Op::Move {
                        node,
                        new_parent: p,
                        position: pos,
                        old_parent: NodeId::root(),
                        old_position: Fractional::first(),
                    },
                });
                live.push(node);
            }
            Step::Create { actor, parent } => {
                let a = actors[*actor % actors.len()];
                let p = pages[*parent % pages.len()];
                let node = NodeId::new();
                let t = tick(&mut clock);
                ops.push(LogOp {
                    ts: Hlc::new(t, 0, a),
                    actor: a,
                    op: Op::Create {
                        node,
                        parent: p,
                        position: position(idx),
                    },
                });
                live.push(node);
            }
            Step::Relocate {
                actor,
                node,
                parent,
            } => {
                let Some(n) = live.get(*node % live.len().max(1)).copied() else {
                    continue;
                };
                let a = actors[*actor % actors.len()];
                let p = pages[*parent % pages.len()];
                let t = tick(&mut clock);
                ops.push(LogOp {
                    ts: Hlc::new(t, 0, a),
                    actor: a,
                    op: Op::Move {
                        node: n,
                        new_parent: p,
                        position: position(idx),
                        old_parent: NodeId::root(),
                        old_position: Fractional::first(),
                    },
                });
            }
            Step::Trash { actor, node } => {
                let Some(n) = live.get(*node % live.len().max(1)).copied() else {
                    continue;
                };
                let a = actors[*actor % actors.len()];
                let t = tick(&mut clock);
                ops.push(LogOp {
                    ts: Hlc::new(t, 0, a),
                    actor: a,
                    op: Op::Move {
                        node: n,
                        new_parent: NodeId::trash(),
                        position: Fractional::first(),
                        old_parent: NodeId::root(),
                        old_position: Fractional::first(),
                    },
                });
            }
            Step::Restore {
                actor,
                node,
                parent,
            } => {
                let Some(n) = live.get(*node % live.len().max(1)).copied() else {
                    continue;
                };
                let a = actors[*actor % actors.len()];
                let p = pages[*parent % pages.len()];
                let pos = position(idx);
                let t1 = tick(&mut clock);
                ops.push(LogOp {
                    ts: Hlc::new(t1, 0, a),
                    actor: a,
                    op: Op::Create {
                        node: n,
                        parent: p,
                        position: pos.clone(),
                    },
                });
                let t2 = tick(&mut clock);
                ops.push(LogOp {
                    ts: Hlc::new(t2, 0, a),
                    actor: a,
                    op: Op::Move {
                        node: n,
                        new_parent: p,
                        position: pos,
                        old_parent: NodeId::root(),
                        old_position: Fractional::first(),
                    },
                });
            }
            Step::DuplicateCreate { actor, node } => {
                let Some(n) = live.get(*node % live.len().max(1)).copied() else {
                    continue;
                };
                let a = actors[*actor % actors.len()];
                let t = tick(&mut clock);
                ops.push(LogOp {
                    ts: Hlc::new(t, 0, a),
                    actor: a,
                    op: Op::Create {
                        node: n,
                        parent: pages[0],
                        position: position(idx),
                    },
                });
            }
            Step::Edit { actor, node } => {
                let Some(n) = live.get(*node % live.len().max(1)).copied() else {
                    continue;
                };
                let a = actors[*actor % actors.len()];
                let t = tick(&mut clock);
                ops.push(LogOp {
                    ts: Hlc::new(t, 0, a),
                    actor: a,
                    op: Op::Edit {
                        node: n,
                        text_op: vec![idx as u8],
                    },
                });
            }
        }
    }
    ops
}

// --------------------------------------------------------------------------
// Workspace round trip
// --------------------------------------------------------------------------

fn write_workspace(ops: &[LogOp]) -> TempDir {
    let tmp = TempDir::new().expect("tempdir");
    let dir = tmp.path().join("ops");
    std::fs::create_dir_all(&dir).expect("mkdir ops");
    std::fs::create_dir_all(tmp.path().join(".outl")).expect("mkdir .outl");
    let mut by_actor: BTreeMap<ActorId, Vec<&LogOp>> = BTreeMap::new();
    for op in ops {
        by_actor.entry(op.actor).or_default().push(op);
    }
    for (actor, mut log) in by_actor {
        // A device appends its own ops in the order it produced them.
        log.sort_by_key(|op| op.ts);
        let body: String = log
            .iter()
            .map(|op| serde_json::to_string(op).expect("serialize") + "\n")
            .collect();
        std::fs::write(dir.join(format!("ops-{actor}.jsonl")), body).expect("write");
    }
    tmp
}

fn read_workspace(root: &std::path::Path) -> Vec<LogOp> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root.join("ops")).expect("read dir") {
        let path = entry.expect("entry").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if !(name.starts_with("ops-") && name.ends_with(".jsonl")) {
            continue;
        }
        for line in std::fs::read_to_string(&path).expect("read").lines() {
            if !line.trim().is_empty() {
                out.push(serde_json::from_str(line).expect("parse"));
            }
        }
    }
    out
}

/// Canonical, comparable materialization of the whole tree.
type Snapshot = BTreeMap<String, (String, String)>;

fn snapshot(tree: &Tree) -> Snapshot {
    tree.iter_nodes()
        .map(|(n, p, pos)| (n.to_string(), (p.to_string(), pos.as_str().to_string())))
        .collect()
}

/// Deliver `ops` in `order` through the real `apply_op`, which reorders
/// via undo/redo exactly as a peer receiving them out of order would.
fn deliver(ops: &[LogOp], order: &[usize]) -> Snapshot {
    let mut tree = Tree::new();
    let mut log = OpLog::new();
    for &i in order {
        tree.apply_op(&mut log, ops[i].clone());
    }
    snapshot(&tree)
}

fn hlc_order(ops: &[LogOp]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..ops.len()).collect();
    idx.sort_by_key(|&i| ops[i].ts);
    idx
}

/// Cheap deterministic shuffle — proptest seeds `seed`, so a failure
/// shrinks to a reproducible permutation.
fn shuffled(len: usize, mut seed: u64) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..len).collect();
    for i in (1..len).rev() {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        idx.swap(i, (seed >> 33) as usize % (i + 1));
    }
    idx
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 120, ..ProptestConfig::default() })]

    /// The correctness bar: compaction is convergence-preserving.
    #[test]
    fn compaction_preserves_the_materialized_tree(program in program_strategy()) {
        let actors: Vec<ActorId> = (0..3).map(|_| ActorId::new()).collect();
        let pages: Vec<NodeId> = (0..3).map(|_| NodeId::new()).collect();
        let ops = build(&program, &actors, &pages);

        let tmp = write_workspace(&ops);
        let before = read_workspace(tmp.path());
        let expected = deliver(&before, &hlc_order(&before));

        let plan = plan_compaction(tmp.path(), &CompactOptions { horizon_ms: 0 })
            .expect("plan");
        apply_compaction(tmp.path(), &plan).expect("apply");

        let after = read_workspace(tmp.path());
        prop_assert_eq!(
            after.len() + plan.report().ops_dropped,
            before.len(),
            "the rewrite dropped a different number of ops than it planned"
        );
        prop_assert_eq!(deliver(&after, &hlc_order(&after)), expected);
    }

    /// A peer does not receive the log in HLC order. The property that
    /// matters is therefore not "the compacted log converges to its own
    /// HLC replay" but the stronger pairwise one: **under the same
    /// arbitrary delivery order, the compacted log materializes what the
    /// original does.**
    ///
    /// Stated that way it also stays honest about a CRDT wrinkle this
    /// suite deliberately generates and does not own: a node with two
    /// `Create` ops does not converge under reordering, because
    /// `undo_op(Create)` removes the node outright while the earlier
    /// `Create` should have kept it alive. Comparing the two logs under
    /// one order isolates compaction's contribution from that.
    #[test]
    fn the_compacted_log_matches_the_original_under_any_delivery_order(
        program in single_create_program_strategy(),
        seeds in prop::array::uniform3(any::<u64>()),
    ) {
        let actors: Vec<ActorId> = (0..3).map(|_| ActorId::new()).collect();
        let pages: Vec<NodeId> = (0..3).map(|_| NodeId::new()).collect();
        let ops = build(&program, &actors, &pages);

        let tmp = write_workspace(&ops);
        let before = read_workspace(tmp.path());

        let plan = plan_compaction(tmp.path(), &CompactOptions { horizon_ms: 0 })
            .expect("plan");
        apply_compaction(tmp.path(), &plan).expect("apply");
        let kept: BTreeSet<Hlc> = read_workspace(tmp.path()).iter().map(|o| o.ts).collect();

        for seed in seeds {
            let order = shuffled(before.len(), seed);
            let delivered: Vec<LogOp> = order.iter().map(|&i| before[i].clone()).collect();
            let compacted: Vec<LogOp> = delivered
                .iter()
                .filter(|op| kept.contains(&op.ts))
                .cloned()
                .collect();
            let straight: Vec<usize> = (0..delivered.len()).collect();
            let straight_c: Vec<usize> = (0..compacted.len()).collect();
            prop_assert_eq!(
                deliver(&compacted, &straight_c),
                deliver(&delivered, &straight)
            );
        }
    }

    /// Gate on the restricted generator: the ORIGINAL log converges
    /// under reordering, so the property above is really measuring
    /// compaction rather than inheriting the double-`Create` defect the
    /// module doc describes. If this ever fails, the shape is a CRDT
    /// question and the property above is meaningless until it is
    /// answered.
    #[test]
    fn the_generated_log_itself_converges(
        program in single_create_program_strategy(),
        seed in any::<u64>(),
    ) {
        let actors: Vec<ActorId> = (0..3).map(|_| ActorId::new()).collect();
        let pages: Vec<NodeId> = (0..3).map(|_| NodeId::new()).collect();
        let ops = build(&program, &actors, &pages);
        let expected = deliver(&ops, &hlc_order(&ops));
        let order = shuffled(ops.len(), seed);
        prop_assert_eq!(deliver(&ops, &order), expected);
    }

    /// Compaction never removes an op whose application would change the
    /// tree — stated independently of the tree comparison, so a bug that
    /// cancels itself out across two ops still fails.
    #[test]
    fn every_dropped_op_is_a_move_that_restates_its_own_create(
        program in program_strategy(),
    ) {
        let actors: Vec<ActorId> = (0..3).map(|_| ActorId::new()).collect();
        let pages: Vec<NodeId> = (0..3).map(|_| NodeId::new()).collect();
        let ops = build(&program, &actors, &pages);

        let tmp = write_workspace(&ops);
        let before = read_workspace(tmp.path());
        let plan = plan_compaction(tmp.path(), &CompactOptions { horizon_ms: 0 })
            .expect("plan");
        apply_compaction(tmp.path(), &plan).expect("apply");
        let after: BTreeSet<Hlc> = read_workspace(tmp.path()).iter().map(|o| o.ts).collect();

        // Replay the ORIGINAL log in HLC order; at each dropped op the
        // tree must already hold exactly what the Move would write.
        let mut tree = Tree::new();
        let mut log = OpLog::new();
        for &i in &hlc_order(&before) {
            let op = &before[i];
            if !after.contains(&op.ts) {
                let Op::Move { node, new_parent, position, .. } = &op.op else {
                    prop_assert!(false, "a dropped op was not a Move: {:?}", op.op);
                    unreachable!()
                };
                prop_assert_eq!(tree.parent(*node), Some(*new_parent));
                prop_assert_eq!(tree.position(*node), Some(position));
                prop_assert_ne!(*new_parent, NodeId::trash(), "a delete is never inert");
            }
            tree.apply_op(&mut log, op.clone());
        }
    }
}
