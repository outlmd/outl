//! A backlink edit writes to a page the user is not looking at, and
//! used to do it by rendering that page's whole `.md` and letting
//! `reconcile_md` infer the ops. These drive the keystrokes
//! (`enter_insert` → `commit_insert`) rather than the commit helper,
//! so they describe what the user gets either way.

use crate::actions::block::test_support::{
    workspace_with_a_backlink, workspace_with_a_self_backlink,
};
use crate::actions::block::InsertCursor;
use crate::edit_buffer::EditBuffer;
use crate::state::{App, EditTarget, Focus, Mode};

const EDITED: &str = "mentions [[target]] and is edited";

/// `i`, replace the line, `Esc` — on whichever backlink row the
/// cursor is parked on.
///
/// `pub(super)` because `ahead_of_log_tests` drives the same keystrokes
/// at the same row; a second copy of the driver is how two test modules
/// start disagreeing about what "a backlink edit" is.
pub(super) fn edit_the_focused_backlink(app: &mut App, text: &str) {
    app.enter_insert(InsertCursor::Start);
    let Mode::Insert { buffer, .. } = &mut app.mode else {
        panic!(
            "`i` on a backlink row must enter Insert, got {:?}",
            app.mode
        );
    };
    *buffer = EditBuffer::from_text(text);
    app.commit_insert();
}

/// A page's top-level block texts, read from the **tree**.
fn page_texts(app: &App, slug: &str) -> Vec<String> {
    let page = outl_actions::find_by_slug(&app.workspace, slug).expect("the page is in the tree");
    outl_actions::children_of(&app.workspace, page)
        .into_iter()
        .filter_map(|(id, _)| app.workspace.block_text(id))
        .collect()
}

#[test]
fn a_backlink_edit_lands_as_an_op_on_the_source_block() {
    // Asserted on the workspace tree, not on the `.md`: the op log
    // is the source of truth and arriving there as an op keyed on
    // the citing block's `NodeId` is the point of the change.
    let (mut app, _d, _source) = workspace_with_a_backlink();
    let backlink = app
        .focused_backlink_target()
        .expect("a healthy backlink row resolves");

    edit_the_focused_backlink(&mut app, EDITED);

    assert_eq!(
        app.workspace.block_text(backlink.block).as_deref(),
        Some(EDITED),
        "the edit never reached the op log on the citing block"
    );
}

#[test]
fn the_edit_reaches_the_source_pages_md_too() {
    // The projection still runs; the `.md` is downstream of the op
    // now instead of being the thing that carried it.
    let (mut app, _d, source) = workspace_with_a_backlink();

    edit_the_focused_backlink(&mut app, EDITED);

    let md = std::fs::read_to_string(&source).expect("the source page's `.md`");
    assert!(
        md.contains("and is edited"),
        "the source page's `.md` is behind the op log:\n{md}"
    );
}

/// **The flush has to be first, and only a self-citing page can
/// prove it.**
///
/// On two pages a queued save and a cross-page op never interact —
/// the save renders `target.md`, the op writes `source.md` — so the
/// same assertions pass with `flush_pending_save()` moved to the
/// last line of `commit_backlink_mutation`. That is what the first
/// version of this test did, and it pinned nothing.
///
/// When the source page *is* the open page the order decides the
/// outcome: with the flush last, `project` writes the `.md` from a
/// log that has not seen the queued edit, the reload then replaces
/// `App.page` with that file, and the queued edit is gone before the
/// flush it was waiting for ever runs.
#[test]
fn a_pending_ast_edit_is_flushed_before_the_cross_page_op() {
    let (mut app, _d, _open) = workspace_with_a_self_backlink();
    let i = app
        .page
        .blocks
        .iter()
        .position(|b| b.text == "target body")
        .expect("setup: the fixture's non-citing block");
    app.page.blocks[i].text = "target body, rewritten".to_string();
    app.save();
    assert!(app.has_pending_save(), "setup: the AST edit must be queued");
    let backlink = app
        .focused_backlink_target()
        .expect("the self-citation resolves");

    edit_the_focused_backlink(&mut app, EDITED);

    let open = page_texts(&app, "target");
    assert!(
        open.iter().any(|t| t == "target body, rewritten"),
        "the queued AST edit was dropped: the commit reloaded the open \
         page over it before the flush ran, got {open:?}"
    );
    assert_eq!(
        app.workspace.block_text(backlink.block).as_deref(),
        Some(EDITED),
        "and the cross-page edit has to land as well"
    );
}

