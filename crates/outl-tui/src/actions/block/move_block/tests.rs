//! Single-block move in the outline (Normal mode).
//!
//! The fixtures build a real page (ops, `.md`, sidecar) and open it
//! through `open_path`, because the bugs here live in the seam between
//! the in-flight AST and the parallel `id_by_flat` vector: a test that
//! pokes `page.blocks` directly would pass whether or not the two
//! stayed aligned.

use crate::actions::block::InsertCursor;
use crate::actions::page_test_support::fresh_app;
use crate::edit_buffer::EditBuffer;
use crate::input::{handle_insert_key, handle_normal_key};
use crate::state::{App, EditTarget, Focus, HistorySnapshot, Mode, MAX_HISTORY};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use outl_actions::{
    append_block, apply_page_md_with_sidecar_guarded, open_or_create_page, PageKind,
};
use outl_core::id::NodeId;
use tempfile::TempDir;

const CTRL_SHIFT: KeyModifiers = KeyModifiers::CONTROL.union(KeyModifiers::SHIFT);

/// A page `alpha` holding `spec` — `(text, parent)` where `parent` is an
/// index into earlier entries — opened in the outline. Ids come back in
/// spec order.
fn app_with(spec: &[(&str, Option<usize>)]) -> (App, TempDir, Vec<NodeId>) {
    let (mut app, dir) = fresh_app();
    let hlc = app.hlc.clone();
    let page = open_or_create_page(&mut app.workspace, &hlc, "alpha", "Alpha", PageKind::Page)
        .expect("page");
    let mut ids: Vec<NodeId> = Vec::new();
    for (text, parent) in spec {
        let parent = parent.map_or(page, |i| ids[i]);
        let id =
            append_block(&mut app.workspace, &hlc, Some(parent), Some(text)).expect("append block");
        ids.push(id);
    }
    apply_page_md_with_sidecar_guarded(&app.workspace, &app.workspace_root, page)
        .expect("project the page");
    app.open_path(app.workspace_root.join("pages/alpha.md"))
        .expect("open");
    (app, dir, ids)
}

fn flat<'a>(texts: &[&'a str]) -> Vec<(&'a str, Option<usize>)> {
    texts.iter().map(|t| (*t, None)).collect()
}

fn top_texts(app: &App) -> Vec<String> {
    app.page.blocks.iter().map(|b| b.text.clone()).collect()
}

fn press(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    handle_normal_key(app, KeyEvent::new(code, mods)).expect("key");
}

