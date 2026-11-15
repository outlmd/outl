//! What `/merge` has to get right: ask before writing, project what the
//! merge touched, and never leave the user on a page that is gone.

use super::*;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use outl_actions::{
    append_block, apply_page_md_with_sidecar_guarded, open_or_create_page, PageKind,
};
use outl_core::{ActorId, Workspace};
use tempfile::TempDir;

fn fresh_app() -> (App, TempDir) {
    let dir = TempDir::new().unwrap();
    let actor = ActorId::new();
    let ws = Workspace::open_in_memory(actor).unwrap();
    let app = App::new_for_tests(
        dir.path().to_path_buf(),
        ws,
        actor,
        crate::theme::default_theme(),
        false,
    )
    .unwrap();
    (app, dir)
}

/// Two projected pages: `alpha` holds `one` and `two`, `beta` holds
/// `three`. The view is left on `alpha`.
fn seed(app: &mut App) {
    let hlc = app.hlc.clone();
    for (slug, title, blocks) in [
        ("alpha", "Alpha", ["one", "two"].as_slice()),
        ("beta", "Beta", ["three"].as_slice()),
    ] {
        let ws = &mut app.workspace;
        let page = open_or_create_page(ws, &hlc, slug, title, PageKind::Page).unwrap();
        for &text in blocks {
            append_block(ws, &hlc, Some(page), Some(text)).unwrap();
        }
        apply_page_md_with_sidecar_guarded(&app.workspace, &app.workspace_root, page).unwrap();
    }
    open(app, "alpha");
}

fn open(app: &mut App, slug: &str) {
    let path = app.workspace_root.join(format!("pages/{slug}.md"));
    app.open_path(path).unwrap();
}

fn toasted(app: &App, message: &str) -> bool {
    app.toasts.iter().any(|t| t.message == message)
}

#[test]
fn merge_arms_the_prompt_and_writes_nothing() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);

    app.arm_page_merge("Beta");

    let pending = app
        .pending_sidebar_delete
        .as_ref()
        .expect("a prompt is armed");
    assert_eq!(pending.slug, "alpha");
    assert_eq!(pending.merge_into.as_deref(), Some("beta"));
    assert!(
        app.status.contains("merge 'Alpha' into 'Beta'") && app.status.ends_with("y/n"),
        "the prompt names both pages and asks, got {:?}",
        app.status
    );
    let alpha = outl_actions::find_by_slug(&app.workspace, "alpha").unwrap();
    assert!(!outl_actions::is_trashed(&app.workspace, alpha));
    assert!(app.workspace_root.join("pages/alpha.md").exists());
}

#[test]
fn the_slash_command_reaches_the_same_prompt() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);

    let registry = app.command_registry.clone();
    registry.dispatch(&mut app, "merge Beta").unwrap();

    let pending = app
        .pending_sidebar_delete
        .as_ref()
        .expect("a prompt is armed");
    assert_eq!(pending.merge_into.as_deref(), Some("beta"));
}

#[test]
fn an_unknown_target_is_refused_not_created() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);

    app.arm_page_merge("Nowhere");

    assert!(app.pending_sidebar_delete.is_none());
    assert!(
        app.status.contains("no page 'Nowhere'"),
        "got {:?}",
        app.status
    );
    assert!(
        !app.workspace_root.join("pages/nowhere.md").exists(),
        "merging into a page that is not there must not create it"
    );
}

#[test]
fn a_page_cannot_be_merged_into_itself() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);

    app.arm_page_merge("alpha");

    assert!(app.pending_sidebar_delete.is_none());
    assert!(app.status.contains("itself"), "got {:?}", app.status);
}

#[test]
fn a_journal_is_never_the_source() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);
    // `fresh_app` lands on today's journal; `seed` moved off it.
    app.go_today().unwrap();

    app.arm_page_merge("Beta");

    assert!(app.pending_sidebar_delete.is_none());
    assert!(toasted(&app, "can't merge a journal page"));
}

#[test]
fn confirming_moves_the_blocks_and_lands_on_the_target() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);
    app.arm_page_merge("Beta");

    app.sidebar_confirm_delete().unwrap();

    let alpha = outl_actions::find_by_slug(&app.workspace, "alpha");
    assert!(
        alpha.is_none_or(|id| outl_actions::is_trashed(&app.workspace, id)),
        "the source is in the trash"
    );
    let md = std::fs::read_to_string(app.workspace_root.join("pages/beta.md")).unwrap();
    for text in ["one", "two", "three"] {
        assert!(md.contains(text), "beta.md lost {text:?}:\n{md}");
    }
    assert!(
        !app.workspace_root.join("pages/alpha.md").exists(),
        "the source's projection is removed once the target projected"
    );
    assert_eq!(app.current_slug(), "beta", "the user lands on the target");
    assert!(app.pending_sidebar_delete.is_none());
    assert!(
        toasted(&app, "merged 'Alpha' into 'Beta': 2 blocks"),
        "got {:?}",
        app.toasts
            .iter()
            .map(|t| t.message.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn confirming_drops_undo_entries_of_the_pages_it_changed() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);
    // A snapshot of `beta` from before the merge: restored afterwards it
    // would render the merged blocks away and reconcile that as a delete.
    open(&mut app, "beta");
    app.snapshot_for_undo();
    open(&mut app, "alpha");
    assert!(!app.undo.is_empty(), "precondition: there is history");
    app.arm_page_merge("Beta");

    app.sidebar_confirm_delete().unwrap();

    assert!(
        app.undo.is_empty(),
        "a stale snapshot of the target would undo the merge"
    );
    assert!(app.redo.is_empty());
}

fn press(app: &mut App, c: char) {
    crate::input::handle_normal_key(app, KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
        .unwrap();
}

#[test]
fn y_on_the_prompt_runs_the_merge() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);
    app.arm_page_merge("Beta");

    press(&mut app, 'y');

    assert_eq!(app.current_slug(), "beta");
    assert!(!app.workspace_root.join("pages/alpha.md").exists());
}

#[test]
fn any_other_key_cancels_and_changes_nothing() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);
    app.arm_page_merge("Beta");

    press(&mut app, 'x');

    assert!(
        app.pending_sidebar_delete.is_none(),
        "the prompt is dropped"
    );
    let alpha = outl_actions::find_by_slug(&app.workspace, "alpha").unwrap();
    assert!(!outl_actions::is_trashed(&app.workspace, alpha));
    assert!(app.workspace_root.join("pages/alpha.md").exists());
    assert_eq!(app.current_slug(), "alpha");
}

#[test]
fn the_summary_says_only_what_happened() {
    assert_eq!(count(1, "block"), "1 block");
    assert_eq!(count(3, "property conflict"), "3 property conflicts");
}
