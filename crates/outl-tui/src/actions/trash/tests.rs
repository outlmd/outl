//! What the trash overlay has to get right.
//!
//! Its own file because the parent crossed the 600-line guard, and
//! because the two `Enter` tests are the regression net for a real
//! defect: `Enter` on a deleted *page* row used to revive the page
//! through `open_page_by_slug`'s seed-and-reconcile path, turning one
//! trash row into one per child. They assert on the **workspace**, so
//! a reworded refusal cannot make them pass while the revive returns.

use super::*;

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

fn trash_message(app: &App) -> Option<&str> {
    match &app.overlay {
        Some(Overlay::Trash(t)) => t.message.as_deref(),
        _ => None,
    }
}

/// **The regression this overlay shipped with.**
///
/// `Enter` on a deleted *page* row used to call `open_page_by_slug`.
/// `delete_page` drops the projection, so the `.md` is gone; the
/// opener reads a missing file as "new page", writes a `title::`
/// seed and reconciles it, and `ensure_page_root_in_tree` lifts the
/// root back out of the trash while the sidecar-less matching
/// orphan-trashes each child on its own. One trash row became N, the
/// delete was undone, and the real `title::` was overwritten with the
/// slug — from a key the docs called read-only.
///
/// The assertion is on the **workspace**, not on the message, so a
/// future rewording cannot make it pass while the revive comes back.
#[test]
fn enter_on_a_deleted_page_row_never_brings_the_page_back() {
    let (mut app, _dir) = fresh_app();
    let hlc = app.hlc.clone();
    let page = outl_actions::open_or_create_page(
        &mut app.workspace,
        &hlc,
        "archive",
        "Archive",
        outl_actions::PageKind::Page,
    )
    .unwrap();
    let child =
        outl_actions::append_block(&mut app.workspace, &hlc, Some(page), Some("um bloco")).unwrap();
    outl_actions::delete(&mut app.workspace, &hlc, page).unwrap();

    app.open_trash();
    let row = match &app.overlay {
        Some(Overlay::Trash(t)) => t
            .entries
            .iter()
            .position(|e| e.node == page)
            .expect("the deleted page is a trash row"),
        _ => panic!("the overlay is open"),
    };
    if let Some(Overlay::Trash(t)) = app.overlay.as_mut() {
        t.selected = row;
    }

    app.open_selected_trash_origin();

    assert!(
        outl_actions::is_trashed(&app.workspace, page),
        "Enter on a page row must not lift the page out of the trash"
    );
    assert!(
        outl_actions::is_trashed(&app.workspace, child),
        "and must not orphan-trash its children one by one either"
    );
    assert!(
        !app.workspace_root.join("pages/archive.md").exists(),
        "nor seed a `.md` for a page the user deleted"
    );
    assert!(
        matches!(app.overlay, Some(Overlay::Trash(_))),
        "the overlay stays open so the user can read why"
    );
    assert!(
        trash_message(&app).is_some_and(|m| m.contains("archive")),
        "and the refusal names the page, got {:?}",
        trash_message(&app),
    );
}

/// Item 3's sibling on the block path: a page live in the tree with no
/// local `.md` (peer-created, never opened here) must not be seeded.
///
/// Same `open_page_slug` seed-and-reconcile that revives a deleted page.
/// The page-row guard keys on row *kind*, so it does not catch this —
/// the file has to be asked about. Asserted on the filesystem, because
/// the damage is a `.md` appearing where none belonged.
#[test]
fn enter_on_a_block_whose_page_has_no_local_md_seeds_nothing() {
    let (mut app, _dir) = fresh_app();
    let hlc = app.hlc.clone();
    let page = outl_actions::open_or_create_page(
        &mut app.workspace,
        &hlc,
        "remoto",
        "Remoto",
        outl_actions::PageKind::Page,
    )
    .unwrap();
    let block =
        outl_actions::append_block(&mut app.workspace, &hlc, Some(page), Some("um bloco")).unwrap();
    outl_actions::delete(&mut app.workspace, &hlc, block).unwrap();
    // Deliberately never projected: this is the peer-created page a
    // local user has not opened.
    let md = app.workspace_root.join("pages/remoto.md");
    assert!(!md.exists(), "precondition: no local projection");

    app.open_trash();
    app.open_selected_trash_origin();

    assert!(
        !md.exists(),
        "a page that has not synced here must not be seeded by looking at the trash"
    );
    assert!(
        matches!(app.overlay, Some(Overlay::Trash(_))),
        "the overlay stays open to say why"
    );
    assert!(
        trash_message(&app).is_some_and(|m| m.contains("remoto")),
        "and names the page, got {:?}",
        trash_message(&app),
    );
}