/// Order of the page's children in the **tree**, i.e. what the op log
/// says rather than what the AST shows.
fn tree_order(app: &App) -> Vec<NodeId> {
    let page = outl_actions::find_by_slug(&app.workspace, "alpha").expect("page");
    outl_actions::children_of(&app.workspace, page)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

#[test]
fn ctrl_shift_arrows_move_the_block_in_normal() {
    let (mut app, _d, ids) = app_with(&flat(&["a", "b", "c"]));
    app.selected = 1;

    press(&mut app, KeyCode::Up, CTRL_SHIFT);
    assert_eq!(top_texts(&app), ["b", "a", "c"]);
    assert_eq!(app.selected, 0, "the selection follows the block");

    press(&mut app, KeyCode::Down, CTRL_SHIFT);
    press(&mut app, KeyCode::Down, CTRL_SHIFT);
    assert_eq!(top_texts(&app), ["a", "c", "b"]);
    assert_eq!(app.selected, 2);
    assert_eq!(app.id_by_flat[2], ids[1], "ids rode along with the move");
}

#[test]
fn alt_arrows_and_shift_letters_still_move() {
    let (mut app, _d, _ids) = app_with(&flat(&["a", "b", "c"]));
    app.selected = 2;
    press(&mut app, KeyCode::Up, KeyModifiers::ALT);
    press(&mut app, KeyCode::Char('K'), KeyModifiers::SHIFT);
    assert_eq!(top_texts(&app), ["c", "a", "b"]);
    press(&mut app, KeyCode::Char('J'), KeyModifiers::SHIFT);
    assert_eq!(top_texts(&app), ["a", "c", "b"]);
}

#[test]
fn shift_arrows_and_ctrl_arrows_alone_do_not_move() {
    let (mut app, _d, _ids) = app_with(&flat(&["a", "b", "c"]));
    for mods in [KeyModifiers::SHIFT, KeyModifiers::CONTROL] {
        for code in [KeyCode::Up, KeyCode::Down] {
            press(&mut app, code, mods);
            assert_eq!(top_texts(&app), ["a", "b", "c"], "{code:?} + {mods:?}");
        }
    }
    assert!(app.undo.is_empty(), "nothing was snapshotted");
}

#[test]
fn a_boundary_move_keeps_redo_and_pushes_no_undo() {
    for (dir, selected) in [(KeyCode::Up, 0), (KeyCode::Down, 2)] {
        let (mut app, _d, _ids) = app_with(&flat(&["a", "b", "c"]));
        // Redo holds an entry, undo is full: a snapshot-then-truncate
        // would clear the first and evict the oldest of the second.
        let snap: HistorySnapshot = HistorySnapshot {
            page: app.page.clone(),
            selected: 0,
            cursor_col: 0,
            view_path: app.current_path(),
        };
        app.undo = vec![snap.clone(); MAX_HISTORY];
        app.redo = vec![snap];
        app.selected = selected;

        press(&mut app, dir, CTRL_SHIFT);

        assert_eq!(top_texts(&app), ["a", "b", "c"], "{dir:?}");
        assert_eq!(app.undo.len(), MAX_HISTORY, "{dir:?} pushed or evicted");
        assert_eq!(app.redo.len(), 1, "{dir:?} cleared redo");
        assert!(!app.has_pending_save(), "{dir:?} marked the page dirty");
    }
}

#[test]
fn fold_after_a_move_targets_the_block_under_the_cursor() {
    let (mut app, _d, ids) = app_with(&flat(&["a", "b", "c"]));
    app.selected = 1;
    press(&mut app, KeyCode::Up, CTRL_SHIFT);
    assert_eq!(app.id_by_flat[app.selected], ids[1], "before any flush");

    app.toggle_collapse_selected();

    assert!(app.workspace.tree().is_collapsed(ids[1]));
    assert!(
        !app.workspace.tree().is_collapsed(ids[0]),
        "its old neighbour"
    );
}

#[test]
fn a_folded_neighbour_stays_folded_after_the_swap() {
    let (mut app, _d, ids) = app_with(&[("a", None), ("a1", Some(0)), ("b", None)]);
    app.selected = 0;
    app.toggle_collapse_selected();
    assert_eq!(app.hidden_by_collapse, [false, true, false]);

    app.selected = 2;
    press(&mut app, KeyCode::Up, CTRL_SHIFT);

    assert_eq!(top_texts(&app), ["b", "a"]);
    assert_eq!(app.id_by_flat, [ids[2], ids[0], ids[1]]);
    assert_eq!(app.hidden_by_collapse, [false, false, true]);
    let kept = app.hidden_by_collapse.clone();
    app.recompute_hidden_by_collapse();
    assert_eq!(app.hidden_by_collapse, kept, "matches a fresh recompute");
}

#[test]
fn id_by_flat_follows_a_subtree_across_a_move() {
    let (mut app, _d, ids) = app_with(&[
        ("a", None),
        ("b", None),
        ("b1", Some(1)),
        ("b2", Some(1)),
        ("c", None),
    ]);
    app.selected = 4;
    press(&mut app, KeyCode::Up, CTRL_SHIFT);
    assert_eq!(app.id_by_flat, [ids[0], ids[4], ids[1], ids[2], ids[3]]);
    assert_eq!(app.selected, 1);

    press(&mut app, KeyCode::Down, CTRL_SHIFT);
    press(&mut app, KeyCode::Down, CTRL_SHIFT);
    assert_eq!(top_texts(&app), ["a", "b", "c"].map(String::from));
    assert_eq!(app.id_by_flat, [ids[0], ids[1], ids[2], ids[3], ids[4]]);
}

#[test]
fn the_zoom_root_does_not_move() {
    let (mut app, _d, _ids) = app_with(&[("p", None), ("x", Some(0)), ("q", None)]);
    app.selected = 0;
    app.zoom_in_on_selected();
    let zoom = app.zoom_stack.clone();

    press(&mut app, KeyCode::Down, CTRL_SHIFT);

    assert_eq!(top_texts(&app), ["p", "q"]);
    assert_eq!(app.zoom_stack, zoom);
    assert!(app.undo.is_empty() && !app.has_pending_save());
}

#[test]
fn a_child_of_the_zoom_root_moves() {
    let (mut app, _d, _ids) = app_with(&[("p", None), ("x", Some(0)), ("y", Some(0))]);
    app.selected = 0;
    app.zoom_in_on_selected();
    app.selected = 2;

    press(&mut app, KeyCode::Up, CTRL_SHIFT);

    let kids: Vec<_> = app.page.blocks[0]
        .children
        .iter()
        .map(|b| b.text.as_str())
        .collect();
    assert_eq!(kids, ["y", "x"]);
    assert_eq!(app.selected, 1);
}

#[test]
fn a_move_reaches_the_op_log_on_flush() {
    let (mut app, _d, ids) = app_with(&flat(&["a", "b", "c"]));
    app.selected = 2;
    press(&mut app, KeyCode::Up, CTRL_SHIFT);
    assert!(app.has_pending_save());

    app.flush_pending_save();

    assert_eq!(tree_order(&app), [ids[0], ids[2], ids[1]]);
}

#[test]
fn undo_then_flush_realigns_id_by_flat() {
    let (mut app, _d, ids) = app_with(&flat(&["a", "b", "c"]));
    app.selected = 2;
    press(&mut app, KeyCode::Up, CTRL_SHIFT);
    app.flush_pending_save();
    assert_eq!(app.id_by_flat, [ids[0], ids[2], ids[1]]);

    app.undo();
    assert_eq!(top_texts(&app), ["a", "b", "c"]);
    app.flush_pending_save();

    assert_eq!(app.id_by_flat, [ids[0], ids[1], ids[2]]);
}

#[test]
fn opening_a_page_without_a_sidecar_leaves_no_ids_from_the_previous_page() {
    let (mut app, _d, _ids) = app_with(&flat(&["a", "b"]));
    assert_eq!(app.id_by_flat.len(), 2, "setup: alpha has a sidecar");
    let bare = app.workspace_root.join("pages/bare.md");
    std::fs::write(&bare, "- x\n- y\n").expect("write a page nobody reconciled");

    app.open_path(bare).expect("open");

    assert!(
        app.id_by_flat.is_empty(),
        "`c` would fold a block of the previous page: {:?}",
        app.id_by_flat
    );
}

// ---- Insert ----

fn insert_press(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    handle_insert_key(app, KeyEvent::new(code, mods)).expect("key");
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        insert_press(app, KeyCode::Char(c), KeyModifiers::NONE);
    }
}