/// The broader regression the two-page case still buys: both edits
/// land, and nothing is left queued. It does **not** pin the
/// ordering — see the test above for why.
#[test]
fn a_queued_ast_edit_and_a_cross_page_edit_both_land() {
    let (mut app, _d, _source) = workspace_with_a_backlink();
    app.page.blocks[0].text = "target body, rewritten".to_string();
    app.save();
    let backlink = app
        .focused_backlink_target()
        .expect("a healthy backlink row resolves");

    edit_the_focused_backlink(&mut app, EDITED);

    assert!(!app.has_pending_save(), "the save was never drained");
    let open = page_texts(&app, "target");
    assert!(
        open.iter().any(|t| t == "target body, rewritten"),
        "the queued AST edit never reached the op log, got {open:?}"
    );
    assert_eq!(
        app.workspace.block_text(backlink.block).as_deref(),
        Some(EDITED)
    );
}

/// The reload is the **same page reparsed**, not a view change, so
/// the resets `load_current_no_autorun` makes for navigation are
/// wrong here: the cursor is still on the backlink row and the row
/// still exists.
///
/// Leaving `focus` reset is not cosmetic — `enter_insert` dispatches
/// on it, which is the first link in
/// `a_structural_op_after_a_self_citing_edit_still_targets_the_source`.
#[test]
fn editing_a_backlink_on_the_open_page_keeps_the_cursor_in_the_panel() {
    let (mut app, _d, _open) = workspace_with_a_self_backlink();
    assert_eq!(
        app.focus,
        Focus::Backlink { idx: 0 },
        "setup: the cursor starts on the citing row"
    );
    app.zoom_stack.push(vec![0]);

    edit_the_focused_backlink(&mut app, EDITED);

    assert_eq!(
        app.focus,
        Focus::Backlink { idx: 0 },
        "the reload snapped the cursor back to the outline"
    );
    assert_eq!(
        app.zoom_stack,
        vec![vec![0]],
        "and it discarded an active `z i` zoom"
    );
}

/// **The silent revert.** `Tab` on a self-citing backlink row, mid
/// edit: the commit reloads the open page, the reload used to reset
/// `Focus`, and `enter_insert` then re-entered Insert on `App.page`
/// with `EditTarget::CurrentPage` — an AST loaded *before* the
/// structural op wrote the indent. The next keystroke
/// plus `persist` rendered that stale AST back over the file and the
/// indent was gone, with nothing said.
#[test]
fn a_structural_op_after_a_self_citing_edit_still_targets_the_source() {
    let (mut app, _d, open) = workspace_with_a_self_backlink();
    app.enter_insert(InsertCursor::Start);
    // The buffer has to actually differ, or the branch's
    // `commit_insert` is a no-op and never reaches the reload that
    // resets `Focus` — the test would pass without exercising it.
    let Mode::Insert { buffer, .. } = &mut app.mode else {
        panic!("`i` on a backlink row must enter Insert");
    };
    *buffer = EditBuffer::from_text(EDITED);

    app.indent_current();

    assert!(
        matches!(
            &app.mode,
            Mode::Insert {
                target: EditTarget::SourcePage { .. },
                ..
            }
        ),
        "`Tab` re-entered Insert on the open page instead of the citing \
         block, got {:?}",
        app.mode
    );

    // The rest of the chain: the user keeps typing, then everything
    // drains.
    if let Mode::Insert { buffer, .. } = &mut app.mode {
        buffer.insert_str(" and more");
    }
    app.commit_insert();
    app.flush_pending_save();

    let md = std::fs::read_to_string(&open).expect("the open page's `.md`");
    assert!(
        md.contains("  - "),
        "the indent was reverted by a persist of the stale AST:\n{md}"
    );
    assert!(
        md.contains("and more"),
        "and the typed text has to be on the citing block:\n{md}"
    );
}