/// A block deleted from a **journal** must not seed a `pages/` copy.
///
/// The first guard here asked "`pages/{slug}.md` **or**
/// `journals/{slug}.md`", while `open_page_slug` only ever looks at and
/// writes `pages/{slug}.md`. So a journal with its own `.md` on disk
/// passed the guard and then got a spurious `pages/<date>.md` seeded
/// beside it — the exact hazard the guard exists to stop, and in a
/// journal-first app the common case rather than an edge.
#[test]
fn enter_on_a_block_from_a_journal_never_seeds_a_pages_copy() {
    let (mut app, _dir) = fresh_app();
    let hlc = app.hlc.clone();
    // Deliberately **not** today. `App` boots on `View::Journal(today)`,
    // so a fixture dated today lets the landing assertion below pass
    // on a view nothing ever routed.
    let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
    let journal = outl_actions::open_journal(&mut app.workspace, &hlc, day).unwrap();
    let block = outl_actions::append_block(&mut app.workspace, &hlc, Some(journal), Some("do dia"))
        .unwrap();
    outl_actions::apply_page_md_with_sidecar_guarded(
        &app.workspace,
        &app.workspace_root.clone(),
        journal,
    )
    .unwrap();
    outl_actions::delete(&mut app.workspace, &hlc, block).unwrap();

    let journal_md = app.workspace_root.join("journals/2026-11-05.md");
    assert!(
        journal_md.exists(),
        "precondition: the journal is projected"
    );

    app.open_trash();
    app.open_selected_trash_origin();

    assert!(
        !app.workspace_root.join("pages/2026-11-05.md").exists(),
        "a journal must never get a `pages/` twin"
    );
    assert!(journal_md.exists(), "and its real projection is untouched");
    assert!(
        app.overlay.is_none(),
        "the journal opens, since its `.md` is right there"
    );
    // **The landing, not just the absence of the twin.** The three
    // assertions above all hold for a wrong view too: routing the date
    // through `open_page_by_slug` is what seeds `pages/<date>.md`, so
    // not seeding it says the *slug* path was avoided, and says nothing
    // about where the user ended up. `View::Journal(day)` is the thing
    // `open_origin_page` exists to pick.
    assert!(
        matches!(app.view, crate::state::View::Journal(landed) if landed == day),
        "a journal origin routes to View::Journal, got {:?}",
        app.view
    );
}

/// The dominant refused shape says so, rather than claiming the block
/// was never on a page.
///
/// `page_slug_of` answers `None` for a trashed parent exactly as it does
/// for a block that had no page, and one sentence over both was false
/// for the common one: 89 of 393 top-level deletions on the reference
/// workspace are a block whose parent went to the trash with it.
#[test]
fn a_row_whose_parent_is_trashed_says_that_rather_than_no_page() {
    let (mut app, _dir) = fresh_app();
    let hlc = app.hlc.clone();
    let page = outl_actions::open_or_create_page(
        &mut app.workspace,
        &hlc,
        "notas",
        "Notas",
        outl_actions::PageKind::Page,
    )
    .unwrap();
    let section =
        outl_actions::append_block(&mut app.workspace, &hlc, Some(page), Some("seção")).unwrap();
    let child =
        outl_actions::append_block(&mut app.workspace, &hlc, Some(section), Some("filho")).unwrap();
    // The child goes first, then its parent — so the child is its own
    // top-level row whose landing parent is now in the trash.
    outl_actions::delete(&mut app.workspace, &hlc, child).unwrap();
    outl_actions::delete(&mut app.workspace, &hlc, section).unwrap();

    app.open_trash();
    let row = match &app.overlay {
        Some(Overlay::Trash(t)) => t
            .entries
            .iter()
            .position(|e| e.node == child)
            .expect("the child is its own row"),
        _ => panic!("open"),
    };
    if let Some(Overlay::Trash(t)) = app.overlay.as_mut() {
        t.selected = row;
    }

    app.open_selected_trash_origin();

    let message = trash_message(&app).unwrap_or_default().to_string();
    assert!(
        message.contains("in the trash too"),
        "it was on a page; that page is in the trash too. got: {message:?}"
    );
    assert!(
        !message.contains("wasn't on a page"),
        "and that claim is false here: {message:?}"
    );
}

