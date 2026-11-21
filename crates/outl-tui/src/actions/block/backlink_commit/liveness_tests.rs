//! A chord in the backlinks panel, aimed at a block that is no longer
//! live.
//!
//! Its own module for the reason `ahead_of_log_tests` is: every test in
//! `tests.rs` asks *what a cross-page commit does*, and these ask
//! *whether it is allowed to happen at all*.
//!
//! **The state is ordinary and reachable.** The backlink index is built
//! from disk, so a row outlives a peer's delete of the block it names —
//! the source `.md` has not changed, so the row keeps rendering. What
//! made that dangerous is that `outl_actions::block::moves` guards with
//! `ensure_in_tree`, **not** `ensure_live`, and says why in place:
//! `trash::restore` is itself a move, so a trashed node has to be a
//! legal source there. `block/edit.rs` asks `ensure_live` at all three
//! of its entries; the four structural moves ask nobody.
//!
//! And this crate's own boundary predicates could not fill the gap,
//! because they ask the tree about **position**: for a trashed node the
//! parent is `TRASH_ROOT`, a node the tree keeps, and `children_of` is a
//! plain `iter_nodes` filter — so `children_of(NodeId::trash())` is
//! every trashed node in the workspace. `Tab` found a previous sibling
//! there and re-parented the block under an unrelated deletion, out of
//! `trash::list`, which enumerates only the trash root's **direct**
//! children. A block the user cannot restore.
//!
//! So the assertions here are **structural** — the op log's length, and
//! where the block sits in the tree. Three of the four chords said
//! nothing at all in the broken state, so a test that read `app.status`
//! alone would have gone green over a block the user had just lost.
//!
//! **One member is message-shaped, and saying "structural" of all of
//! them was a false census.**
//! `outdenting_a_row_whose_block_a_peer_deleted_says_the_catalogs_sentence`
//! drives a block sitting *directly* under the trash root, where
//! `outl_actions::outdent` fails with `NoGrandparent` before minting
//! anything — so its `log().len()` line passes in the broken state too
//! and the defect it covers is genuinely the wording (`NoGrandparent`'s
//! `Display` names a `NodeId`). Both of its assertions are
//! reword-proof, so the member is sound; what was not sound was
//! counting it as cover for the write.
//! `outdenting_a_nested_deleted_row_leaves_it_inside_the_deletion_it_came_with`
//! is that cover: one level down the same keystroke *does* write, so
//! the gesture has a structural net as well as a message one.

use crate::actions::block::backlink_target::BacklinkTarget;
use crate::actions::block::test_support::{
    workspace_with_a_backlink, workspace_with_a_self_backlink,
};
use crate::state::{App, Focus, HistorySnapshot, Overlay};
use outl_core::id::NodeId;

/// What [`a_peer_deleted_the_focused_block`] left behind.
struct TrashedRow {
    /// The row's page and block, resolved before the delete.
    target: BacklinkTarget,
    /// The deletion sitting immediately before it under the trash root.
    before: NodeId,
}