#[test]
fn editing_a_backlink_on_the_open_page_keeps_the_outline_in_step() {
    // A page can cite itself, so the source page is sometimes the
    // page on screen. Without a reload the in-memory AST is behind
    // the op log and the next `persist` writes the stale text back.
    let (mut app, _d, open) = workspace_with_a_self_backlink();
    let backlink = app
        .focused_backlink_target()
        .expect("the self-citation resolves");
    assert_eq!(
        Some(backlink.page),
        outl_actions::find_by_slug(&app.workspace, "target"),
        "setup: the source page must be the page on screen"
    );

    edit_the_focused_backlink(&mut app, EDITED);

    let on_screen: Vec<&str> = app.page.blocks.iter().map(|b| b.text.as_str()).collect();
    assert!(
        on_screen.contains(&EDITED),
        "the open page's AST is behind the op log, got {on_screen:?}"
    );
    let md = std::fs::read_to_string(&open).expect("the open page's `.md`");
    assert!(
        md.contains("and is edited"),
        "and the `.md` has to agree with both:\n{md}"
    );
}

#[test]
fn a_backlink_edit_leaves_an_undo_entry_for_the_source_page() {
    // [#431](https://github.com/outlmd/outl/issues/431): a cross-page
    // op snapshots the page it changes, under that page's own path,
    // so `top_applies_here` only offers it once the user opens it.
    let (mut app, _d, source) = workspace_with_a_backlink();
    app.undo.clear();

    edit_the_focused_backlink(&mut app, EDITED);

    let snap = app
        .undo
        .iter()
        .find(|s| s.view_path == source)
        .unwrap_or_else(|| {
            panic!(
                "no undo entry for {}; stack holds {:?}",
                source.display(),
                app.undo.iter().map(|s| &s.view_path).collect::<Vec<_>>()
            )
        });
    assert!(
        snap.page
            .blocks
            .iter()
            .any(|b| b.text == "mentions [[target]]"),
        "the snapshot must carry the pre-edit text, got {:?}",
        snap.page.blocks.iter().map(|b| &b.text).collect::<Vec<_>>()
    );
}

/// `self.index` is what `/` search, the `((` autocomplete and
/// `((blk-XXXXXX))` resolution all read, and **navigation does not
/// rebuild it** — so a miss here lasts until `Ctrl+L`, a peer
/// reload or a relaunch, not until the next keystroke.
///
/// Asserted on `iter_blocks`, the entry point the first two of those
/// three readers use.
#[test]
fn a_backlink_edit_keeps_the_workspace_index_searchable() {
    let (mut app, _d, _source) = workspace_with_a_backlink();
    let backlink = app
        .focused_backlink_target()
        .expect("a healthy backlink row resolves");

    edit_the_focused_backlink(&mut app, EDITED);

    let indexed: Vec<&str> = app
        .index
        .iter_blocks()
        .map(|entry| entry.text.as_str())
        .collect();
    assert!(
        indexed.contains(&EDITED),
        "searching the new text finds nothing: {indexed:?}"
    );
    assert!(
        !indexed.contains(&"mentions [[target]]"),
        "and the pre-edit text still matches: {indexed:?}"
    );
    assert!(
        app.index
            .iter_blocks()
            .any(|entry| entry.id == backlink.block && entry.text == EDITED),
        "the entry under the citing block's own id was not patched"
    );
}

/// **The cross-page half, and the one that needs the clamp most.**
///
/// A `d d` on the only backlink of a page that is *not* on screen
/// empties the panel with no reload involved, so a clamp wired to the
/// self-citing reload never runs. From `Focus::Backlink { idx: 0 }`
/// over zero rows there is no keyboard way out: `j` and `k` both
/// decline a stale `idx` by design, `enter_insert_backlink` returns
/// silently, and every other chord reports that the row names nothing.
/// Only a navigation or a mouse click recovered it, and mouse capture
/// is off by default.
#[test]
fn a_cross_page_delete_that_empties_the_panel_returns_the_cursor_to_the_outline() {
    let (mut app, _d, _source) = workspace_with_a_backlink();
    assert_eq!(app.focus, Focus::Backlink { idx: 0 });
    assert!(
        !app.page_is_open(
            app.focused_backlink_target()
                .expect("a healthy backlink row resolves")
                .page
        ),
        "setup: the source page must be off screen, or this exercises the reload instead"
    );

    app.delete_current();

    assert!(
        app.backlinks_for_current().is_empty(),
        "setup: the delete has to empty the panel"
    );
    assert_eq!(
        app.focus,
        Focus::Outline,
        "the cursor was stranded on a row that no longer exists"
    );
}

