//! Root `CLAUDE.md` invariant 4 for `trash::restore`: a `Move` that
//! would close a loop is a no-op on the materialized tree **and the op
//! still goes into the log**.
//!
//! Restore re-parents a node to a parent read out of the log, and that
//! parent can have moved *inside* the node since the deletion —
//! `Move(section, block)` is legal while `block` sits in the trash,
//! because the walk up from `block` reaches the trash sentinel and
//! finds no cycle. So the restore's destination is a live cycle risk,
//! not a theoretical one.
//!
//! Two answers, and both are correct for their case: when this replica
//! can see the cycle, `trash::landing_for` refuses up front
//! (`crates/outl-actions/src/trash.rs`, the `is_under` arm); when the
//! two moves were concurrent, nothing could have seen it, and the
//! loser becomes a logged no-op.

mod trash_restore;

use outl_actions::block::{append_block, delete as delete_block, move_under};
use outl_actions::page::{open_or_create, PageKind};
use outl_actions::refusal::Surface;
use outl_actions::trash;
use outl_actions::ActionError;
use outl_core::fractional::Fractional;
use outl_core::id::NodeId;
use outl_core::op::{LogOp, Op};
use trash_restore::{assert_converges, branch, describe, log_ids, ops, replay, replica, shape};

/// No parent chain in `ws` reaches a node twice.
fn assert_acyclic(ws: &outl_core::workspace::Workspace) {
    for (node, _, _) in ws.tree().iter_nodes() {
        let mut seen = vec![node];
        let mut cursor = node;
        while let Some(parent) = ws.tree().parent(cursor) {
            assert!(
                !seen.contains(&parent),
                "cycle through {parent} reachable from {node}"
            );
            seen.push(parent);
            cursor = parent;
            assert!(seen.len() <= 64, "unbounded parent chain from {node}");
        }
    }
}

/// The stored parent moved inside the trashed node. This replica can
/// see it, so the refusal comes before the op is built.
#[test]
fn a_restore_whose_stored_parent_moved_inside_it_is_refused_up_front() {
    let (mut ws, hlc) = replica();
    let page = open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).expect("page");
    let block = append_block(&mut ws, &hlc, Some(page), Some("bloco")).expect("block");
    let section = append_block(&mut ws, &hlc, Some(block), Some("seção")).expect("section");
    move_under(&mut ws, &hlc, section, page).expect("section to the page");
    move_under(&mut ws, &hlc, block, section).expect("block under the section");
    delete_block(&mut ws, &hlc, block).expect("delete");
    assert_eq!(
        trash::parent_at_deletion(&ws, block).expect("foldable"),
        Some(section),
        "precondition: the fold says it came from `section`"
    );

    // Legal while `block` is in the trash: the walk up from `block`
    // hits the trash sentinel, so `creates_cycle` says no.
    move_under(&mut ws, &hlc, section, block).expect("section inside the trashed block");
    assert_eq!(ws.tree().parent(section), Some(block));

    let before_tree = shape(&ws);
    let before_log = log_ids(&ws);
    let err = trash::restore(&mut ws, &hlc, block).expect_err("that landing is a cycle");

    assert_eq!(shape(&ws), before_tree, "nothing moved: {err}");
    assert_eq!(before_log, log_ids(&ws), "and nothing was logged");

    // Its own variant, not `TrashOriginUnknown`. That variant's sentence
    // said the op log could not say where the block was, which is false
    // here: the fold above read `section` straight off the log, and
    // reading it is what produced this refusal.
    assert!(
        matches!(err, ActionError::TrashOriginInsideSubtree { .. }),
        "the log placed it; the destination is what moved: {err:?}"
    );

    let row = |surface| {
        trash::list(&ws, surface)
            .into_iter()
            .find(|entry| entry.node == block)
            .and_then(|entry| entry.refusal)
    };
    assert_eq!(
        row(Surface::Cli).as_deref(),
        Some(outl_actions::refusal::trash_refusal_text(Surface::Cli, &err).as_str()),
        "the listing carries the same refusal rather than offering the restore"
    );

    // **The per-surface half.** "Move that block back out" is a real
    // instruction on the CLI, where `outl block move` accepts a trashed
    // source, and a dead end on an outline: the block that has to move
    // is inside this deletion, so no live view draws it, and invariant 2
    // keeps block ids off those surfaces. A surface with no way out says
    // so instead of printing a to-do the user cannot perform.
    let cli = row(Surface::Cli).expect("the CLI row is refused");
    assert!(
        cli.contains("outl block move"),
        "the CLI can perform the move and should be told how: {cli}"
    );
    for surface in [Surface::Tui, Surface::Desktop, Surface::Mobile] {
        let text = row(surface).expect("still refused");
        assert!(
            !text.contains("outl block move") && !text.contains("move that block"),
            "{surface:?} cannot reach that block and must not be told to move it: {text}"
        );
        assert!(
            text.contains("nothing to do about this one from here"),
            "{surface:?} must say it has no way out rather than going quiet: {text}"
        );
    }
}

