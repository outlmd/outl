//! A backlink row outlives the block it names, in two ways the panel
//! must refuse instead of re-aiming:
//!
//! - **the node left the row's subtree** (a peer deleted or moved it,
//!   and the index was rebuilt): the cursor still carries the node id,
//!   and resolving it to the *citing block* would run `d d` / `Tab` on
//!   the parent and its whole subtree;
//! - **the block is live but on another page now** (a peer moved it
//!   and the index rebuild has not landed): `commit_page` scopes the
//!   undo snapshot and the projection to the row's page, so the op
//!   would land on a page whose `.md` is never rewritten.
//!
//! Both are asserted on the op log, never on the message alone.

use crate::actions::block::test_support::workspace_with_two_backlinks;
use crate::actions::block::InsertCursor;
use crate::actions::node_id;
use crate::state::{App, Focus, Mode};
use outl_core::id::NodeId;

fn row_with_a_child(app: &App) -> (usize, NodeId, NodeId) {
    app.backlinks_for_current()
        .iter()
        .enumerate()
        .find_map(|(idx, bl)| {
            let sub = app.backlink_subtree(&bl.block_id)?;
            Some((idx, node_id(&bl.block_id)?, node_id(&sub.first()?.id)?))
        })
        .expect("setup: one citing block has a child")
}

/// Focus on the child, then rewrite the source `.md` without it and
/// re-read the page: the index no longer holds the child but the focus
/// (which a refetch does not clamp) still names it.
fn focus_on_a_child_the_index_lost(app: &mut App, dir: &std::path::Path) -> (NodeId, NodeId) {
    let (row, parent, child) = row_with_a_child(app);
    app.focus = Focus::Backlink {
        idx: row,
        node: Some(child),
    };
    std::fs::write(
        dir.join("pages").join("source.md"),
        "- mentions [[target]]\n- second [[target]] mention\n",
    )
    .unwrap();
    app.reindex_backlinks_for_slug("source");
    assert_eq!(
        app.focus,
        Focus::Backlink {
            idx: row,
            node: Some(child)
        },
        "setup: the refetch leaves the node in the focus"
    );
    assert!(
        app.backlink_subtree(&app.backlinks_for_current()[row].block_id)
            .is_none(),
        "setup: the rebuilt index no longer holds the child"
    );
    (parent, child)
}

#[test]
fn a_node_the_index_lost_resolves_to_no_target_not_to_the_citing_block() {
    let (mut app, d) = workspace_with_two_backlinks();
    focus_on_a_child_the_index_lost(&mut app, d.path());

    assert_eq!(app.focused_backlink_target(), None);
}

#[test]
fn delete_on_a_node_the_index_lost_leaves_the_citing_block_alone() {
    let (mut app, d) = workspace_with_two_backlinks();
    let (parent, child) = focus_on_a_child_the_index_lost(&mut app, d.path());
    let ops = app.workspace.log().len();

    app.delete_current();

    assert_eq!(app.workspace.log().len(), ops, "no op was minted");
    assert!(outl_actions::is_live(&app.workspace, parent));
    assert!(outl_actions::is_live(&app.workspace, child));
    assert_eq!(
        app.status,
        outl_actions::refusal::stale_backlink_block_unchanged_refusal(&app.workspace, child),
        "the catalog's sentence, not the generic one"
    );
}

#[test]
fn indent_on_a_node_the_index_lost_leaves_the_citing_block_alone() {
    let (mut app, d) = workspace_with_two_backlinks();
    let (parent, _) = focus_on_a_child_the_index_lost(&mut app, d.path());
    let ops = app.workspace.log().len();
    let parent_of_parent = app.workspace.tree().parent(parent);

    app.indent_current();

    assert_eq!(app.workspace.log().len(), ops, "no op was minted");
    assert_eq!(app.workspace.tree().parent(parent), parent_of_parent);
}

#[test]
fn entering_insert_on_a_node_the_index_lost_says_so() {
    let (mut app, d) = workspace_with_two_backlinks();
    focus_on_a_child_the_index_lost(&mut app, d.path());
    app.status.clear();

    app.enter_insert(InsertCursor::Start);

    assert!(matches!(app.mode, Mode::Normal));
    assert!(!app.status.is_empty(), "a refusal is said out loud");
}

#[test]
fn a_landed_index_pulls_the_focus_back_to_the_citing_block() {
    let (mut app, d) = workspace_with_two_backlinks();
    let (row, _, child) = row_with_a_child(&app);
    app.focus = Focus::Backlink {
        idx: row,
        node: Some(child),
    };
    std::fs::write(
        d.path().join("pages").join("source.md"),
        "- mentions [[target]]\n- second [[target]] mention\n",
    )
    .unwrap();
    let metas: Vec<_> = ["target", "source"]
        .iter()
        .filter_map(|s| outl_actions::find_by_slug(&app.workspace, s))
        .filter_map(|id| outl_actions::page_meta(&app.workspace, id))
        .collect();
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(outl_actions::build_backlink_index_from_disk(
        &metas,
        d.path(),
    ))
    .unwrap();
    app.backlink_index_rx = Some(rx);

    assert!(app.poll_backlink_index_updates());

    assert_eq!(
        app.focus,
        Focus::Backlink {
            idx: row,
            node: None
        }
    );
}

/// A peer moved the child to the open page; the row (and the index
/// behind it) still says it lives on `source`.
fn a_peer_moved_the_child_to_another_page(app: &mut App) -> (NodeId, NodeId) {
    let (row, _, child) = row_with_a_child(app);
    app.focus = Focus::Backlink {
        idx: row,
        node: Some(child),
    };
    let elsewhere = outl_actions::find_by_slug(&app.workspace, "target").expect("target page");
    let hlc = app.hlc.clone();
    outl_actions::move_under(&mut app.workspace, &hlc, child, elsewhere).expect("move");
    assert!(outl_actions::is_live(&app.workspace, child), "setup: live");
    (child, elsewhere)
}

#[test]
fn a_chord_on_a_row_whose_block_moved_to_another_page_is_refused() {
    let (mut app, _d) = workspace_with_two_backlinks();
    let (child, elsewhere) = a_peer_moved_the_child_to_another_page(&mut app);
    let ops = app.workspace.log().len();

    app.delete_current();

    assert_eq!(app.workspace.log().len(), ops, "no op was minted");
    assert_eq!(app.workspace.tree().parent(child), Some(elsewhere));
    assert_eq!(
        app.status,
        outl_actions::refusal::stale_backlink_block_unchanged_refusal(&app.workspace, child)
    );
}

#[test]
fn entering_insert_on_a_row_whose_block_moved_to_another_page_is_refused() {
    let (mut app, _d) = workspace_with_two_backlinks();
    a_peer_moved_the_child_to_another_page(&mut app);

    app.enter_insert(InsertCursor::Start);

    assert!(
        matches!(app.mode, Mode::Normal),
        "Insert must not start on a block the row's page no longer holds, got {:?}",
        app.mode
    );
}
