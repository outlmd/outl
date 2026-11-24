//! Structural ops driven from the backlinks panel.
//!
//! They write to a page the user is not looking at. They used to do it
//! by parsing that page's `.md`, mutating the AST and writing it back,
//! letting `reconcile_md` infer which ops that implied; the op log is
//! the source of truth, so the op comes first now and the `.md` is
//! downstream of it.
//!
//! The fixtures build a real two-page workspace and a real backlink
//! index, because every one of these resolves its target through
//! `backlinks_for_current` — a test that fakes that list has not
//! exercised the path under test. The regression net for #430 / #431
//! lives with the commit helper (`backlink_commit/tests.rs`).

use crate::actions::block::test_support::{
    workspace_with_a_backlink, workspace_with_a_self_backlink,
    workspace_with_backlinks_on_two_pages,
};
use crate::actions::block::InsertCursor;
use crate::state::{App, EditTarget, Focus, Mode};
use outl_core::id::NodeId;

/// The citing block's parent in the workspace tree.
fn parent_of(app: &App, node: NodeId) -> Option<NodeId> {
    app.workspace.tree().parent(node)
}

/// How many top-level blocks a page holds, read from the **tree**.
///
/// Counted rather than collected by text, because a freshly created
/// block has none and `block_text` answers `None` for it — which is
/// exactly the block these tests are looking for.
fn top_level_count(app: &App, slug: &str) -> usize {
    let page = outl_actions::find_by_slug(&app.workspace, slug).expect("the page is in the tree");
    outl_actions::children_of(&app.workspace, page).len()
}

/// `workspace_with_a_self_backlink` is the fixture with a citing block
/// that has a previous sibling — `workspace_with_a_backlink` puts it
/// first on its page, where an indent is a boundary no-op.
#[test]
fn indenting_a_backlink_lands_as_a_move_op() {
    let (mut app, _d, _open) = workspace_with_a_self_backlink();
    let target = app
        .focused_backlink_target()
        .expect("the self-citation resolves");
    let before = parent_of(&app, target.block).expect("the citing block is in the tree");

    let moved = app.indent_current();

    assert!(moved, "the indent reported no move");
    assert_ne!(
        parent_of(&app, target.block),
        Some(before),
        "the indent never reached the op log: the citing block kept its parent"
    );
}

/// The structural ops read the tree, so they do not need the source
/// page's projection on this disk. The `.md`-first path returned early
/// at its `read_to_string`, which is the state a page whose ops have
/// synced but whose `.md` has not is in.
#[test]
fn a_structural_op_needs_no_md_on_this_disk() {
    let (mut app, _d, source) = workspace_with_a_backlink();
    let target = app
        .focused_backlink_target()
        .expect("a healthy backlink row resolves");
    std::fs::remove_file(&source).expect("drop the source page's projection");

    app.delete_current();

    assert!(
        outl_actions::is_trashed(&app.workspace, target.block),
        "the delete went looking for a `.md` instead of the op log"
    );
}

#[test]
fn deleting_a_backlink_trashes_the_citing_block() {
    let (mut app, _d, _source) = workspace_with_a_backlink();
    let target = app
        .focused_backlink_target()
        .expect("a healthy backlink row resolves");

    app.delete_current();

    assert!(
        outl_actions::is_trashed(&app.workspace, target.block),
        "delete is `Move(node, TRASH_ROOT)`, never a physical removal"
    );
}

/// **The self-citing revert, closed.** The `.md`-first route parsed
/// the source `.md`, mutated it and wrote it — and when that
/// page was also the page on screen, `App.page` was the AST loaded
/// *before* the write. One more keystroke marked it dirty and the next
/// `persist` rendered the stale AST back over the file, taking the
/// structural op with it, silently.
///
/// `commit_backlink_mutation` reloads the open page after the commit,
/// which is what closes it — so this asserts through a `persist`
/// rather than on the file the op just wrote.
#[test]
fn a_structural_op_on_a_self_citing_page_survives_the_next_persist() {
    let (mut app, _d, open) = workspace_with_a_self_backlink();

    app.indent_current();

    // The user keeps working on the page on screen, and it drains.
    app.page.blocks.push(outl_md::parse::OutlineNode {
        text: "a later note".into(),
        ..Default::default()
    });
    app.save();
    app.flush_pending_save();

    let md = std::fs::read_to_string(&open).expect("the open page's `.md`");
    assert!(
        md.contains("  - mentions [[target]]"),
        "the indent was reverted by a persist of the stale AST:\n{md}"
    );
    assert!(
        md.contains("a later note"),
        "and the later edit has to survive too:\n{md}"
    );
}