/// Select `row` and open it for editing with the caret at the start.
fn edit_row(app: &mut App, row: usize) {
    app.selected = row;
    app.cursor_col = 0;
    app.enter_insert(InsertCursor::AtCursor);
}

fn draft(app: &App) -> (String, usize) {
    match &app.mode {
        Mode::Insert { buffer, .. } => (buffer.as_string(), buffer.cursor),
        other => panic!("expected Insert, got {other:?}"),
    }
}

#[test]
fn insert_move_keeps_the_caret_and_the_draft() {
    let (mut app, _d, _ids) = app_with(&flat(&["a", "b", "c"]));
    edit_row(&mut app, 1);
    type_text(&mut app, "xy");
    assert_eq!(draft(&app), ("xyb".to_string(), 2));

    insert_press(&mut app, KeyCode::Up, KeyModifiers::ALT);

    assert_eq!(top_texts(&app), ["xyb", "a", "c"]);
    assert_eq!(draft(&app), ("xyb".to_string(), 2));
    assert_eq!(app.selected, 0);
    let Mode::Insert { block_path, .. } = &app.mode else {
        panic!("still Insert")
    };
    assert_eq!(block_path, &[0]);
}

#[test]
fn insert_move_keeps_the_node_id_of_an_edited_short_block() {
    let (mut app, _d, ids) = app_with(&flat(&["zz", "ab", "cc"]));
    edit_row(&mut app, 1);
    if let Mode::Insert { buffer, .. } = &mut app.mode {
        *buffer = EditBuffer::from_text("xyz");
    }

    insert_press(&mut app, KeyCode::Up, CTRL_SHIFT);
    app.flush_pending_save();

    assert_eq!(tree_order(&app), [ids[1], ids[0], ids[2]]);
    assert_eq!(app.workspace.block_text(ids[1]).as_deref(), Some("xyz"));
    assert_eq!(app.workspace.block_text(ids[0]).as_deref(), Some("zz"));
}