/// The trash root's direct children, in `children_of`'s canonical
/// sibling order — which is the order every boundary predicate here
/// reads for a trashed node.
fn trash_order(app: &App) -> Vec<NodeId> {
    outl_actions::children_of(&app.workspace, NodeId::trash())
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

/// A peer deleted the block behind the focused row — and the trash
/// holds a deletion on **either side** of it.
///
/// Both halves are load-bearing. The delete is what the row outlives.
/// The two neighbours are what make the position-only boundary
/// questions answer *yes* about a trashed node, which is the only state
/// in which `Tab`, `Alt+↑` and `Alt+↓` reach their commit at all: a
/// trashed block that happens to sort first in the trash is declined by
/// `has_previous_sibling` on its own, and a test set up that way passes
/// without ever touching the gate.
///
/// The order is pinned with `move_after` rather than left to `delete`,
/// which places every node at `Fractional::first()` — so `sort_siblings`
/// settles the trash by `NodeId` and the neighbour relation would be a
/// coin flip per run. A flaky precondition reads as a flaky fix.
fn a_peer_deleted_the_focused_block(app: &mut App) -> TrashedRow {
    let target = app
        .focused_backlink_target()
        .expect("a healthy backlink row resolves");
    let hlc = app.hlc.clone();
    let before =
        outl_actions::append_block(&mut app.workspace, &hlc, None, Some("deleted earlier"))
            .expect("a neighbour for the trash");
    let after = outl_actions::append_block(&mut app.workspace, &hlc, None, Some("deleted later"))
        .expect("a second neighbour for the trash");
    for node in [before, target.block, after] {
        outl_actions::delete(&mut app.workspace, &hlc, node).expect("trash it");
    }
    outl_actions::move_after(&mut app.workspace, &hlc, target.block, before)
        .expect("pin the citing block after the first neighbour");
    outl_actions::move_after(&mut app.workspace, &hlc, after, target.block)
        .expect("pin the second neighbour after the citing block");

    let order = trash_order(app);
    let at = order
        .iter()
        .position(|id| *id == target.block)
        .expect("setup: the citing block is in the trash");
    assert!(
        at > 0 && at + 1 < order.len(),
        "setup: the trashed citing block needs a deletion on each side, or \
         the boundary predicates decline on their own and nothing here \
         reaches the gate"
    );
    assert!(
        !app.backlinks_for_current().is_empty(),
        "setup: the row has to outlive the delete — the index is built from \
         disk and the source `.md` has not changed"
    );
    TrashedRow { target, before }
}

/// **`Tab` could put a block somewhere the trash UI cannot list.**
///
/// `has_previous_sibling` answered yes about the trash root's children,
/// so `outl_actions::indent` took an unrelated deletion as the new
/// parent and emitted `Op::Move`. The block stopped being a direct child
/// of `TRASH_ROOT`, and `trash::list` enumerates only those — so the
/// only route back was to restore a block the user has no way of
/// knowing about.
///
/// Asserted on the tree and the log, never on the message: the broken
/// state wrote no status line at all, so a message-only test would have
/// passed over the loss.
#[test]
fn indenting_a_row_whose_block_a_peer_deleted_leaves_it_a_direct_child_of_the_trash() {
    let (mut app, _d, _source) = workspace_with_a_backlink();
    let row = a_peer_deleted_the_focused_block(&mut app);
    let ops_before = app.workspace.log().len();
    let order_before = trash_order(&app);

    let moved = app.indent_current();

    assert!(!moved, "the indent reported a move on a deleted block");
    assert_eq!(
        app.workspace.tree().parent(row.target.block),
        Some(NodeId::trash()),
        "the block stopped being a direct child of the trash root, so \
         `trash::list` cannot list it and nothing can restore it"
    );
    assert!(
        outl_actions::children_of(&app.workspace, row.before).is_empty(),
        "an unrelated deletion adopted the block"
    );
    assert_eq!(
        app.workspace.log().len(),
        ops_before,
        "an `Op::Move` was written for a block the user cannot see"
    );
    assert_eq!(
        trash_order(&app),
        order_before,
        "the trash was reordered by a refused chord"
    );
}

/// **`Shift+Tab` printed a raw block ULID.**
///
/// `can_outdent` answers yes for a trashed node — its parent is the
/// trash root, which is neither the page nor `root()` — and
/// `outl_actions::outdent` then asks for the trash root's *parent*,
/// which is `None` because that sentinel is implicit. That is
/// `NoGrandparent`, not `NotInTree`, so it fell through to the generic
/// `backlink edit failed: {e}` arm and put a 26-character id on the
/// status line, two lines below the comment saying no outl surface shows
/// a user a block id.
///
/// Compared against the catalog's own output rather than a literal: a
/// literal here is the second owner of the sentence that
/// `outl_actions::refusal` exists to prevent (invariant 12).
#[test]
fn outdenting_a_row_whose_block_a_peer_deleted_says_the_catalogs_sentence() {
    let (mut app, _d, _source) = workspace_with_a_backlink();
    let row = a_peer_deleted_the_focused_block(&mut app);
    let ops_before = app.workspace.log().len();

    let moved = app.outdent_current();

    assert!(!moved, "an outdent on a deleted block cannot move it");
    assert_eq!(
        app.status,
        outl_actions::refusal::stale_backlink_block_unchanged_refusal(
            &app.workspace,
            row.target.block
        ),
        "the wording belongs to the catalog, never to this client"
    );
    assert!(
        !app.status.contains(&row.target.block.to_string()),
        "a block id reached the status line: {}",
        app.status
    );
    assert_eq!(app.workspace.log().len(), ops_before);
}

/// **`Shift+Tab`'s structural half, which the member above cannot
/// reach.**
///
/// On a block sitting *directly* under the trash root the broken state
/// is message-only: `outl_actions::outdent` asks for the trash root's
/// parent, which is `None`, so it fails with `NoGrandparent` before
/// minting anything. The op-count assertion up there is therefore
/// inert, and the module doc used to count it as structural cover.
///
/// Nest the trashed block one level and the same keystroke writes. A
/// peer deleting a *parent* leaves its children attached to it, so the
/// citing block is trashed with a trashed parent and a grandparent that
/// exists (`TRASH_ROOT`) — `can_outdent` says yes, `outdent` emits
/// `Op::Move`, and one trash entry becomes two with the user's block
/// detached from the deletion it belonged to.
#[test]
fn outdenting_a_nested_deleted_row_leaves_it_inside_the_deletion_it_came_with() {
    let (mut app, _d, _source) = workspace_with_a_backlink();
    let target = app
        .focused_backlink_target()
        .expect("a healthy backlink row resolves");
    let hlc = app.hlc.clone();
    // A previous sibling to indent into, so the citing block has a
    // parent that is not the page — then the peer deletes that parent.
    let parent =
        outl_actions::create_before(&mut app.workspace, &hlc, target.block, Some("branch"))
            .expect("a sibling above the citing block");
    outl_actions::indent(&mut app.workspace, &hlc, target.block).expect("nest the citing block");
    outl_actions::delete(&mut app.workspace, &hlc, parent).expect("the peer deletes the branch");
    assert_eq!(
        app.workspace.tree().parent(target.block),
        Some(parent),
        "setup: the child stays attached to the deleted parent"
    );
    let ops_before = app.workspace.log().len();

    let moved = app.outdent_current();

    assert!(!moved, "an outdent on a deleted block cannot move it");
    assert_eq!(
        app.workspace.tree().parent(target.block),
        Some(parent),
        "the block was lifted out of the deletion it was removed with, so \
         the trash now lists it on its own"
    );
    assert_eq!(
        app.workspace.log().len(),
        ops_before,
        "an `Op::Move` was written inside the trash for a keystroke that \
         should have done nothing"
    );
}

/// **`d d` reported success and cleared the user's redo stack.**
///
/// `delete` is `move_to(node, TRASH_ROOT, Fractional::first())`, which
/// succeeds on a node already sitting there — so the commit returned
/// `true`, pushed an undo snapshot for the source page, and
/// `snapshot_page_for_undo` cleared `redo` **unconditionally**. Three
/// lies in one keystroke: the success, the lost redo, and the op.
#[test]
fn deleting_a_row_whose_block_a_peer_deleted_writes_no_op_and_keeps_both_stacks() {
    let (mut app, _d, source) = workspace_with_a_backlink();
    let _row = a_peer_deleted_the_focused_block(&mut app);
    app.undo.clear();
    app.redo.push(HistorySnapshot {
        page: Default::default(),
        selected: 0,
        cursor_col: 0,
        view_path: source,
    });
    let ops_before = app.workspace.log().len();

    app.delete_current();

    assert_eq!(
        app.workspace.log().len(),
        ops_before,
        "a second delete of an already-deleted block reached the op log"
    );
    assert!(
        app.undo.is_empty(),
        "a chord that changed nothing recorded history for the source page"
    );
    assert_eq!(
        app.redo.len(),
        1,
        "the user's redo stack was cleared by a keystroke that changed nothing"
    );
}

/// **`Alt+↑` / `Alt+↓` reordered unrelated deletions.**
///
/// `has_previous_sibling` / `has_next_sibling` answer about the trash
/// root's children for a trashed node, so `swap_positions` emitted a
/// **pair** of `Op::Move`s over two blocks the user never touched,
/// reported success, and changed nothing visible.
#[test]
fn moving_a_row_whose_block_a_peer_deleted_reorders_no_deletions() {
    for (chord, run) in [
        ("Alt+↑", App::move_block_up as fn(&mut App)),
        ("Alt+↓", App::move_block_down as fn(&mut App)),
    ] {
        let (mut app, _d, _source) = workspace_with_a_backlink();
        let _row = a_peer_deleted_the_focused_block(&mut app);
        let ops_before = app.workspace.log().len();
        let order_before = trash_order(&app);

        run(&mut app);

        assert_eq!(
            app.workspace.log().len(),
            ops_before,
            "{chord}: ops were written for blocks the user never touched"
        );
        assert_eq!(
            trash_order(&app),
            order_before,
            "{chord}: the trash was reordered"
        );
    }
}

// ── The three mirrors ────────────────────────────────────────────────
//
// A liveness gate is as wrong when it is too broad as when it is
// absent, and the broad failure is the one that reads as "the panel
// stopped working". These three name the cases the gate must leave
// alone. They pass before the fix as well as after — that is their
// job — so each was checked by placing the gate where it *would* break
// them and watching it fail.

/// **Mirror one: a backlink row whose block is live.**
///
/// The gate is a question about the target, asked once per commit, so
/// the ordinary case must be untouched. Both directions of the chord
/// set are covered because they fail differently: `indent` refuses with
/// an `Err` and `delete` succeeds, so a gate that leaked into either
/// one would show up here and nowhere else in this file.
#[test]
fn a_live_backlink_row_still_takes_its_chords() {
    let (mut app, _d, _open) = workspace_with_a_self_backlink();
    let target = app
        .focused_backlink_target()
        .expect("the self-citation resolves");
    let parent_before = app.workspace.tree().parent(target.block);

    assert!(
        app.indent_current(),
        "an indent on a live citing block has to land, got status {:?}",
        app.status
    );
    assert_ne!(
        app.workspace.tree().parent(target.block),
        parent_before,
        "the indent never reached the op log"
    );

    let (mut app, _d, _source) = workspace_with_a_backlink();
    let target = app
        .focused_backlink_target()
        .expect("a healthy backlink row resolves");

    app.delete_current();

    assert!(
        outl_actions::is_trashed(&app.workspace, target.block),
        "a `d d` on a live citing block has to trash it, got status {:?}",
        app.status
    );
}

/// **Mirror two: restoring a block from the trash.**
///
/// `trash::restore` is implemented as a move and legitimately acts on a
/// trashed node — `moves.rs` carries `ensure_in_tree` rather than
/// `ensure_live` for exactly that reason. So a liveness gate placed any
/// lower (in `move_under`, in `move_to`, or in the boundary predicates
/// the trash overlay also reads) breaks the one gesture that undoes a
/// delete.
///
/// Driven through the overlay rather than the backend, because the
/// backend call is outside this crate: the question is whether *this
/// client's* restore path still reaches it.
#[test]
fn restoring_a_block_from_the_trash_still_puts_it_back() {
    let (mut app, _d, _source) = workspace_with_a_backlink();
    let row = a_peer_deleted_the_focused_block(&mut app);

    app.open_trash();
    let at = match &app.overlay {
        Some(Overlay::Trash(t)) => t
            .entries
            .iter()
            .position(|e| e.node == row.target.block)
            .expect("the deleted citing block is a trash row"),
        _ => panic!("the overlay is open"),
    };
    if let Some(Overlay::Trash(t)) = app.overlay.as_mut() {
        t.selected = at;
    }

    app.restore_selected_trash();

    assert!(
        !outl_actions::is_trashed(&app.workspace, row.target.block),
        "`r` in the trash left the block deleted; message was {:?}",
        match &app.overlay {
            Some(Overlay::Trash(t)) => t.message.clone(),
            _ => None,
        }
    );
}

/// **Mirror three: an ordinary outline indent.**
///
/// The outline path never reaches `outl_actions::indent` at all — it
/// manipulates the in-flight AST through `outl_md::outline_ops`, and
/// the ops are derived later from the rendered `.md`. So the gate is
/// not on it, and this is what says so out loud rather than leaving the
/// next reader to re-derive it from `structural.rs`'s two branches.
#[test]
fn an_ordinary_outline_indent_still_nests_the_block() {
    let (mut app, _d, _open) = workspace_with_a_self_backlink();
    app.focus = Focus::Outline;
    app.selected = 1;
    assert_eq!(
        app.page.blocks.len(),
        2,
        "setup: two top-level blocks, so the second has somewhere to go"
    );

    assert!(app.indent_current(), "the outline indent reported no move");

    assert_eq!(
        app.page.blocks.len(),
        1,
        "the second block was not nested, got {:?}",
        app.page.blocks.iter().map(|b| &b.text).collect::<Vec<_>>()
    );
    assert_eq!(app.page.blocks[0].children.len(), 1);
}