/// The concurrent version, which no pre-check can catch: one replica
/// restores, the other re-parents the landing inside the node. Exactly
/// one move wins, the tree stays acyclic, and **both** ops are in every
/// replica's log.
#[test]
fn a_concurrent_restore_and_reparent_leaves_the_loser_in_the_log() {
    let (mut seed, hlc) = replica();
    let page = open_or_create(&mut seed, &hlc, "p", "P", PageKind::Page).expect("page");
    let section = append_block(&mut seed, &hlc, Some(page), Some("seção")).expect("section");
    let block = append_block(&mut seed, &hlc, Some(section), Some("bloco")).expect("block");
    delete_block(&mut seed, &hlc, block).expect("delete");
    let prefix = ops(&seed);

    let restored = branch(&prefix, |ws, hlc| {
        trash::restore(ws, hlc, block).expect("restorable from the prefix");
    });
    let reparented = branch(&prefix, |ws, hlc| {
        move_under(ws, hlc, section, block).expect("legal while the block is trashed");
    });

    let mut union = prefix.clone();
    union.extend(restored.clone());
    union.extend(reparented.clone());

    let agreed = assert_converges("restore vs reparent-into-it", &union);
    let ws = replay(&union);
    assert_acyclic(&ws);

    let block_parent = &agreed[&block.to_string()].0;
    let section_parent = &agreed[&section.to_string()].0;
    let restore_won = block_parent == &section.to_string();
    assert_eq!(
        restore_won,
        section_parent == &page.to_string(),
        "exactly one of the two moves applied: block->{block_parent}, section->{section_parent}"
    );

    let present = log_ids(&ws);
    for op in restored.iter().chain(reparented.iter()) {
        assert!(
            present.contains(&op.ts),
            "invariant 5: {} fell out of the log",
            describe(op)
        );
    }
}

/// The bare pairing, asked of `Workspace::apply` directly so the claim
/// is not inferred from a count: a `Move` the tree refuses is absent
/// from the tree and present in the log.
///
/// Reachable without a race — `outl-plugins`' host calls
/// `block::move_under` with no cycle pre-check, and a reordered peer op
/// arrives the same way.
#[test]
fn a_move_the_tree_refuses_is_still_a_line_in_the_log() {
    let (mut ws, hlc) = replica();
    let page = open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).expect("page");
    let outer = append_block(&mut ws, &hlc, Some(page), Some("fora")).expect("outer");
    let inner = append_block(&mut ws, &hlc, Some(outer), Some("dentro")).expect("inner");

    let before_parent = ws.tree().parent(outer);
    let ts = hlc.next();
    ws.apply(LogOp {
        ts,
        actor: ts.actor,
        op: Op::Move {
            node: outer,
            new_parent: inner,
            position: Fractional::first(),
            old_parent: page,
            old_position: Fractional::first(),
        },
    })
    .expect("apply");

    assert_eq!(
        ws.tree().parent(outer),
        before_parent,
        "invariant 4: the tree refused the cycle"
    );
    assert!(
        log_ids(&ws).contains(&ts),
        "invariant 5: the refused op stayed in the log"
    );
    assert_ne!(before_parent, Some(NodeId::trash()), "sanity");
}
