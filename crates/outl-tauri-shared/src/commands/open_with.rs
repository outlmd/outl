//! "Open With → outl": importing an external `.md` / `.txt` file.
//!
//! The policy — where the page lands, what happens when the same file
//! is opened twice, which extensions are accepted — is
//! [`outl_actions::open_with`]'s and only its. This module is the
//! adapter: normalise the path the OS handed us, read the file off the
//! filesystem *before* taking the workspace lock (the bytes are not
//! workspace state), then run the create + import as one
//! [`crate::helpers::finish_in_page`] commit.
//!
//! The commit needs the page's `NodeId` before the mutation runs, and
//! an unimported file has no page yet. That works because page ids are
//! derived from the slug — [`OpenWithTarget::page_id`] answers for a
//! page that does not exist. `the_page_id_is_known_before_the_page_exists`
//! in `outl-actions` pins it.

use std::path::Path;

use outl_actions::open_with::{import_into, read_source, resolve_target, OpenWithTarget};

use crate::commands::page::open_page_by_slug;
use crate::helpers::{finish_in_page, normalize_picker_path, with_ws};
use crate::host::AppHost;
use crate::state::PageView;

/// Import `source_path` into the workspace and return the page to show.
///
/// A file already imported under this exact path is **not** imported
/// again — the command navigates to the page it produced the first
/// time. Re-importing would duplicate every block, and overwriting
/// would delete whatever the user wrote on the page afterwards; see
/// [`outl_actions::open_with`] for the full reasoning.
pub fn open_external_file<S: AppHost>(state: &S, source_path: String) -> Result<PageView, String> {
    let normalized = normalize_picker_path(&source_path);
    let path = Path::new(&normalized);

    let target = with_ws(state, |ws| {
        resolve_target(ws, path).map_err(|e| e.to_string())
    })?;

    // Already imported from this exact path: hand off to the ordinary
    // page-open command rather than re-deriving its body here. Opening
    // an existing page is one job with one owner — the ahead-of-log
    // re-projection it runs first is the part a local copy silently
    // drops, and that is the banner telling the user a page stopped
    // syncing.
    //
    // This returns before the file is read, deliberately. Navigating
    // to an existing page does not need the bytes, so a file that has
    // since grown past the cap or stopped being UTF-8 must not stop
    // the user reaching the page it already produced.
    if matches!(target, OpenWithTarget::Existing { .. }) {
        return open_page_by_slug(state, target.slug().to_string());
    }

    // Read outside the lock: the file lives outside the workspace and
    // its bytes are never workspace state, so there is no reason to
    // hold every other command out while the disk answers.
    let contents = read_source(path).map_err(|e| e.to_string())?;
    finish_in_page(state, target.page_id(), |ws| {
        import_into(ws, state.hlc(), &target, &contents).map(|_| ())
    })
}
