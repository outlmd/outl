//! Which deletions belong to which page.
//!
//! Split out of `mod.rs` because it is one question with ten answers,
//! and because the fold it exercises stopped being local: the parent
//! trail now belongs to [`crate::trash::parent_at_deletion`], which
//! `came_from` calls. These tests are what keeps the two surfaces —
//! a page's history and `outl trash list` — agreeing about where a
//! block lived when it went.

use outl_core::fractional::Fractional;
use outl_core::id::NodeId;
use outl_core::op::Op;

use super::*;

/// The reason this module exists. A block the user deleted is exactly
/// what they open a history to look for, and it is no longer in the
/// page's subtree — so the naive "walk the page" scan misses it.
#[test]
fn a_deleted_block_stays_in_the_page_history() {
    let mut fixture = Fixture::new();
    let block = fixture.block(fixture.page, "the paragraph I lost");
    fixture.delete(block);
    let timeline = fixture.timeline();

    assert!(timeline
        .events
        .iter()
        .any(|e| e.node == block && matches!(e.change, Change::Deleted { .. })));
    assert!(timeline
        .events
        .iter()
        .filter(|e| e.node == block)
        .all(|e| e.node_deleted));
}

/// Naming the deletion is not enough — the text it took has to come back
/// with it, or the history says "something was here" and stops.
#[test]
fn the_text_a_deletion_took_is_in_the_event() {
    let mut fixture = Fixture::new();
    let block = fixture.block(fixture.page, "the paragraph I lost");
    fixture.delete(block);

    let timeline = fixture.timeline();
    let deleted = timeline
        .events
        .iter()
        .find_map(|e| match &e.change {
            Change::Deleted { text } => Some(text.clone()),
            _ => None,
        })
        .expect("a Deleted event");
    assert_eq!(deleted, "the paragraph I lost");
}

/// Deleting a parent trashes the subtree in one `Move`, so the children
/// are only reachable by walking down from the trashed root.
#[test]
fn deleting_a_parent_keeps_its_children_in_the_history() {
    let mut fixture = Fixture::new();
    let parent = fixture.block(fixture.page, "parent");
    let child = fixture.block(parent, "child");
    fixture.delete(parent);

    let timeline = fixture.timeline();
    assert!(
        timeline.events.iter().any(|e| e.node == child),
        "the child's history went to the trash with its parent"
    );
}

/// The trash scan has to reach a fixpoint. Delete a child and *then* its
/// parent and the child becomes its own direct child of the trash, whose
/// parent-at-deletion is a block no longer in the live subtree — one pass
/// drops it along with the text it took.
#[test]
fn a_child_deleted_before_its_parent_stays_in_the_history() {
    let mut fixture = Fixture::new();
    let parent = fixture.block(fixture.page, "parent");
    let child = fixture.block(parent, "the child I deleted first");
    fixture.delete(child);
    fixture.delete(parent);

    let timeline = fixture.timeline();
    assert!(
        timeline.events.iter().any(|e| e.node == child
            && matches!(&e.change, Change::Deleted { text } if text == "the child I deleted first")),
        "the child's deletion vanished because the parent left the page after it"
    );
}

/// Deleting a subtree is one `Move` on its root, so every block inside it
/// keeps pointing at that root. A direct-parent test answers `false` for
/// all of them and the client renders a live block that isn't there.
#[test]
fn every_block_inside_a_deleted_subtree_is_flagged_deleted() {
    let mut fixture = Fixture::new();
    let parent = fixture.block(fixture.page, "parent");
    let child = fixture.block(parent, "child");
    let grandchild = fixture.block(child, "grandchild");
    fixture.delete(parent);

    let timeline = fixture.timeline();
    for node in [parent, child, grandchild] {
        let rows: Vec<_> = timeline.events.iter().filter(|e| e.node == node).collect();
        assert!(!rows.is_empty(), "no events for {node}");
        assert!(
            rows.iter().all(|e| e.node_deleted),
            "{node} not flagged deleted"
        );
    }
}

