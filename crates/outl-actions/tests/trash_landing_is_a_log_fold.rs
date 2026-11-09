//! Where a restore lands is a **pure fold over the op log**, so two
//! devices holding the same ops compute the same destination.
//!
//! This is the property that makes restore safe to expose on every
//! client at once. If the landing read live tree state that the log
//! does not determine — a cached index, a `.md` on disk, the order ops
//! happened to arrive in — two devices would emit `Move`s to different
//! parents for the same user action, and the CRDT would faithfully
//! converge on whichever one was later. The tree would agree and the
//! block would be in the wrong place.
//!
//! `deletion_of` (`crates/outl-actions/src/trash.rs`) folds
//! `Create.parent` / `Move.new_parent` through `trash/replay.rs`, which
//! re-decides each placement with `creates_cycle` against the tree *as
//! it stood at that op's HLC*. The guards layered on top (`is_under`,
//! `is_trashed`, `contains`) read the materialized tree — which is
//! itself a function of the same log, so they do not weaken this.

mod trash_restore;

use outl_actions::block::{append_block, delete as delete_block, move_under};
use outl_actions::page::{open_or_create, PageKind};
use outl_actions::refusal::Surface;
use outl_actions::trash;
use outl_core::fractional::Fractional;
use outl_core::id::NodeId;
use outl_core::op::{LogOp, Op};
use outl_core::workspace::Workspace;
use trash_restore::{ops, orders, replay, replica};

/// Every trash row's landing parent and refusal, sorted — the whole
/// verdict surface a client reads, in a comparable shape.
fn verdicts(ws: &Workspace) -> Vec<(String, Option<String>, Option<String>)> {
    let mut rows: Vec<_> = trash::list(ws, Surface::Cli)
        .into_iter()
        .map(|entry| {
            (
                entry.node.to_string(),
                entry.parent_at_deletion.map(|p| p.to_string()),
                entry.refusal,
            )
        })
        .collect();
    rows.sort();
    rows
}

/// A history with every shape the fold has to tell apart: a block that
/// changed parents before it was deleted, a block whose parent was
/// deleted after it, and the parent itself.
#[test]
fn the_landing_is_the_same_whatever_order_the_log_arrived_in() {
    let (mut ws, hlc) = replica();
    let page = open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).expect("page");
    let section = append_block(&mut ws, &hlc, Some(page), Some("seção")).expect("section");
    let moved = append_block(&mut ws, &hlc, Some(page), Some("mudou de pai")).expect("moved");
    move_under(&mut ws, &hlc, moved, section).expect("re-parent before deleting");
    delete_block(&mut ws, &hlc, moved).expect("delete the moved block");
    let orphan = append_block(&mut ws, &hlc, Some(section), Some("órfão")).expect("orphan");
    delete_block(&mut ws, &hlc, orphan).expect("delete the orphan");
    delete_block(&mut ws, &hlc, section).expect("delete the section");
    let log = ops(&ws);

    let baseline = replay(&log);
    let expected = verdicts(&baseline);
    assert_eq!(expected.len(), 3, "three top-level deletions");
    assert!(
        expected.iter().any(|(_, _, refusal)| refusal.is_some())
            && expected.iter().any(|(_, _, refusal)| refusal.is_none()),
        "the rows have to disagree about something, or this proves nothing: {expected:#?}"
    );

    for (i, order) in orders(&log).iter().enumerate() {
        let ws = replay(order);
        assert_eq!(
            verdicts(&ws),
            expected,
            "delivery order #{i} computed a different landing or refusal"
        );
        for node in trash::list(&ws, Surface::Cli)
            .iter()
            .map(|entry| entry.node)
        {
            assert_eq!(
                trash::parent_at_deletion(&ws, node).expect("foldable"),
                trash::parent_at_deletion(&baseline, node).expect("foldable"),
                "delivery order #{i} folded a different origin for {node}"
            );
        }
    }
}

/// The landing comes off `Move.new_parent`, never off `old_parent`.
///
/// That field is the originating replica's local derivation for
/// `undo_op`, and 65,141 of 65,703 stored `Move`s on the reference
/// workspace name `root` regardless of where the block actually was.
/// An append-only log never rewrites them, so a reader that trusted it
/// would work on this week's history and lie about every year before
/// it — and the lie it tells is `root()`, the orphan destination
/// `trash_restore_refuses_rather_than_orphans.rs` exists to refuse.
#[test]
fn a_lying_old_parent_does_not_move_the_landing() {
    let (mut ws, hlc) = replica();
    let page = open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).expect("page");
    let section = append_block(&mut ws, &hlc, Some(page), Some("seção")).expect("section");
    let block = append_block(&mut ws, &hlc, Some(section), Some("bloco")).expect("block");

    // The shape every historical op on disk has: `old_parent = root`.
    let ts = hlc.next();
    ws.apply(LogOp {
        ts,
        actor: ts.actor,
        op: Op::Move {
            node: block,
            new_parent: NodeId::trash(),
            position: Fractional::first(),
            old_parent: NodeId::root(),
            old_position: Fractional::first(),
        },
    })
    .expect("apply");

    assert_eq!(
        trash::parent_at_deletion(&ws, block).expect("foldable"),
        Some(section),
        "the fold must ignore the lying `old_parent`"
    );
    trash::restore(&mut ws, &hlc, block).expect("restore");
    assert_eq!(
        ws.tree().parent(block),
        Some(section),
        "and the block goes back to where the log says it was"
    );
}
