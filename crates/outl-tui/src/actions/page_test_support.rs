//! One `App` fixture for the page-level action tests.
//!
//! `/rename`, `/merge` and the `title::` chip are three doors onto one
//! operation, and their tests all need the same two projected pages
//! with a mention crossing between them. The fixture lived inside
//! `page_rename/tests.rs`, which is what kept the `title::` tests
//! there too — beside the rename rather than beside
//! [`crate::actions::property_write`], the module whose single owner
//! of that fork they actually exercise.
//!
//! `#[cfg(test)]` at the declaration in `actions/mod.rs`, so none of
//! this is compiled into the binary.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use outl_actions::{
    append_block, apply_page_md_with_sidecar_guarded, open_or_create_page, PageKind,
};
use outl_core::{ActorId, Workspace};
use tempfile::TempDir;

use crate::state::App;

/// An empty workspace in a temp dir, with the default theme.
///
/// The `TempDir` comes back with it and must be held: dropping it
/// deletes the workspace out from under the `App`.
pub(crate) fn fresh_app() -> (App, TempDir) {
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

/// Two projected pages: `alpha` holds `one`, `beta` holds a block that
/// cites `[[Alpha]]`. The view is left on `alpha`.
///
/// The mention is the point: every operation here has to respell it, so
/// a fixture without a cross-page reference would pass whether or not
/// the link rewrite ran.
pub(crate) fn seed(app: &mut App) {
    let hlc = app.hlc.clone();
    for (slug, title, blocks) in [
        ("alpha", "Alpha", ["one"].as_slice()),
        ("beta", "Beta", ["see [[Alpha]]"].as_slice()),
    ] {
        let ws = &mut app.workspace;
        let page = open_or_create_page(ws, &hlc, slug, title, PageKind::Page).unwrap();
        for &text in blocks {
            append_block(ws, &hlc, Some(page), Some(text)).unwrap();
        }
        // Step 5 of `commit_page` alone, deliberately: this is a
        // fixture reaching a starting state, not a mutation a user
        // made. There is no undo stack to snapshot into, no backlink
        // cache built yet, and no peer to announce to — and a test
        // that began with history or a warm cache would be asserting
        // against a state the fixture invented.
        apply_page_md_with_sidecar_guarded(&app.workspace, &app.workspace_root, page).unwrap();
    }
    open(app, "alpha");
}

/// Open a page by slug, through the path the client really uses.
fn open(app: &mut App, slug: &str) {
    let path = app.workspace_root.join(format!("pages/{slug}.md"));
    app.open_path(path).unwrap();
}

/// Every toast message currently on screen.
pub(crate) fn toasts(app: &App) -> Vec<String> {
    app.toasts.iter().map(|t| t.message.clone()).collect()
}

/// Whether some toast contains `needle`.
pub(crate) fn toasted(app: &App, needle: &str) -> bool {
    app.toasts.iter().any(|t| t.message.contains(needle))
}

/// One Normal-mode keystroke, through the real dispatcher.
pub(crate) fn press(app: &mut App, c: char) {
    crate::input::handle_normal_key(app, KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
        .unwrap();
}
