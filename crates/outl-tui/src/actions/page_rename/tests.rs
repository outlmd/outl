//! What `/rename` has to get right: refuse without writing, move the
//! file *and* everything in this client that remembers it by name, keep
//! the old `.md` when the new one could not be written, and offer the
//! merge instead of refusing a taken name.

use super::*;

use outl_actions::link_rewrite::SkippedRef;

use crate::actions::page_commit::rename_summary;
use crate::actions::page_test_support::{fresh_app, press, seed, toasted, toasts};
use crate::state::{LastSearch, SearchHit};

// ---------------------------------------------------------------------
// the happy path
// ---------------------------------------------------------------------

#[test]
fn renaming_moves_the_file_and_the_view_with_it() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);

    app.rename_current_page("Gamma").unwrap();

    assert!(
        app.workspace_root.join("pages/gamma.md").exists(),
        "the page's new file is written"
    );
    assert!(
        !app.workspace_root.join("pages/alpha.md").exists(),
        "the old projection goes once the new one landed"
    );
    assert_eq!(
        app.current_slug(),
        "gamma",
        "the view follows, or the next save recreates pages/alpha.md"
    );
    let meta = outl_actions::page_meta(
        &app.workspace,
        outl_actions::find_by_slug(&app.workspace, "gamma").expect("the page is live"),
    )
    .unwrap();
    assert_eq!(meta.title, "Gamma");
}

#[test]
fn the_slash_command_reaches_the_rename() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);

    let registry = app.command_registry.clone();
    registry.dispatch(&mut app, "rename Gamma").unwrap();

    assert_eq!(app.current_slug(), "gamma");
}

#[test]
fn renaming_respells_every_mention() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);

    app.rename_current_page("Gamma").unwrap();

    let beta = std::fs::read_to_string(app.workspace_root.join("pages/beta.md")).unwrap();
    assert!(
        beta.contains("[[Gamma]]") && !beta.contains("[[Alpha]]"),
        "the citing page is re-projected too:\n{beta}"
    );
    assert!(
        toasted(&app, "renamed 'Alpha' to 'Gamma'"),
        "got {:?}",
        toasts(&app)
    );
}

#[test]
fn an_edit_on_screen_is_flushed_before_the_rename_reads_the_log() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);
    // One keystroke's worth of in-memory AST that the op log has not
    // seen: without the flush it is rendered to `pages/alpha.md` after
    // the rename, which recreates the page under its old slug.
    app.page.blocks[0].text = "one edited".into();
    app.save();

    app.rename_current_page("Gamma").unwrap();

    assert!(!app.workspace_root.join("pages/alpha.md").exists());
    let gamma = std::fs::read_to_string(app.workspace_root.join("pages/gamma.md")).unwrap();
    assert!(gamma.contains("one edited"), "the edit survives:\n{gamma}");
}

#[test]
fn a_rename_that_only_changes_the_title_keeps_the_file() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);

    // `slugify` folds the `!` away, so the slug does not move.
    app.rename_current_page("Alpha!").unwrap();

    assert!(app.workspace_root.join("pages/alpha.md").exists());
    assert_eq!(app.current_slug(), "alpha");
    let meta = outl_actions::page_meta(
        &app.workspace,
        outl_actions::find_by_slug(&app.workspace, "alpha").unwrap(),
    )
    .unwrap();
    assert_eq!(meta.title, "Alpha!");
}

// ---------------------------------------------------------------------
// refusals, none of which may write
// ---------------------------------------------------------------------

#[test]
fn an_empty_argument_is_refused_and_writes_nothing() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);
    let before = app.workspace.log().len();

    app.rename_current_page("   ").unwrap();

    assert!(app.status.contains("usage: rename"), "got {:?}", app.status);
    assert_eq!(app.workspace.log().len(), before);
    assert_eq!(app.current_slug(), "alpha");
}

#[test]
fn a_journal_is_never_renamed() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);
    app.go_today().unwrap();
    let before = app.workspace.log().len();

    app.rename_current_page("Gamma").unwrap();

    assert!(toasted(&app, "can't rename a journal page"));
    assert_eq!(app.workspace.log().len(), before);
}

#[test]
fn a_name_that_folds_to_nothing_takes_slugifys_fallback_like_the_gui_does() {
    // Not a refusal: `slugify` always answers with a legal slug, and the
    // Tauri command layer renames on the same answer. A TUI-only guard
    // here would be this client refusing what the GUIs accept.
    let (mut app, _dir) = fresh_app();
    seed(&mut app);

    app.rename_current_page("...").unwrap();

    assert_eq!(app.current_slug(), outl_md::slug::slugify("..."));
    let meta = outl_actions::page_meta(
        &app.workspace,
        outl_actions::find_by_slug(&app.workspace, &app.current_slug()).unwrap(),
    )
    .unwrap();
    assert_eq!(meta.title, "...", "the title is what the user typed");
}