/// **`o` must not land the caret on a block the user did not aim at.**
///
/// The cursor in this panel is `Focus::Backlink { idx }`, which names a
/// *backlink row*; a brand-new sibling cites nothing, so it is not a
/// row and the focus model cannot point at it. The re-entry used that
/// focus anyway and so put the caret back on the **citing** block,
/// where the next keystroke edited text the user had not aimed at.
#[test]
fn opening_a_block_from_the_backlinks_panel_does_not_enter_insert() {
    let (mut app, _d, _source) = workspace_with_a_backlink();

    app.create_block_below();

    assert!(
        matches!(app.mode, Mode::Normal),
        "`o` on a backlink row entered Insert on a block it cannot name, got {:?}",
        app.mode
    );
    assert!(
        app.status.contains("source") && app.status.contains("not editable"),
        "a gesture that declines to start editing has to name the page the \
         block went to and say so, got {:?}",
        app.status
    );
}

#[test]
fn opening_a_block_from_the_backlinks_panel_still_creates_it_on_the_source_page() {
    let (mut app, _d, _source) = workspace_with_a_backlink();
    let before = top_level_count(&app, "source");

    app.create_block_below();

    assert_eq!(
        top_level_count(&app, "source"),
        before + 1,
        "the block was not created on the source page"
    );
}

/// **The case that can finish.** A page can cite itself, and the
/// commit's reload has already put the new block in `App.page` — so
/// `o` does what it does anywhere else rather than telling a user
/// looking straight at the block that it is not editable.
#[test]
fn opening_a_block_on_a_self_citing_page_lands_the_caret_in_it() {
    let (mut app, _d, _open) = workspace_with_a_self_backlink();

    app.create_block_below();

    assert!(
        matches!(
            &app.mode,
            Mode::Insert {
                target: EditTarget::CurrentPage,
                ..
            }
        ),
        "the new block is on the page on screen, so `o` can finish there; got {:?}",
        app.mode
    );
    assert_eq!(
        app.focus,
        Focus::Outline,
        "the cursor has to leave the panel for the block it is now editing"
    );
    assert_eq!(
        app.current_block_text(),
        "",
        "the caret landed on an existing block instead of the new one"
    );
}

#[test]
fn opening_a_block_above_a_backlink_row_does_not_enter_insert_either() {
    let (mut app, _d, _source) = workspace_with_a_backlink();

    app.create_block_above();

    assert!(
        matches!(app.mode, Mode::Normal),
        "`O` on a backlink row entered Insert on a block it cannot name, got {:?}",
        app.mode
    );
    assert_eq!(
        top_level_count(&app, "source"),
        3,
        "the block still has to be created on the source page"
    );
}

/// A top-level citing block has nowhere to be promoted to. The outline
/// path is a silent `false` there, and so is this one: the refusal
/// `outl_actions::outdent` returns names a `NodeId`, which is not a
/// thing any outl surface shows a user.
#[test]
fn outdenting_a_top_level_backlink_says_nothing_and_records_nothing() {
    let (mut app, _d, _source) = workspace_with_a_backlink();
    app.status = "untouched".into();
    app.undo.clear();

    let moved = app.outdent_current();

    assert!(!moved, "a top-level citing block cannot be outdented");
    assert_eq!(
        app.status, "untouched",
        "a boundary no-op must not write a status line"
    );
    assert!(
        app.undo.is_empty(),
        "and it must not record history for a change that did not happen"
    );
}