/// `idx` is positional and the list can shrink under the very edit
/// that triggered the reload: dropping the `[[target]]` out of a
/// self-citing block takes its own row with it.
///
/// The consumers tolerate an out-of-range `idx` (both answer "nothing
/// focused"), but the reset this replaced could not produce the state
/// at all — so it is clamped where the commit ends rather than relied
/// upon where it is read.
#[test]
fn an_edit_that_drops_the_last_self_citation_returns_the_cursor_to_the_outline() {
    let (mut app, _d, _open) = workspace_with_a_self_backlink();
    assert_eq!(app.focus, Focus::Backlink { idx: 0 });

    edit_the_focused_backlink(&mut app, "no longer mentions anything");

    assert!(
        app.backlinks_for_current().is_empty(),
        "setup: the edit has to empty the panel"
    );
    assert_eq!(
        app.focus,
        Focus::Outline,
        "the cursor was left on a backlink row that no longer exists"
    );
}

#[test]
fn the_buffer_seeds_from_the_op_log_not_the_md() {
    // No `.md` is read to start the edit. Bytes nothing emitted ops
    // for are exactly what the old path would have handed the buffer
    // — and then written back as the user's own text.
    let (mut app, _d, source) = workspace_with_a_backlink();
    std::fs::write(&source, "- something else entirely\n").expect("rewrite the source `.md`");

    app.enter_insert(InsertCursor::Start);

    let Mode::Insert { buffer, .. } = &app.mode else {
        panic!(
            "`i` on a backlink row must enter Insert, got {:?}",
            app.mode
        );
    };
    assert_eq!(
        buffer.as_string(),
        "mentions [[target]]",
        "the buffer was seeded from the `.md` instead of the op log"
    );
}

// ── #430 / #431: a cross-page op's undo entry ────────────────────
//
// These four arrived with the `.md`-first mechanism (`backlink_edit.rs`)
// and moved here unchanged when it was deleted: they drive the chords
// (`delete_current`, `move_block_up`), so they describe what the user
// gets from whichever mechanism is underneath.
//
// A structural op driven from the backlinks panel mutates a file that
// is not on screen, and used to leave no undo entry at all
// ([#431](https://github.com/outlmd/outl/issues/431)).

#[test]
fn deleting_a_block_from_the_backlinks_panel_leaves_an_undo_entry() {
    let (mut app, _d, source) = workspace_with_a_backlink();
    app.undo.clear();

    app.delete_current();

    assert!(
        app.undo.iter().any(|s| s.view_path == source),
        "the cross-page delete left no undo entry for {}; stack holds {:?}",
        source.display(),
        app.undo.iter().map(|s| &s.view_path).collect::<Vec<_>>()
    );
}

#[test]
fn the_snapshot_holds_the_source_page_as_it_was_before_the_delete() {
    let (mut app, _d, source) = workspace_with_a_backlink();
    app.undo.clear();

    app.delete_current();

    let snap = app
        .undo
        .iter()
        .find(|s| s.view_path == source)
        .expect("a snapshot for the source page");
    assert!(
        snap.page
            .blocks
            .iter()
            .any(|b| b.text.contains("mentions [[target]]")),
        "the snapshot must carry the pre-delete text, got {:?}",
        snap.page.blocks.iter().map(|b| &b.text).collect::<Vec<_>>()
    );
}