// ---------------------------------------------------------------------
// the collision
// ---------------------------------------------------------------------

#[test]
fn a_taken_name_arms_the_merge_prompt_and_writes_nothing() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);
    let before = app.workspace.log().len();

    app.rename_current_page("Beta").unwrap();

    let pending = app
        .pending_page_action
        .as_ref()
        .expect("a prompt is armed instead of a refusal");
    assert_eq!(pending.slug, "alpha");
    assert_eq!(
        pending.action,
        PageAction::RenameCollision {
            new_slug: "beta".into(),
            new_title: "Beta".into(),
        }
    );
    assert!(
        app.status.contains("already has that name") && app.status.ends_with("y/n"),
        "the prompt names the collision and asks, got {:?}",
        app.status
    );
    assert_eq!(
        app.workspace.log().len(),
        before,
        "`PageExists` is refused before any batch opens"
    );
    assert!(app.workspace_root.join("pages/alpha.md").exists());
    assert!(app.workspace_root.join("pages/beta.md").exists());
}

#[test]
fn y_on_the_collision_prompt_merges_into_the_page_that_has_the_name() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);
    app.rename_current_page("Beta").unwrap();

    press(&mut app, 'y');

    let alpha = outl_actions::find_by_slug(&app.workspace, "alpha");
    assert!(
        alpha.is_none_or(|id| outl_actions::is_trashed(&app.workspace, id)),
        "the renamed page is the merge's source"
    );
    let beta = std::fs::read_to_string(app.workspace_root.join("pages/beta.md")).unwrap();
    assert!(beta.contains("one"), "alpha's block moved over:\n{beta}");
    assert!(!app.workspace_root.join("pages/alpha.md").exists());
    assert_eq!(app.current_slug(), "beta");
    assert!(app.pending_page_action.is_none());
    assert!(
        toasted(&app, "merged 'Alpha' into 'Beta'"),
        "{:?}",
        toasts(&app)
    );
}

#[test]
fn n_on_the_collision_prompt_changes_nothing() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);
    app.rename_current_page("Beta").unwrap();
    let before = app.workspace.log().len();

    press(&mut app, 'n');

    assert!(app.pending_page_action.is_none(), "the prompt is dropped");
    assert_eq!(app.workspace.log().len(), before);
    assert_eq!(app.current_slug(), "alpha");
    assert!(app.workspace_root.join("pages/alpha.md").exists());
}

#[test]
fn a_collision_whose_owner_went_away_renames_after_all() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);
    app.rename_current_page("Beta").unwrap();
    // A peer's delete between the prompt and the `y`. `rename_or_merge`
    // re-runs the rename, which now succeeds — so `y` must not assume
    // it is confirming a merge.
    let meta = outl_actions::delete_page(&mut app.workspace, &app.hlc, "beta").unwrap();
    outl_actions::remove_page_projection(&app.workspace_root, &meta).unwrap();

    app.confirm_pending_page_action().unwrap();

    assert_eq!(app.current_slug(), "beta", "the rename landed");
    assert!(
        toasted(&app, "renamed 'Alpha' to 'Beta'"),
        "{:?}",
        toasts(&app)
    );
}

// ---------------------------------------------------------------------
// the four things a slug change breaks
// ---------------------------------------------------------------------

#[test]
fn the_rename_repoints_the_recent_path_that_would_undo_it() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);
    let old = app.workspace_root.join("pages/alpha.md");
    assert!(
        app.recent_paths.contains(&old),
        "precondition: opening the page remembered it"
    );

    app.rename_current_page("Gamma").unwrap();

    assert!(
        !app.recent_paths.contains(&old),
        "one `Enter` on this row recreates pages/alpha.md and reconciles \
         the rename away (RFC 0557 known limit 5)"
    );
    assert!(app
        .recent_paths
        .contains(&app.workspace_root.join("pages/gamma.md")));
}

#[test]
fn the_rename_repoints_a_persisted_search_hit() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);
    let old = app.workspace_root.join("pages/alpha.md");
    app.last_search = Some(LastSearch {
        hits: vec![SearchHit {
            page_label: "Alpha".into(),
            page_icon: None,
            md_path: old.clone(),
            snippet: "one".into(),
            block_path: vec![0],
            score: 1,
        }],
        cursor: 0,
    });

    app.rename_current_page("Gamma").unwrap();

    let hits = &app.last_search.as_ref().expect("the hit survives").hits;
    assert_eq!(hits[0].md_path, app.workspace_root.join("pages/gamma.md"));
}

#[test]
fn the_rename_forgets_the_history_of_the_file_it_moved() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);
    app.snapshot_for_undo();
    assert!(!app.undo.is_empty(), "precondition: there is history");

    app.rename_current_page("Gamma").unwrap();

    assert!(
        app.undo.is_empty(),
        "a snapshot keyed to pages/alpha.md is restorable the moment that \
         path comes back, and restoring it un-renames the page"
    );
    assert!(app.redo.is_empty());
}

