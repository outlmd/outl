//! Root `CLAUDE.md` invariants 1 and 2 for `trash::restore`: two
//! replicas that end up holding the same ops materialize the same tree,
//! whatever order the ops arrived in.
//!
//! Restore resolves its destination from the **log** (`deletion_of`'s
//! fold) and emits an ordinary `Op::Move`, so convergence is the tree
//! CRDT's, not something this action adds. What these cover is the part
//! an action can still get wrong: emitting a *second* op, emitting one
//! whose target depends on local-only state, or producing a pair of
//! concurrent moves the tree cannot order.

mod trash_restore;

use outl_actions::block::{append_block, delete as delete_block};
use outl_actions::page::{open_or_create, PageKind};
use outl_actions::trash;
use outl_actions::tree::children_of;
use outl_core::op::LogOp;
use trash_restore::{assert_converges, branch, log_ids, ops, permutations, replay, replica, shape};

/// One replica restores the block; the other deletes the page it would
/// land on. Both delivery orders, then the full permutation of the
/// whole union.
#[test]
fn a_restore_racing_a_delete_of_its_landing_page_converges() {
    let (mut seed, hlc) = replica();
    let page = open_or_create(&mut seed, &hlc, "p", "P", PageKind::Page).expect("page");
    let block = append_block(&mut seed, &hlc, Some(page), Some("texto")).expect("block");
    delete_block(&mut seed, &hlc, block).expect("delete");
    let prefix = ops(&seed);

    let restored = branch(&prefix, |ws, hlc| {
        trash::restore(ws, hlc, block).expect("restorable from the prefix");
    });
    assert_eq!(restored.len(), 1, "a restore is exactly one op");

    let page_deleted = branch(&prefix, |ws, hlc| {
        delete_block(ws, hlc, page).expect("delete the page");
    });

    let concurrent: Vec<LogOp> = restored
        .iter()
        .chain(page_deleted.iter())
        .cloned()
        .collect();
    let mut union = prefix.clone();
    union.extend(concurrent.clone());

    // Both deliveries of the concurrent pair, with the causal prefix
    // first — the order a real sync produces.
    let mut agreed = None;
    for (i, tail) in permutations(&concurrent).iter().enumerate() {
        let mut order = prefix.clone();
        order.extend(tail.clone());
        let ws = replay(&order);
        match &agreed {
            None => agreed = Some(shape(&ws)),
            Some(first) => assert_eq!(&shape(&ws), first, "delivery order #{i} diverged"),
        }
        assert_eq!(
            log_ids(&ws).len(),
            union.len(),
            "every op stayed in the log (order #{i})"
        );
    }

    // And the whole union permuted, which also delivers ops before
    // their causal predecessors — the `apply_op` undo/replay path.
    assert_converges("restore vs page delete", &union);
}

/// Both replicas restore the **same** trashed node. Two `Move`s to the
/// same parent at different positions: the block has to come back
/// exactly once, in one place.
#[test]
fn two_concurrent_restores_of_one_node_converge_on_a_single_placement() {
    let (mut seed, hlc) = replica();
    let page = open_or_create(&mut seed, &hlc, "p", "P", PageKind::Page).expect("page");
    append_block(&mut seed, &hlc, Some(page), Some("vizinho")).expect("neighbour");
    let block = append_block(&mut seed, &hlc, Some(page), Some("texto")).expect("block");
    delete_block(&mut seed, &hlc, block).expect("delete");
    let prefix = ops(&seed);

    let one = branch(&prefix, |ws, hlc| {
        trash::restore(ws, hlc, block).expect("restorable");
    });
    let two = branch(&prefix, |ws, hlc| {
        trash::restore(ws, hlc, block).expect("restorable");
    });

    let mut union = prefix.clone();
    union.extend(one);
    union.extend(two);

    let agreed = assert_converges("two concurrent restores", &union);
    assert_eq!(
        agreed[&block.to_string()].0,
        page.to_string(),
        "the block is back on the page it was deleted from"
    );

    let ws = replay(&union);
    assert_eq!(
        children_of(&ws, page)
            .iter()
            .filter(|(node, _)| *node == block)
            .count(),
        1,
        "and appears once among its children, not twice"
    );
}