#[test]
fn the_snapshot_does_not_apply_to_the_page_on_screen() {
    // It belongs to the source page, so pressing `u` while still
    // looking at the target must not restore it here. That guard is
    // `top_applies_here` (#430), and it is what makes carrying a
    // cross-page snapshot safe in the first place.
    let (mut app, _d, source) = workspace_with_a_backlink();
    app.undo.clear();
    app.delete_current();

    let before = app.undo.len();
    app.undo();

    assert_eq!(
        app.undo.len(),
        before,
        "the cross-page snapshot was consumed by an undo on the wrong page"
    );
    assert_eq!(app.status, "no undo for this page");
    assert!(app.undo.iter().any(|s| s.view_path == source));
}

#[test]
fn a_boundary_no_op_leaves_both_history_stacks_unchanged() {
    // The backlink's block is the source page's first sibling, so
    // moving it up changes nothing and must record nothing.
    let (mut app, _d, source) = workspace_with_a_backlink();
    app.redo.push(crate::state::HistorySnapshot {
        page: Default::default(),
        selected: 0,
        cursor_col: 0,
        view_path: source,
    });
    let undo_before: Vec<_> = app.undo.iter().map(|s| s.view_path.clone()).collect();
    let redo_before: Vec<_> = app.redo.iter().map(|s| s.view_path.clone()).collect();

    app.move_block_up();

    let undo_after: Vec<_> = app.undo.iter().map(|s| s.view_path.clone()).collect();
    let redo_after: Vec<_> = app.redo.iter().map(|s| s.view_path.clone()).collect();
    assert_eq!(undo_after, undo_before);
    assert_eq!(redo_after, redo_before);
}

/// A row can name a block the op log no longer has: the backlink index
/// is built from disk, and a peer's delete arrives as an op under it.
///
/// The refusal has to reach the user **without a block id in it**, and
/// in words this crate does not own. Routing the chords through
/// `commit_page` handed the status line `ActionError`'s own sentence,
/// which names the `NodeId` — and no outl surface shows a user a block
/// id (the trash refusals were reworded for exactly this reason).
///
/// Asserted against `outl_actions::refusal` rather than a literal, on
/// purpose: `outl-actions/CLAUDE.md`'s rule is that a client decides
/// where a notice goes and never what it says, so a copy of the
/// sentence **in the test** would be the second owner just as much as
/// one in the source. The `unchanged` half is the one that fits a
/// chord: nothing was typed, so nothing was lost.
#[test]
fn a_chord_on_a_block_the_log_lost_says_so_in_the_shared_words() {
    let (mut app, _d, _source) = workspace_with_a_backlink();
    let target = app
        .focused_backlink_target()
        .expect("a healthy backlink row resolves");
    let hlc = app.hlc.clone();
    outl_actions::delete(&mut app.workspace, &hlc, target.block)
        .expect("trash the block behind the row");

    app.toggle_todo();

    assert_eq!(
        app.status,
        outl_actions::refusal::stale_backlink_block_unchanged_refusal(&app.workspace, target.block),
        "the refusal has to be the catalog's sentence, not one this crate wrote"
    );
    assert!(
        !app.status.contains(&target.block.to_string()),
        "a block id reached the status line: {}",
        app.status
    );
    assert!(
        !app.status.contains("your text"),
        "a chord typed nothing, so it must not claim lost text: {}",
        app.status
    );
}

/// The other half of the same rule, on the entry path: `i` on a row
/// whose block the log no longer has is refused in the catalog's words
/// too, not in a second literal twenty lines away in `insert.rs`.
#[test]
fn starting_an_edit_on_a_block_the_log_lost_says_so_in_the_shared_words() {
    let (mut app, _d, _source) = workspace_with_a_backlink();
    let target = app
        .focused_backlink_target()
        .expect("a healthy backlink row resolves");
    let hlc = app.hlc.clone();
    outl_actions::delete(&mut app.workspace, &hlc, target.block)
        .expect("trash the block behind the row");

    app.enter_insert(InsertCursor::Start);

    assert!(
        matches!(app.mode, Mode::Normal),
        "the edit must not start on a block the log lost, got {:?}",
        app.mode
    );
    assert_eq!(
        app.status,
        outl_actions::refusal::stale_backlink_block_unchanged_refusal(&app.workspace, target.block)
    );
}
