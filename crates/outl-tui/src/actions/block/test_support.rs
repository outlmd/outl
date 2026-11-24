//! Real two-page workspaces for the backlink tests in this module.
//!
//! These build a workspace, reconcile both `.md` files and run the real
//! `build_backlink_index_from_disk` rather than poking `App` state: a
//! backlink op resolves its target through `backlinks_for_current`, so
//! a test that fakes that list has not exercised the path under test.
//!
//! It lives in its own file because more than one module needs it: a
//! fixture inside a `#[cfg(test)] mod tests` cannot be reached from a
//! sibling. It started inside `backlink_edit`'s test module, which is
//! why it moved out — that module is gone, along with the `.md`-first
//! mechanism it held.

use crate::state::{App, Focus, View};
use outl_core::hlc::HlcGenerator;
use outl_core::{ActorId, Workspace};
use outl_md::reconcile::reconcile_md;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// `pages/target.md` is on screen; `pages/source.md` mentions it, so it
/// shows up in the backlinks panel below. Returns the app, the temp dir
/// (keep it alive) and the source page's path.
pub(super) fn workspace_with_a_backlink() -> (App, TempDir, PathBuf) {
    let (app, dir) = open_pages("- target body\n", "- mentions [[target]]\n- second line\n");
    let source = dir.path().join("pages").join("source.md");
    (app, dir, source)
}

/// `pages/target.md` cites **itself**, so the backlink's source page is
/// the page on screen. `backlinks_for_page` includes self-references
/// deliberately, and `source.md` mentions nothing here so the only row
/// is the self-citation. Returns the app, the temp dir and the open
/// page's own path — which is also the source page's.
/// The citing block is the **second** sibling so an indent on it has
/// somewhere to go — a cross-page structural op on the first sibling is
/// a no-op and would pin nothing.
pub(super) fn workspace_with_a_self_backlink() -> (App, TempDir, PathBuf) {
    let (app, dir) = open_pages("- target body\n- mentions [[target]]\n", "- unrelated\n");
    let target = dir.path().join("pages").join("target.md");
    (app, dir, target)
}

/// Two citing blocks on one source page, the first of which has a
/// child. The child mentions nothing, so it is not a backlink of its
/// own — it is there so a cursor that walked into a backlink's subtree
/// would be visible as a landing with no `[[target]]` in it.
pub(super) fn workspace_with_two_backlinks() -> (App, TempDir) {
    open_pages(
        "- target body\n",
        "- mentions [[target]]\n  - a child of the first citing block\n- second [[target]] mention\n",
    )
}

/// `pages/target.md` is on screen and **two different pages** cite it,
/// one row each.
///
/// The cross-page shape is what makes a *positional* re-entry visible.
/// A commit that drops one row leaves `idx` naming a citing block on
/// the **other** page, so a caret that comes back by position comes
/// back in a different file — which the single-source-page fixtures
/// cannot show, because there `idx` and the block agree on one page
/// whatever the clamp does.
///
/// `b`'s citing block is the **second** sibling so an indent on it has
/// somewhere to go; `a` holds the row the broken re-entry lands on.
pub(super) fn workspace_with_backlinks_on_two_pages() -> (App, TempDir) {
    open_slugs(
        &[
            ("target", "- target body\n"),
            ("a", "- mentions [[target]] from a\n"),
            ("b", "- first line on b\n- mentions [[target]] from b\n"),
        ],
        "target",
    )
}

/// Open `pages/target.md` with both page bodies written to disk, the
/// backlink index built from disk, and the cursor parked on the first
/// backlink.
fn open_pages(target_md: &str, source_md: &str) -> (App, TempDir) {
    open_slugs(&[("target", target_md), ("source", source_md)], "target")
}

/// [`open_pages`] over any number of pages, because more than two is
/// what a cross-page cursor defect needs.
///
/// One builder rather than a second copy per page count: the reconcile
/// order, the from-disk index and the parked cursor are the parts a
/// test must not fake, and a fixture that drifts on any of them stops
/// exercising the path under test.
fn open_slugs(pages: &[(&str, &str)], open: &str) -> (App, TempDir) {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("pages")).unwrap();
    let path_for = |slug: &str| dir.path().join("pages").join(format!("{slug}.md"));
    for (slug, md) in pages {
        fs::write(path_for(slug), md).unwrap();
    }

    let actor = ActorId::new();
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let hlc = HlcGenerator::new(actor);
    for (slug, _) in pages {
        reconcile_md(&mut ws, &hlc, &path_for(slug), None).unwrap();
    }

    let slugs: Vec<&str> = pages.iter().map(|(slug, _)| *slug).collect();
    let index = build_index(&ws, dir.path(), &slugs);

    let mut app = App::new_for_tests(
        dir.path().to_path_buf(),
        ws,
        actor,
        crate::theme::default_theme(),
        false,
    )
    .unwrap();
    *app.backlink_index.borrow_mut() = Some(index);
    app.view = View::Page(path_for(open));
    app.load_current_no_autorun();
    assert!(
        !app.backlinks_for_current().is_empty(),
        "setup: the open page must have a backlink"
    );
    app.focus = Focus::Backlink { idx: 0, node: None };
    (app, dir)
}

fn build_index(ws: &Workspace, root: &Path, slugs: &[&str]) -> outl_actions::BacklinkIndex {
    let metas: Vec<_> = slugs
        .iter()
        .filter_map(|s| outl_actions::find_by_slug(ws, s))
        .filter_map(|id| outl_actions::page_meta(ws, id))
        .collect();
    assert_eq!(
        metas.len(),
        slugs.len(),
        "setup: every page must be in the workspace"
    );
    outl_actions::build_backlink_index_from_disk(&metas, root)
}
