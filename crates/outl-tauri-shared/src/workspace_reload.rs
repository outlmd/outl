//! Publishing a freshly replayed `Workspace` without dropping a local op.
//!
//! A reload replays the whole op log (`SyncEngine::reload_workspace`),
//! which is O(all ops) and CPU-bound — seconds on a freshly-synced
//! workspace. Both GUI clients therefore run it on a blocking pool thread
//! rather than on the Tauri IPC thread, where a synchronous replay froze
//! the window and, on iOS, tripped the scene-update watchdog.
//!
//! ## The cost that offload carries
//!
//! The replay runs **outside** the workspace mutex, so a local edit can be
//! applied — and appended to `ops-<actor>.jsonl` — after
//! `JsonlStorage::open` has already scanned the log and before the fresh
//! workspace is swapped in. The op survives on disk, but it is missing
//! from the tree that gets published, and the next projection renders that
//! tree over the page's `.md`: the guard in
//! `apply_page_md_with_sidecar_guarded` compares the file against the
//! *sidecar*, which still lists the block, so nothing refuses and the
//! user's line is deleted from disk. A dropped op becomes deleted text.
//!
//! ## Why this is a publish-time check and not a longer lock
//!
//! Holding the workspace mutex across the replay would serialize it
//! against mutations, and it would also reinstate the seconds-long stall
//! the offload exists to remove — a blocked mutation command is a blocked
//! IPC thread. So the *publish* is made conditional instead.
//!
//! [`OpLog::len`](outl_core::log::OpLog::len) is the marker. It is exact
//! for the question being asked and O(1) to read:
//!
//! - `Workspace::apply` appends to the resident log exactly when it also
//!   persists — a re-delivered op returns before both, so a length that
//!   did not move means nothing reached disk either;
//! - the log is never pruned (`apply_lru_cap` bounds the op *cache*, not
//!   `OpLog`), so the length only ever grows for a given workspace;
//! - a `WorkspaceBatch` buffers ops that are already in the log, and its
//!   guard borrows `&mut Workspace`, so it cannot outlive the critical
//!   section that reads the marker.
//!
//! Peer ops are deliberately *not* covered: sync ingest writes
//! `ops-<peer>.jsonl` straight to disk without touching the live
//! workspace, so a replay that misses one publishes a tree no worse than
//! the one already on screen, and the next reload picks it up. Only local
//! ops can be *lost* by a swap, because only they exist in the live tree.
//!
//! The comparison and the swap happen in one critical section. Splitting
//! them — the shape both clients had, which compared under one lock and
//! swapped under the next — reopens the same window on a smaller scale.

use std::path::PathBuf;

use outl_actions::{open_today, SyncEngine};
use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use tracing::warn;

use crate::helpers::{invalidate_backlink_index, invalidate_changed_history};
use crate::host::AppHost;

/// How many times a reload re-runs its replay after losing a race with a
/// local edit.
///
/// Bounded, because a user typing continuously must not be able to hold a
/// reload in a loop forever; three rather than one, because a retry costs
/// a replay and losing is cheap to detect. Two lost races in a row is
/// already unusual: the desktop defers peer-driven reloads while a block
/// is being edited, and the FS watcher that triggers them ignores this
/// device's own `ops-<actor>.jsonl`.
pub const RELOAD_ATTEMPTS: usize = 3;