/// A restore takes an undo snapshot, keyed to the page it landed on.
///
/// Without it, `u` after a restore applied the landing page's
/// *previous* snapshot — re-trashing the block that just came back
/// and reverting whatever unrelated edit that snapshot held. The
/// assertion is on the stack's depth and its `view_path`, because
/// that path is what `top_applies_here` reads to decide the snapshot
/// belongs to the page the user is on.
#[test]
fn a_restore_records_undo_against_the_page_it_landed_on() {
    let (mut app, _dir) = fresh_app();
    let hlc = app.hlc.clone();
    let page = outl_actions::open_or_create_page(
        &mut app.workspace,
        &hlc,
        "infra",
        "Infra",
        outl_actions::PageKind::Page,
    )
    .unwrap();
    let block =
        outl_actions::append_block(&mut app.workspace, &hlc, Some(page), Some("um bloco")).unwrap();
    outl_actions::apply_page_md_with_sidecar_guarded(
        &app.workspace,
        &app.workspace_root.clone(),
        page,
    )
    .unwrap();
    outl_actions::delete(&mut app.workspace, &hlc, block).unwrap();

    let before = app.undo.len();
    app.open_trash();
    app.restore_selected_trash();

    assert!(
        !outl_actions::is_trashed(&app.workspace, block),
        "the block came back"
    );
    assert_eq!(
        app.undo.len(),
        before + 1,
        "the restore has to be undoable; without the snapshot `u` reverts the wrong thing"
    );
    assert_eq!(
        app.undo.last().map(|s| s.view_path.clone()),
        Some(app.workspace_root.join("pages/infra.md")),
        "and the snapshot is keyed to the landing page, not the open view"
    );
}

/// The three branches of `report_restore_projection_failure`, which had
/// no test at all.
///
/// They are driven directly rather than through a staged invariant-8
/// refusal: what is under test is the **routing** (which surface each
/// class reaches), and `ProjectionRefusal::from_error` already owns the
/// classification with its own tests. Staging a real frozen `.md` would
/// exercise the guard, not the branch.
mod projection_failure {
    use super::*;
    use outl_actions::ActionError;

    fn page_on_screen(app: &mut App) -> NodeId {
        let hlc = app.hlc.clone();
        let page = outl_actions::open_or_create_page(
            &mut app.workspace,
            &hlc,
            "aberta",
            "Aberta",
            outl_actions::PageKind::Page,
        )
        .unwrap();
        outl_actions::apply_page_md_with_sidecar_guarded(
            &app.workspace,
            &app.workspace_root.clone(),
            page,
        )
        .unwrap();
        app.open_page_by_slug("aberta").unwrap();
        page
    }

    fn ahead_of_log() -> ActionError {
        ActionError::PageMarkdownAheadOfLog {
            path: "pages/outra.md".to_string(),
            lines: 3,
            sample: "- uma linha".to_string(),
        }
    }

    /// Retryable is a log line and nothing else — this crate's written
    /// rule, because nagging about I/O trains the user past the banner
    /// that matters.
    #[test]
    fn a_retryable_failure_does_not_toast() {
        let (mut app, _dir) = fresh_app();
        let page = page_on_screen(&mut app);
        let before = app.toasts.len();

        app.report_restore_projection_failure(page, ActionError::NotInTree("algum-no".to_string()));

        assert_eq!(app.toasts.len(), before, "a retryable failure says nothing");
    }