#[test]
fn insert_move_at_the_edge_keeps_insert_and_caret() {
    let (mut app, _d, _ids) = app_with(&flat(&["a", "b"]));
    edit_row(&mut app, 0);
    type_text(&mut app, "q");

    insert_press(&mut app, KeyCode::Up, KeyModifiers::ALT);

    assert_eq!(top_texts(&app), ["qa", "b"]);
    assert_eq!(draft(&app), ("qa".to_string(), 1));
    insert_press(&mut app, KeyCode::Down, KeyModifiers::ALT);
    insert_press(&mut app, KeyCode::Down, KeyModifiers::ALT);
    assert_eq!(top_texts(&app), ["b", "qa"]);
    assert_eq!(draft(&app), ("qa".to_string(), 1));
}

#[test]
fn insert_move_with_the_autocomplete_open_moves_and_closes_the_popup() {
    let (mut app, _d, _ids) = app_with(&flat(&["a", "b"]));
    edit_row(&mut app, 1);
    type_text(&mut app, "[[");
    assert!(app.autocomplete.is_some(), "setup: `[[` opens the popup");

    insert_press(&mut app, KeyCode::Up, KeyModifiers::ALT);

    assert!(app.autocomplete.is_none());
    assert_eq!(top_texts(&app)[0], "[[]]b");
    assert!(matches!(app.mode, Mode::Insert { .. }));
}

#[test]
fn plain_shift_and_ctrl_arrows_keep_their_caret_behaviour() {
    for mods in [KeyModifiers::SHIFT, KeyModifiers::CONTROL] {
        let (mut app, _d, _ids) = app_with(&flat(&["a", "b", "c"]));
        edit_row(&mut app, 1);
        insert_press(&mut app, KeyCode::Up, mods);
        insert_press(&mut app, KeyCode::Down, mods);
        assert_eq!(top_texts(&app), ["a", "b", "c"], "{mods:?} moved a block");
    }
}

#[test]
fn insert_move_on_a_backlink_row_moves_it_on_its_own_page_and_re_enters_there() {
    use crate::actions::block::test_support::workspace_with_a_backlink;
    let (mut app, _d, _source) = workspace_with_a_backlink();
    app.focus = Focus::Backlink { idx: 0, node: None };
    let target = app.focused_backlink_target().expect("a backlink row");
    app.enter_insert(InsertCursor::Start);

    insert_press(&mut app, KeyCode::Down, KeyModifiers::ALT);

    let siblings = outl_actions::children_of(
        &app.workspace,
        app.workspace.tree().parent(target.block).expect("parent"),
    );
    assert_eq!(siblings.last().map(|(id, _)| *id), Some(target.block));
    assert_eq!(
        top_texts(&app),
        ["target body"],
        "the open page is untouched"
    );
    let Mode::Insert { target: edit, .. } = &app.mode else {
        panic!("re-entered Insert");
    };
    assert_eq!(
        *edit,
        EditTarget::SourcePage {
            page: target.page,
            block: target.block
        }
    );
}

