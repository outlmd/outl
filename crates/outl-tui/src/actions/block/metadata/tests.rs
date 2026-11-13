//! Embed-row gesture tests for [`super::App`]: `Ctrl+T` cycles and
//! `g D` completes a query-result row by rewriting the **source** block,
//! never the `!((blk-…))` token — prefixing the token (`TODO !((blk-…))`)
//! destroys the reference. These drive the real dispatch through a real
//! `App` with a real on-disk source page and a live index.
//!
//! The two gestures share every fixture here; they differ only in the
//! destination the press lands on: one step closer, or `DONE` outright
//! from any state (see [`App::mark_done`] for why a block carrying a
//! rule but no marker would arm `TODO ` under a cycle instead of
//! settling).

use crate::state::{App, Focus, View};
use outl_core::{ActorId, Workspace};
use outl_md::parse::OutlineNode;
use tempfile::TempDir;

fn fresh_app() -> (App, TempDir) {
    let dir = TempDir::new().unwrap();
    let actor = ActorId::new();
    let ws = Workspace::open_in_memory(actor).unwrap();
    let app = App::new(
        dir.path().to_path_buf(),
        ws,
        actor,
        crate::theme::default_theme(),
        false,
        outl_config::TuiIconStyle::NerdFont,
    )
    .unwrap();
    (app, dir)
}

