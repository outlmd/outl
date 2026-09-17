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
use outl_actions::{apply_page_md_with_sidecar_guarded, ActionError};
use outl_core::id::NodeId;

use crate::commands::page::open_page_by_slug;
use crate::helpers::{finish_in_page_with, normalize_picker_path, with_ws, with_ws_mut};
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
    let (outcome, view) = finish_in_page_with(state, target.page_id(), |ws| {
        import_into(ws, state.hlc(), &target, &contents)
    })?;

    // An import dirties two pages: the one it created and today's
    // journal, which gained the `[[ref]]`. `commit_page` is scoped to
    // one (issue #264), so the journal is projected here, the same
    // shape `move_block_after` uses for a cross-page move.
    if let Some(journal) = outcome.journal {
        // The journal gained a block outside its own `commit_page`, so
        // any undo snapshot it holds predates that block. Restoring one
        // would reconcile the link away: the user undoes what they
        // typed and silently loses the entry too. Same rule
        // `invalidate_changed_history` applies after a peer reload
        // changes a page from outside.
        if let Some(history) = state.history() {
            history.lock().remove(&journal);
        }
        project_journal(state, journal);
    }
    Ok(view)
}

/// Write the journal's `.md` + sidecar after an import linked into it.
///
/// Best-effort on purpose. The link is already in the op log, which is
/// the source of truth, so a refused projection leaves the journal's
/// `.md` briefly behind rather than losing the entry — and the guard
/// exists precisely so this write cannot delete content the log never
/// saw (invariant 8).
///
/// With a `ProjectionWriter` (both GUI clients) the worker reports a
/// refusal on the event bridge itself, which is what reaches the user;
/// the reply renders the *imported* page, so the journal could not
/// carry its own notice anyway. A host without one leaves it in a log
/// line, the same asymmetry `move_block_after` already has for the page
/// a block left behind.
fn project_journal<S: AppHost>(state: &S, journal: NodeId) {
    if let Some(writer) = state.projection_writer() {
        writer.queue(journal);
        return;
    }
    let Ok(root) = state.storage_root() else {
        return;
    };
    let failed: Option<ActionError> = with_ws_mut(state, |ws| {
        Ok(apply_page_md_with_sidecar_guarded(ws, &root, journal).err())
    })
    .ok()
    .flatten();
    if let Some(e) = failed {
        tracing::warn!("open with: journal md+sidecar sync skipped: {e}");
    }
}