/// Replay `<root>/ops/` into a fresh [`Workspace`], resolve today's
/// journal in it, and refresh that page's `.md`.
///
/// Runs on a blocking pool thread and touches no shared state, which is
/// what lets [`publish_replayed`] re-run it.
///
/// **Deliberately not an orphan-`.md` reconcile.** That pass runs
/// `md → ops` and desync recovery, both of which mutate the op log; on a
/// page being edited on two devices at once it turned a routine reload
/// into a projection ↔ op-log feedback loop and made the page flip-flop.
/// iroh peers ship ops, not `.md`, so a reload only has to re-materialize
/// the log; orphan recovery runs once at boot
/// ([`crate::workspace_open::reconcile_orphan_md`]).
pub fn replay_from_disk(root: PathBuf, hlc: HlcGenerator) -> Result<Workspace, String> {
    let engine = SyncEngine::new(root, hlc.actor());
    let mut fresh = engine
        .reload_workspace(&hlc)
        .map_err(|e| format!("reload workspace: {e}"))?;
    // `open_today` is idempotent: an existing page just yields its id, a
    // missing one is created with the deterministic slug-derived id both
    // peers agree on. Resolved in `fresh` so the id reflects the merged log.
    let today = open_today(&mut fresh, &hlc).map_err(|e| e.to_string())?;
    // Guarded (root `CLAUDE.md` invariant 8) — it can refuse when today's
    // `.md` holds content the merge never saw. Not propagated: that would
    // abort the reload before `fresh` (which already holds every peer's
    // merged ops) is published, turning one page's refusal into every page
    // failing to converge. Both frontends re-open the current page right
    // after this command returns, and that open re-runs the equivalent
    // check and sets `PageView.md_ahead_of_log` — so the refusal still
    // reaches the banner, one round-trip later.
    if let Err(e) = engine.reproject_page(&fresh, today) {
        warn!("reload: today's page stopped syncing: {e}");
    }
    Ok(fresh)
}

/// The reload both GUI clients run: replay off-thread, then publish only
/// a tree that is not missing a local op.
///
/// Everything a client owes after the swap that is *not* client-specific
/// happens here — surgical undo invalidation and dropping the cached
/// backlinks index. What stays in the client is what genuinely diverges
/// (the desktop's background `.md` reconcile).
///
/// # Errors
///
/// The replay's own failures, and [`RELOAD_ATTEMPTS`] consecutive lost
/// races. The latter leaves the workspace exactly as it was: no op is
/// lost, the peer's ops are still on disk, and the next sync signal
/// retries.
pub async fn reload_workspace_into<S: AppHost>(state: &S) -> Result<(), String> {
    let root = state.storage_root()?;
    let hlc = state.hlc().clone();
    publish_replayed(state, RELOAD_ATTEMPTS, move || {
        replay_from_disk(root.clone(), hlc.clone())
    })
    .await
}

/// Run `replay` off the calling thread and publish its result only if no
/// local op landed while it ran; retry up to `attempts` times.
///
/// Split out from [`reload_workspace_into`] so a test can supply a replay
/// that lands an edit inside the window, which is otherwise a race nobody
/// can stage.
pub async fn publish_replayed<S, F>(state: &S, attempts: usize, replay: F) -> Result<(), String>
where
    S: AppHost,
    F: Fn() -> Result<Workspace, String> + Clone + Send + 'static,
{
    let attempts = attempts.max(1);
    for attempt in 1..=attempts {
        let before = resident_ops(state);
        let job = replay.clone();
        let fresh = tauri::async_runtime::spawn_blocking(job)
            .await
            .map_err(|e| format!("reload task join: {e}"))??;

        // One critical section: compare, invalidate, swap.
        let mut slot = state.workspace().lock();
        if slot.as_ref().map(|ws| ws.log().len()) == before {
            if let Some(history) = state.history() {
                // Lock order is workspace -> history, the order every
                // other path in this crate takes.
                let mut history = history.lock();
                invalidate_changed_history(slot.as_ref(), &fresh, &mut history);
            }
            *slot = Some(fresh);
            drop(slot);
            // Peer ops replaced the workspace, so the cached index is
            // stale; the next `page_backlinks` rebuilds it.
            invalidate_backlink_index(state);
            return Ok(());
        }
        drop(slot);
        warn!(
            "reload attempt {attempt}/{attempts}: a local edit landed while the op log was \
             replaying; replaying again rather than publishing a tree that is missing it"
        );
    }
    Err(format!(
        "reload lost {attempts} races against local edits; the workspace was left as it was and \
         the next sync signal retries"
    ))
}

/// Ops resident in the published workspace, or `None` when no workspace
/// is open yet (which is itself a state the swap has to match — a
/// workspace that appeared mid-replay was published by someone else).
fn resident_ops<S: AppHost>(state: &S) -> Option<usize> {
    state.workspace().lock().as_ref().map(|ws| ws.log().len())
}