#[test]
fn insert_move_commits_the_draft_to_the_edited_block_not_the_sibling() {
    let (mut app, _d, ids) = app_with(&flat(&["a", "b", "c"]));
    edit_row(&mut app, 1);
    type_text(&mut app, "x");

    insert_press(&mut app, KeyCode::Up, KeyModifiers::ALT);
    app.flush_pending_save();

    assert_eq!(app.workspace.block_text(ids[1]).as_deref(), Some("xb"));
    assert_eq!(app.workspace.block_text(ids[0]).as_deref(), Some("a"));
    assert_eq!(tree_order(&app), [ids[1], ids[0], ids[2]]);
}

#[test]
fn insert_move_moves_a_block_created_by_enter_before_any_flush() {
    let (mut app, _d, ids) = app_with(&flat(&["a", "b"]));
    edit_row(&mut app, 0);
    if let Mode::Insert { buffer, .. } = &mut app.mode {
        buffer.cursor = 1;
    }
    insert_press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    type_text(&mut app, "n");
    assert_eq!(
        top_texts(&app),
        ["a", "", "b"],
        "`n` is still in the buffer"
    );

    insert_press(&mut app, KeyCode::Up, KeyModifiers::ALT);
    app.flush_pending_save();

    assert_eq!(top_texts(&app), ["n", "a", "b"]);
    let order = tree_order(&app);
    assert_eq!(order.len(), 3, "nothing was lost");
    assert_eq!(order[1], ids[0]);
    assert_eq!(order[2], ids[1]);
    assert_eq!(app.workspace.block_text(order[0]).as_deref(), Some("n"));
}

#[test]
fn insert_move_leaves_a_pending_reload_pending() {
    let (mut app, _d, _ids) = app_with(&flat(&["a", "b"]));
    edit_row(&mut app, 1);
    app.pending_reload = true;

    insert_press(&mut app, KeyCode::Up, KeyModifiers::ALT);

    assert!(app.pending_reload, "the next commit drains it");
    assert!(matches!(app.mode, Mode::Insert { .. }));
}

#[test]
fn two_insert_moves_with_typing_between_keep_the_node_id() {
    let (mut app, _d, ids) = app_with(&flat(&["a", "b", "c"]));
    edit_row(&mut app, 2);
    type_text(&mut app, "x");
    insert_press(&mut app, KeyCode::Up, KeyModifiers::ALT);
    type_text(&mut app, "y");
    insert_press(&mut app, KeyCode::Up, KeyModifiers::ALT);
    app.flush_pending_save();

    assert_eq!(tree_order(&app), [ids[2], ids[0], ids[1]]);
    assert_eq!(app.workspace.block_text(ids[2]).as_deref(), Some("xyc"));
}

#[test]
fn repeating_without_typing_does_not_persist() {
    let (mut app, _d, ids) = app_with(&flat(&["a", "b", "c"]));
    edit_row(&mut app, 2);

    insert_press(&mut app, KeyCode::Up, KeyModifiers::ALT);
    insert_press(&mut app, KeyCode::Up, KeyModifiers::ALT);

    assert_eq!(top_texts(&app), ["c", "a", "b"]);
    assert!(app.has_pending_save(), "a held chord never writes inline");
    assert_eq!(
        tree_order(&app),
        [ids[0], ids[1], ids[2]],
        "nothing persisted"
    );
}