/// `do_op`'s `Create` is idempotent and keeps the node's current parent,
/// so a stale reconcile re-emitting `Create{parent: old_page}` must not
/// move the accumulator — otherwise the old page claims a deletion that
/// happened on the new one.
#[test]
fn a_stale_create_does_not_let_the_old_page_claim_the_deletion() {
    let mut fixture = Fixture::new();
    let other_page = fixture.page();
    let block = fixture.block(fixture.page, "moved away");
    fixture.move_to(block, other_page);
    // The stale device's reconcile, naming the page the block left.
    let stale_parent = fixture.page;
    apply(
        &mut fixture.ws,
        &fixture.hlc,
        Op::Create {
            node: block,
            parent: stale_parent,
            position: Fractional::first(),
        },
    );
    fixture.delete(block);

    assert!(
        fixture.timeline().events.iter().all(|e| e.node != block),
        "this page claimed a deletion that happened on another page"
    );
}

/// Another page's deletions are not this page's history. Over-including
/// is worse than a gap: a gap is visibly a gap.
#[test]
fn a_block_deleted_from_another_page_is_not_in_this_ones_history() {
    let mut fixture = Fixture::new();
    let other_page = fixture.page();
    let theirs = fixture.block(other_page, "not mine");
    fixture.delete(theirs);

    let timeline = fixture.timeline();
    assert!(timeline.events.iter().all(|e| e.node != theirs));
}

#[test]
fn coming_back_out_of_the_trash_is_its_own_event() {
    let mut fixture = Fixture::new();
    let block = fixture.block(fixture.page, "oops");
    fixture.delete(block);
    apply(
        &mut fixture.ws,
        &fixture.hlc,
        Op::Move {
            node: block,
            new_parent: fixture.page,
            position: Fractional::first(),
            old_parent: NodeId::trash(),
            old_position: Fractional::first(),
        },
    );

    let timeline = fixture.timeline();
    assert!(timeline.events.iter().any(|e| e.change == Change::Restored));
    // Back in the page, so no longer flagged gone.
    assert!(timeline
        .events
        .iter()
        .filter(|e| e.node == block)
        .all(|e| !e.node_deleted));
}

/// `Move.old_parent` is filled by `do_op` on the copy that reaches the
/// in-memory log, while `Workspace::apply` persists the caller's
/// original — 99% of the Move ops in the reference workspace say `root`
/// no matter where the block was. Attribution must survive that, so the
/// parent trail is folded from `new_parent` instead.
#[test]
fn a_deletion_is_attributed_even_when_old_parent_is_wrong() {
    let mut fixture = Fixture::new();
    let block = fixture.block(fixture.page, "deleted with a lying op");
    apply(
        &mut fixture.ws,
        &fixture.hlc,
        Op::Move {
            node: block,
            new_parent: NodeId::trash(),
            position: Fractional::first(),
            // What the reconcile path actually writes.
            old_parent: NodeId::root(),
            old_position: Fractional::first(),
        },
    );

    let timeline = fixture.timeline();
    assert!(
        timeline.events.iter().any(|e| e.node == block
            && matches!(&e.change, Change::Deleted { text } if text == "deleted with a lying op")),
        "the deletion was dropped because the op misreported where the block came from"
    );
}

/// A deletion belongs to the page the block left **last**, not the
/// first page it was ever deleted from.
///
/// This stopped being hypothetical when `trash::restore` shipped:
/// delete → restore → move elsewhere → delete is now a sequence a user
/// can produce, and the old fold answered `true` for the first page to
/// match, so that page kept claiming a block it no longer held.
#[test]
fn a_deletion_is_attributed_to_the_page_the_block_left_last() {
    let mut fixture = Fixture::new();
    let elsewhere = fixture.page();
    let block = fixture.block(fixture.page, "wandering");

    fixture.delete(block);
    fixture.move_to(block, fixture.page); // restored
    fixture.move_to(block, elsewhere);
    fixture.delete(block);

    let timeline = fixture.timeline();
    assert!(
        !timeline.events.iter().any(|e| e.node == block),
        "the first page must not claim a block that was deleted from another page"
    );
}
