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

use std::time::Instant;

use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use tracing::{debug, warn};

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
    // Every phase is timed, because this is the path a keystroke waits
    // on and nothing measured it. `boot_timing` covers the launch; this
    // covers every edit after it, which is where a user actually lives.
    // Two `Instant` reads per phase, behind a `debug!` — see the module
    // doc's "What this costs to measure".
    let t0 = Instant::now();

    // Step 1. Snapshot before, only if anyone is keeping undo.
    let before = hooks
        .records_undo()
        .then(|| render_page_md(workspace, page));
    let snapshot_us = t0.elapsed().as_micros();

    // Step 2. The mutation. The only step that can fail the commit.
    let t = Instant::now();
    let value = mutate(workspace)?;
    let mutate_us = t.elapsed().as_micros();

    // Step 3. Keep the snapshot only when the render actually moved.
    let t = Instant::now();
    if let Some(before) = before {
        if render_page_md(workspace, page) != before {
            hooks.record_undo(page, before);
        }
    }
    let undo_us = t.elapsed().as_micros();

    // Step 4. The tree changed, so any cached backlinks index is stale.
    let t = Instant::now();
    hooks.invalidate_backlinks();
    let invalidate_us = t.elapsed().as_micros();

    // Step 5. Wake peers, then project. Slug resolution belongs to the
    // hook — see `CommitHooks::announce` for why it is not done here.
    let t = Instant::now();
    hooks.announce(workspace, page);
    let announce_us = t.elapsed().as_micros();
    let t = Instant::now();
    hooks.project(workspace, page);
    let project_us = t.elapsed().as_micros();

    let total_us = t0.elapsed().as_micros();
    // `debug!`, not `info!`: one line per keystroke at info level is a
    // log nobody can read. The desktop's default filter already enables
    // debug for the sync crate only, so reading these means asking for
    // them (`RUST_LOG=outl_actions=debug`).
    debug!(
        total_us,
        snapshot_us,
        mutate_us,
        undo_us,
        invalidate_us,
        announce_us,
        project_us,
        "commit_page phases"
    );
    // A commit a user can feel, said once and loudly enough to find.
    // 50ms is the threshold a keystroke-to-paint budget leaves for the
    // whole commit; past it the cause is one of the phases above.
    if total_us > SLOW_COMMIT_US {
        warn!(
            total_ms = total_us / 1000,
            snapshot_us,
            mutate_us,
            undo_us,
            announce_us,
            project_us,
            "commit_page was slow enough for the user to notice"
        );
    }

    Ok(value)
}

/// Past this, a commit is slow enough that the user felt it, and the
/// phase breakdown is worth a `warn!` rather than a `debug!` nobody
/// enabled.
///
/// 50ms is not a guess: a keystroke that repaints within ~100ms reads as
/// instant, and the commit is only part of that budget (the IPC hop and
/// the render are the rest). It is deliberately a loud log and not a
/// metric — this is the line that tells a maintainer *which* of the five
/// phases to look at, on the machine where it actually happened.
const SLOW_COMMIT_US: u128 = 50_000;

#[cfg(test)]
mod tests;