#[test]
fn a_withheld_projection_keeps_the_old_file() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);
    // `pages/gamma.md` holds a line no op ever produced and has no
    // sidecar, so the guarded projection refuses to overwrite it (root
    // invariant 8). Nothing claims the slug in the tree, so the rename
    // itself is allowed.
    std::fs::write(
        app.workspace_root.join("pages/gamma.md"),
        "- a line the log never saw\n",
    )
    .unwrap();

    app.rename_current_page("Gamma").unwrap();

    assert!(
        app.workspace_root.join("pages/alpha.md").exists(),
        "the only file still holding the page must not be removed on a \
         refused write"
    );
    let gamma = std::fs::read_to_string(app.workspace_root.join("pages/gamma.md")).unwrap();
    assert!(
        gamma.contains("a line the log never saw"),
        "and the refused file is untouched:\n{gamma}"
    );
    assert!(
        toasts(&app).iter().any(|m| m.contains("gamma")),
        "the user hears about it: {:?}",
        toasts(&app)
    );
}

#[test]
fn a_withheld_projection_forgets_the_remembered_path_instead_of_repointing_it() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);
    std::fs::write(
        app.workspace_root.join("pages/gamma.md"),
        "- a line the log never saw\n",
    )
    .unwrap();
    let old = app.workspace_root.join("pages/alpha.md");
    let new = app.workspace_root.join("pages/gamma.md");
    assert!(
        app.recent_paths.contains(&old),
        "precondition: opening the page remembered it"
    );

    app.rename_current_page("Gamma").unwrap();

    // The old file survives a refused write, which is the whole point
    // of the guard — but the remembered row must not keep pointing at
    // it, because one `Enter` there reconciles `page-slug = alpha` back
    // onto the renamed page (known limit 5). The row follows the view
    // onto the page's new path instead, which is where
    // `land_on_page_or_today` put the user.
    assert!(
        app.workspace_root.join("pages/alpha.md").exists(),
        "precondition: the refused write left the old file alone"
    );
    assert!(
        !app.recent_paths.contains(&old),
        "the old path still undoes the rename: {:?}",
        app.recent_paths
    );
    assert!(
        app.recent_paths.contains(&new),
        "and the row follows the view onto the page's own path: {:?}",
        app.recent_paths
    );
}

// ---------------------------------------------------------------------
// the summary
// ---------------------------------------------------------------------

#[test]
fn the_summary_names_the_nested_page_links_left_unchanged() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);
    let mut report = outl_actions::rename_page(
        &mut app.workspace,
        &app.hlc,
        "alpha",
        "gamma",
        Some("Gamma"),
    )
    .unwrap();
    report.skipped_namespace_refs = vec![SkippedRef {
        block_id: "x".into(),
        name: "Alpha/Child".into(),
    }];

    let summary = rename_summary(&report);

    // The wording is `outl_actions::page_summary`'s, so the terminal,
    // the two apps and `outl page merge` name one fact one way. This
    // used to read `1 sub-page link left alone` here while the apps
    // read `1 namespace link left unchanged`.
    assert!(
        summary.contains("1 nested-page link left unchanged"),
        "a rename that drops this leaves [[old/child]] mentions the user \
         cannot see were not updated: {summary}"
    );
    assert!(summary.contains("links updated in 1 block"), "{summary}");
}

#[test]
fn a_rename_to_the_same_name_says_so_instead_of_claiming_work() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);

    app.rename_current_page("Alpha").unwrap();

    assert!(
        toasted(&app, "already called 'Alpha'"),
        "{:?}",
        toasts(&app)
    );
    assert!(app.workspace_root.join("pages/alpha.md").exists());
}

// ---------------------------------------------------------------------
// the shared prompt still does the other two things
// ---------------------------------------------------------------------

#[test]
fn the_prompt_still_deletes_and_still_merges() {
    let (mut app, _dir) = fresh_app();
    seed(&mut app);

    app.arm_page_merge("Beta");
    assert_eq!(
        app.pending_page_action.as_ref().map(|p| p.action.clone()),
        Some(PageAction::MergeInto {
            target: "beta".into()
        })
    );
    app.confirm_pending_page_action().unwrap();
    assert_eq!(app.current_slug(), "beta");

    app.delete_page_from_chord();
    assert_eq!(
        app.pending_page_action.as_ref().map(|p| p.action.clone()),
        Some(PageAction::Delete)
    );
    app.confirm_pending_page_action().unwrap();
    let beta = outl_actions::find_by_slug(&app.workspace, "beta");
    assert!(beta.is_none_or(|id| outl_actions::is_trashed(&app.workspace, id)));
}