/// Write a real source page with one block, reconcile it so the
/// block gets a stable handle, and patch the index. Returns that
/// handle. Mirrors `save_page_with` minus the backlink rebuild and
/// peer announce, which are irrelevant here and would spawn a
/// background thread outliving the temp dir.
fn seed_source_page(app: &mut App, slug: &str, text: &str) -> String {
    let path = app.workspace_root.join("pages").join(format!("{slug}.md"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let page = outl_md::parse::parse(&format!("- {text}\n"));
    outl_md::write_atomic(&path, outl_md::render::render(&page).as_bytes()).unwrap();
    outl_md::reconcile::reconcile_md(&mut app.workspace, &app.hlc, &path, Some(&app.orphans_log))
        .unwrap();
    app.index.patch_page(&path, &page);
    let entries: Vec<_> = app.index.iter_blocks().collect();
    assert_eq!(entries.len(), 1, "exactly one block should be indexed");
    entries[0].ref_handle.clone()
}

/// Point the app at a query page whose single selected row is an
/// embed of `handle`, in outline focus — the shape of a query result.
fn select_embed_row(app: &mut App, dir: &TempDir, slug: &str, handle: &str) {
    app.view = View::Page(dir.path().join("pages").join(format!("{slug}.md")));
    app.page.blocks.clear();
    app.page.blocks.push(OutlineNode {
        text: format!("!(({handle}))"),
        children: vec![],
        properties: vec![],
    });
    app.flat_len = 1;
    app.selected = 0;
    app.focus = Focus::Outline;
}

/// The source block lives on the *current* page, which holds a
/// coalesced (unflushed) edit. The gesture must flush first (or it
/// cycles off a stale snapshot and clobbers the edit), and the flip
/// must survive the **next** `persist()`: without the same-page reload
/// in the embed paths, `self.page` still holds the pre-flip AST and a
/// page with no auto-run fence (a hand-written embed row) never
/// reloads — the next edit renders the stale state back over the flip.
/// `after` is the post-gesture prefix to expect on disk, and
/// `before_edit` the prefix a stale render would revert to.
fn assert_same_page_flip_survives_next_edit(gesture: fn(&mut App), after: &str, before_edit: &str) {
    let (mut app, dir) = fresh_app();
    let path = dir.path().join("pages").join("task.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();

    // On-disk state: just the source block, reconciled for a handle.
    let source_only = outl_md::parse::parse("- TODO buy milk\n");
    outl_md::write_atomic(&path, outl_md::render::render(&source_only).as_bytes()).unwrap();
    outl_md::reconcile::reconcile_md(&mut app.workspace, &app.hlc, &path, Some(&app.orphans_log))
        .unwrap();
    app.index.patch_page(&path, &source_only);
    let handle = app.index.iter_blocks().next().unwrap().ref_handle.clone();

    // In-memory current page: the source block, an embed row pointing at
    // it, and a coalesced edit that exists only in memory (not flushed).
    app.view = View::Page(path.clone());
    app.page.blocks.clear();
    app.page.blocks.push(OutlineNode {
        text: "TODO buy milk".into(),
        children: vec![],
        properties: vec![],
    });
    app.page.blocks.push(OutlineNode {
        text: format!("!(({handle}))"),
        children: vec![],
        properties: vec![],
    });
    app.page.blocks.push(OutlineNode {
        text: "unsaved note".into(),
        children: vec![],
        properties: vec![],
    });
    app.flat_len = 3;
    app.selected = 1; // the embed row
    app.focus = Focus::Outline;
    app.save(); // mark dirty — the coalesced edit is not yet on disk

    gesture(&mut app);

    let disk = std::fs::read_to_string(&path).unwrap();
    assert!(
        disk.contains(&format!("{after} buy milk")),
        "source should carry {after}, got: {disk}"
    );
    assert!(
        disk.contains("unsaved note"),
        "coalesced edit must survive the cross-save, got: {disk}"
    );

    // One more row, then the deferred persist renders `self.page` back.
    // If the gesture left the page on the pre-flip AST, the file reverts.
    app.page.blocks.push(OutlineNode {
        text: "another".into(),
        children: vec![],
        properties: vec![],
    });
    app.save();
    app.persist();

    let disk = std::fs::read_to_string(&path).unwrap();
    assert!(
        disk.contains(&format!("{after} buy milk")),
        "the flip must survive the next persist, got: {disk}"
    );
    assert!(
        !disk.contains(&format!("{before_edit} buy milk")),
        "the pre-flip state leaked back into the file, got: {disk}"
    );
}

// ── Ctrl+T (cycle) ───────────────────────────────────────────────

#[test]
fn cycling_an_embed_row_flips_the_source_block_not_the_token() {
    let (mut app, dir) = fresh_app();
    let handle = seed_source_page(&mut app, "groceries", "TODO buy milk");
    select_embed_row(&mut app, &dir, "query", &handle);

    app.toggle_todo();

    // The row stays a pristine embed token — prefixing it would
    // destroy the reference (the original bug).
    assert_eq!(app.page.blocks[0].text, format!("!(({handle}))"));
    // The source block cycled TODO -> DOING on its own page…
    let source = dir.path().join("pages").join("groceries.md");
    let source_text = std::fs::read_to_string(source).unwrap();
    assert!(
        source_text.contains("DOING buy milk"),
        "source should have cycled, got: {source_text}"
    );
    // …and the index reflects the new state under the same handle.
    let entry = app.index.resolve_block_ref(&handle).expect("still indexed");
    assert_eq!(entry.text, "DOING buy milk");
}

#[test]
fn cycling_an_embed_row_twice_reaches_done() {
    let (mut app, dir) = fresh_app();
    let handle = seed_source_page(&mut app, "groceries", "TODO buy milk");
    select_embed_row(&mut app, &dir, "query", &handle);

    app.toggle_todo();
    app.toggle_todo();

    let entry = app.index.resolve_block_ref(&handle).expect("still indexed");
    assert_eq!(entry.text, "DONE buy milk");
}

#[test]
fn an_orphan_embed_handle_is_a_noop() {
    let (mut app, dir) = fresh_app();
    select_embed_row(&mut app, &dir, "query", "blk-deadbe");

    app.toggle_todo();

    // Nothing resolved, nothing cycled, the token is untouched.
    assert_eq!(app.page.blocks[0].text, "!((blk-deadbe))");
    assert!(app.status.contains("orphan"), "got: {}", app.status);
}

#[test]
fn a_plain_block_still_cycles_in_place() {
    let (mut app, _dir) = fresh_app();
    app.page.blocks.clear();
    app.page.blocks.push(OutlineNode {
        text: "buy milk".into(),
        children: vec![],
        properties: vec![],
    });
    app.flat_len = 1;
    app.selected = 0;
    app.focus = Focus::Outline;

    app.toggle_todo();

    assert_eq!(app.page.blocks[0].text, "TODO buy milk");
}

#[test]
fn cycling_a_same_page_source_keeps_unflushed_edits() {
    assert_same_page_flip_survives_next_edit(App::toggle_todo, "DOING", "TODO");
}

// ── g D (mark done) ──────────────────────────────────────────────

#[test]
fn marking_an_embed_row_done_flips_the_source_not_the_token() {
    let (mut app, dir) = fresh_app();
    let handle = seed_source_page(&mut app, "groceries", "buy milk");
    select_embed_row(&mut app, &dir, "query", &handle);

    app.mark_done();

    // The row stays a pristine embed token — prefixing it would
    // destroy the reference, same trap as Ctrl+T.
    assert_eq!(app.page.blocks[0].text, format!("!(({handle}))"));
    let source = dir.path().join("pages").join("groceries.md");
    let source_text = std::fs::read_to_string(source).unwrap();
    assert!(
        source_text.contains("DONE buy milk"),
        "source should have landed DONE from a bare block, got: {source_text}"
    );
    let entry = app.index.resolve_block_ref(&handle).expect("still indexed");
    assert_eq!(entry.text, "DONE buy milk");
}

#[test]
fn marking_an_embed_row_done_lands_from_any_earlier_state_in_one_press() {
    // Where `toggle_todo` needs two presses to walk TODO -> DOING ->
    // DONE, `mark_done` needs one from any of them.
    let (mut app, dir) = fresh_app();
    let handle = seed_source_page(&mut app, "groceries", "TODO buy milk");
    select_embed_row(&mut app, &dir, "query", &handle);

    app.mark_done();

    let entry = app.index.resolve_block_ref(&handle).expect("still indexed");
    assert_eq!(entry.text, "DONE buy milk");
}

#[test]
fn marking_an_embed_row_done_twice_is_idempotent() {
    let (mut app, dir) = fresh_app();
    let handle = seed_source_page(&mut app, "groceries", "DOING buy milk");
    select_embed_row(&mut app, &dir, "query", &handle);

    app.mark_done();
    app.mark_done();

    let entry = app.index.resolve_block_ref(&handle).expect("still indexed");
    assert_eq!(entry.text, "DONE buy milk");
}

#[test]
fn an_orphan_embed_handle_mark_done_is_a_noop() {
    let (mut app, dir) = fresh_app();
    select_embed_row(&mut app, &dir, "query", "blk-deadbe");

    app.mark_done();

    assert_eq!(app.page.blocks[0].text, "!((blk-deadbe))");
    assert!(app.status.contains("orphan"), "got: {}", app.status);
}

#[test]
fn a_plain_block_marks_done_in_place() {
    let (mut app, _dir) = fresh_app();
    app.page.blocks.clear();
    app.page.blocks.push(OutlineNode {
        text: "buy milk".into(),
        children: vec![],
        properties: vec![],
    });
    app.flat_len = 1;
    app.selected = 0;
    app.focus = Focus::Outline;

    app.mark_done();

    assert_eq!(app.page.blocks[0].text, "DONE buy milk");
}

#[test]
fn marking_a_same_page_source_done_keeps_unflushed_edits() {
    assert_same_page_flip_survives_next_edit(App::mark_done, "DONE", "TODO");
}
