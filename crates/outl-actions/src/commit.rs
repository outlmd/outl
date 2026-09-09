//! What happens after a page mutation, in the one order that is correct.
//!
//! A mutation is never just the mutation. Five things have to happen
//! around it, and the order matters:
//!
//! 1. snapshot the page's `.md` **before** the change, for undo — and
//!    only keep it if the change actually altered the render, so a no-op
//!    command does not turn `Cmd+Z` into a visible nothing;
//! 2. run the mutation;
//! 3. drop the cached backlinks index, which the mutation may have
//!    invalidated;
//! 4. tell peers there are new ops, so a device pulls now instead of on
//!    the next catch-up sweep;
//! 5. project the page back to `.md` + sidecar.
//!
//! That sequence had one implementation, `outl-tauri-shared`'s
//! `finish_in_page_with`, generic over a trait that requires
//! `&Mutex<Option<Workspace>>` — the Tauri state shape. The TUI holds a
//! plain `Workspace` and the CLI holds one in a command context, so
//! neither could reach it, and both re-derived the parts they thought
//! applied. Repo-wide there were 34 direct calls to
//! `apply_page_md_with_sidecar_guarded` across 18 files: step 5 alone,
//! each one a caller that decided the other four did not apply to it.
//! Some of those decisions were right; none were written down, and
//! nothing failed when a new one was wrong.
//!
//! [`commit_page`] is that sequence with no lock, no `Arc` and no DTO in
//! its signature, so a `&mut Workspace` from anywhere can run it.
//!
//! # What a host provides
//!
//! [`CommitHooks`] has exactly one required method — the projection —
//! and defaults the rest to no-ops. A CLI command that does not cache
//! backlinks and has no undo stack implements one method and gets the
//! same ordering guarantees as the desktop.
//!
//! # What this is not
//!
//! It is **not** the Tauri `AppHost` trait relocated. That trait is
//! shaped by Tauri's managed state (`Mutex<Option<Workspace>>`,
//! `Arc<RuntimeRegistry>`) and moving it here would drag `parking_lot`
//! and the lock shape into the UI-agnostic crate — relocating the
//! problem rather than removing it, which is the question root
//! `CLAUDE.md` invariant 9 exists to ask. The lock stays in the client;
//! the *sequence* moves down.
//!
//! # Failure
//!
//! Only the mutation can fail the commit. A projection failure arrives
//! after the op log already holds the truth, so it is reported by the
//! host — [`CommitHooks::project`] returns nothing, and an implementation
//! that needs to surface the failure keeps it on itself. Aborting there
//! would report an error for a change that did happen.

use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::journal::render_page_md;

/// The per-client half of the commit sequence.
///
/// Every method except [`Self::project`] defaults to doing nothing, so a
/// host opts into the steps it actually has state for.
pub trait CommitHooks {
    /// Project the page to `.md` + sidecar (or queue that write).
    ///
    /// The only required method, and deliberately infallible from
    /// [`commit_page`]'s point of view: it runs after the op log already
    /// holds the mutation, so a failure here is something to *report*,
    /// not something that un-does the commit. An implementation that
    /// needs to surface it stores it on `self` and reads it back after.
    fn project(&mut self, workspace: &Workspace, page: NodeId);

    /// Whether this host keeps undo stacks.
    ///
    /// `false` (the default) skips the pre-mutation render entirely, so
    /// a client without undo pays nothing for the feature.
    fn records_undo(&self) -> bool {
        false
    }

    /// Record the pre-mutation `.md` for `page`.
    ///
    /// Called only when [`Self::records_undo`] is `true` **and** the
    /// mutation actually changed the page's render.
    fn record_undo(&mut self, page: NodeId, before: String) {
        let _ = (page, before);
    }

    /// Drop any cached backlinks index. Called on every commit, because
    /// any mutation can add or remove a `[[ref]]`.
    fn invalidate_backlinks(&mut self) {}

    /// Tell peers this device produced new ops for `page`, so they pull
    /// now rather than on the next catch-up sweep.
    ///
    /// Takes the workspace and the node rather than a resolved slug on
    /// purpose. Resolving it costs a `page_meta` — five property lookups
    /// and a `block_text` — and a host with no transport wired has
    /// nothing to announce, so paying that on every commit would tax the
    /// mutation path for a feature that host does not have. The hook
    /// checks its transport first and resolves only if there is one.
    ///
    /// Deliberately **not** gated by a second `announces() -> bool`
    /// predicate: two flags guarding one step is how a host ends up
    /// overriding the action and not the predicate, and silently never
    /// announcing again.
    fn announce(&mut self, workspace: &Workspace, page: NodeId) {
        let _ = (workspace, page);
    }
}

/// Run `mutate` against `page`, then the four steps that go around it.
///
/// Returns whatever the mutation returned (the new `NodeId` for a
/// create, the cut markdown for a cut, `()` for most). The projection
/// has already run — or been queued — by the time this returns, so a
/// caller that reads the page back sees the committed state.
///
/// We deliberately do **not** run `reconcile_md` before `mutate`: the op
/// log is already up to date with whatever peers delivered, and
/// "catching up" from a lagging on-disk `.md` risks emitting a delete
/// cascade for content the log has and the file does not (root
/// `CLAUDE.md` invariant 8).
pub fn commit_page<H, F, T>(
    workspace: &mut Workspace,
    hooks: &mut H,
    page: NodeId,
    mutate: F,
) -> Result<T, ActionError>
where
    H: CommitHooks + ?Sized,
    F: FnOnce(&mut Workspace) -> Result<T, ActionError>,
{
    // Step 1. Snapshot before, only if anyone is keeping undo.
    let before = hooks
        .records_undo()
        .then(|| render_page_md(workspace, page));

    // Step 2. The mutation. The only step that can fail the commit.
    let value = mutate(workspace)?;

    // Step 3. Keep the snapshot only when the render actually moved.
    if let Some(before) = before {
        if render_page_md(workspace, page) != before {
            hooks.record_undo(page, before);
        }
    }

    // Step 4. The tree changed, so any cached backlinks index is stale.
    hooks.invalidate_backlinks();

    // Step 5. Wake peers, then project. Slug resolution belongs to the
    // hook — see `CommitHooks::announce` for why it is not done here.
    hooks.announce(workspace, page);
    hooks.project(workspace, page);

    Ok(value)
}

#[cfg(test)]
mod tests;