/// The in-flight buffer has to land before the structural op, or the
/// keystrokes typed into the row before `Tab` are dropped.
/// The `.md`-first route committed it inline;
/// `commit_backlink_mutation` drains the AST save queue and knows
/// nothing about an Insert buffer, so the branch owns it now.
#[test]
fn an_in_flight_backlink_edit_lands_before_the_indent() {
    let (mut app, _d, _open) = workspace_with_a_self_backlink();
    let target = app
        .focused_backlink_target()
        .expect("the self-citation resolves");
    app.enter_insert(InsertCursor::Start);
    let Mode::Insert { buffer, .. } = &mut app.mode else {
        panic!("`i` on a backlink row must enter Insert");
    };
    *buffer = crate::edit_buffer::EditBuffer::from_text("mentions [[target]] and is edited");

    app.indent_current();

    assert_eq!(
        app.workspace.block_text(target.block).as_deref(),
        Some("mentions [[target]] and is edited"),
        "the typed text was dropped by the structural op"
    );
}

/// **`Tab` mid-edit used to re-enter Insert positionally**, which lands
/// the caret on a different citing block — here, one on a different
/// page.
///
/// `reparent_a_backlink` resolves its target by **id** up front, commits
/// the in-flight buffer, commits the reparent, and then the one step
/// that went back to *position*: the re-entry asked only whether focus
/// was still `Focus::Backlink { .. }` and let `enter_insert` re-resolve
/// the row through `backlinks_for_current()[idx]`. The inner
/// `commit_insert` has re-indexed the source page and **re-clamped**
/// `idx` in between, so `idx` can name a row the chord was never aimed
/// at — and the guard could not tell.
///
/// The edit here drops `[[target]]` out of `b`'s citing block, which
/// takes its own row off the list, so the clamp pulls `idx` onto the row
/// citing from `a`. Every keystroke after the `Tab`, and the `Op::Edit`
/// that `Esc` commits, then landed on `a` — a page the user never aimed
/// at, with the status line and the painted row saying nothing.
///
/// This is the failure `create_beside_a_backlink`'s doc names ("put the
/// caret back on the **citing** block, where the next keystroke edited
/// text the user had not aimed at") reappearing one function over, on
/// the other side of the clamp.
#[test]
fn a_tab_mid_edit_never_re_enters_insert_on_another_pages_block() {
    let (mut app, _d) = workspace_with_backlinks_on_two_pages();
    let rows = app.backlinks_for_current();
    let at = rows
        .iter()
        .position(|bl| bl.block_text.contains("from b"))
        .expect("setup: `b` cites the open page");
    let elsewhere = rows
        .iter()
        .find(|bl| bl.block_text.contains("from a"))
        .expect("setup: `a` cites it too, and is the row a positional re-entry lands on");
    let elsewhere_block = crate::actions::node_id(&elsewhere.block_id).expect("a ULID");
    let elsewhere_page = elsewhere
        .source_page
        .as_ref()
        .and_then(|page| crate::actions::node_id(&page.id))
        .expect("a ULID");
    app.focus = Focus::Backlink {
        idx: at,
        node: None,
    };
    let aimed = app
        .focused_backlink_target()
        .expect("the row under the cursor resolves");
    assert_ne!(
        aimed.block, elsewhere_block,
        "setup: two distinct citing blocks"
    );
    assert_ne!(
        aimed.page, elsewhere_page,
        "setup: and they live on different pages"
    );

    app.enter_insert(InsertCursor::Start);
    let Mode::Insert { buffer, .. } = &mut app.mode else {
        panic!("`i` on a backlink row must enter Insert");
    };
    // Dropping the mention is what shrinks the list under `idx`.
    *buffer = crate::edit_buffer::EditBuffer::from_text("no longer cites it");

    app.indent_current();

    if let Mode::Insert { target, .. } = &app.mode {
        assert_eq!(
            *target,
            EditTarget::SourcePage {
                page: aimed.page,
                block: aimed.block,
            },
            "`Tab` re-entered Insert on a block the chord was not aimed at: \
             the next keystroke edits a page the user never chose"
        );
    }
    // And the op the chord *did* ask for still landed on the right block.
    assert_eq!(
        app.workspace.block_text(aimed.block).as_deref(),
        Some("no longer cites it"),
        "the in-flight edit has to land on the row the chord was aimed at"
    );
}