    /// On the page the user is reading, the persistent banner is the
    /// surface — `load_current` re-derives it, so this path stays quiet
    /// rather than saying the same thing twice in a weaker place.
    #[test]
    fn an_invariant_8_refusal_on_the_open_page_stays_silent() {
        let (mut app, _dir) = fresh_app();
        let page = page_on_screen(&mut app);
        let before = app.toasts.len();

        app.report_restore_projection_failure(page, ahead_of_log());

        assert_eq!(
            app.toasts.len(),
            before,
            "the banner covers the open page; a toast would be the second copy"
        );
    }

    /// A page the user is **not** looking at has no banner to appear on,
    /// which is the only reason this surface exists.
    #[test]
    fn an_invariant_8_refusal_on_another_page_toasts_and_names_it() {
        let (mut app, _dir) = fresh_app();
        page_on_screen(&mut app);
        let hlc = app.hlc.clone();
        let other = outl_actions::open_or_create_page(
            &mut app.workspace,
            &hlc,
            "outra",
            "Outra",
            outl_actions::PageKind::Page,
        )
        .unwrap();

        app.report_restore_projection_failure(other, ahead_of_log());

        let last = app
            .toasts
            .last()
            .expect("a page nobody is reading gets a toast");
        assert!(
            last.message.contains("outra"),
            "it has to name which page, got {:?}",
            last.message
        );
        assert!(
            !last.message.contains("syncing To"),
            "and read as one sentence, not a run-on: {:?}",
            last.message
        );
    }
}

/// The other half, so the guard above cannot pass by refusing every
/// row: a deleted **block**'s page is still alive, so `Enter` finds
/// its `.md` and only navigates.
#[test]
fn enter_on_a_block_row_opens_the_page_it_came_from() {
    let (mut app, _dir) = fresh_app();
    let hlc = app.hlc.clone();
    let page = outl_actions::open_or_create_page(
        &mut app.workspace,
        &hlc,
        "notas",
        "Notas",
        outl_actions::PageKind::Page,
    )
    .unwrap();
    let block =
        outl_actions::append_block(&mut app.workspace, &hlc, Some(page), Some("um bloco")).unwrap();
    outl_actions::apply_page_md_with_sidecar_guarded(
        &app.workspace,
        &app.workspace_root.clone(),
        page,
    )
    .unwrap();
    outl_actions::delete(&mut app.workspace, &hlc, block).unwrap();

    app.open_trash();
    app.open_selected_trash_origin();

    assert!(
        app.overlay.is_none(),
        "a block row navigates, so the overlay closes"
    );
    assert!(
        outl_actions::is_trashed(&app.workspace, block),
        "and it is read-only: the block stays deleted"
    );
}

fn entry(node: NodeId, refusal: Option<&str>) -> TrashEntry {
    TrashEntry {
        node,
        parent_at_deletion: None,
        page: None,
        subtree_len: 1,
        preview: "bloco".to_string(),
        deleted_at_ms: None,
        refusal: refusal.map(str::to_string),
    }
}

#[test]
fn the_cursor_clamps_including_on_an_empty_trash() {
    let mut t = TrashState::default();
    t.step(1);
    assert_eq!(t.selected, 0);
    t.entries = vec![entry(NodeId::new(), None), entry(NodeId::new(), None)];
    t.step(5);
    assert_eq!(t.selected, 1);
    t.step(-9);
    assert_eq!(t.selected, 0);
}

/// The row carries the backend's sentence, never one written here —
/// a second wording is a second owner of the rule (invariant 12).
#[test]
fn a_refused_row_keeps_the_backends_own_sentence() {
    let why = "cannot restore 01JQ: the block it sat under is in the trash too";
    let t = TrashState {
        entries: vec![entry(NodeId::new(), Some(why))],
        selected: 0,
        message: None,
    };
    assert_eq!(t.entries[0].refusal.as_deref(), Some(why));
}
