//! Workspace open / reconcile / actor-id primitives.
//!
//! The boot **openers** stay in the client crates (the desktop wires an
//! FS watcher + background reconcile + iroh slots; the mobile reconciles
//! inline and returns through `AppState`) — but every step they compose
//! lives here so the two can't drift on semantics.
//!
//! ## Why the GUI refuses instead of forking an ephemeral actor
//!
//! [`outl_core::resolve_write_actor`] answers `AlreadyHeld` by minting a
//! fresh [`ActorId`] and writing to a brand-new `ops-<ephemeral>.jsonl`.
//! That is the right answer for the CLI and TUI, and it is **not**
//! available here, because a GUI's actor is fixed before a workspace is
//! picked: `HlcGenerator::new(actor)` is built in the client's `setup()`
//! from [`load_or_create_actor`], and every mutation stamps its op with
//! that generator's actor for the life of the process.
//!
//! Swapping only the *storage* actor would therefore write ops stamped
//! `device_actor` into `ops-<ephemeral>.jsonl`, and leave two live
//! generators sharing one actor id. Two `HlcGenerator`s with the same
//! actor emit the same `(time, counter, actor)` triple whenever they tick
//! in the same millisecond, and op identity *is* that triple — so
//! `Workspace::apply`'s `contains_ts` dedup silently drops one of the two
//! ops. A fallback that loses ops is worse than the collision it is
//! dodging, so [`open_workspace_at`] surfaces the refusal instead.
//!
//! Making the fallback available means making the client's `HlcGenerator`
//! swappable at workspace-open time (it is a plain field on both
//! `AppState`s today, read from ~56 call sites). That is a client-side
//! change, tracked separately.

use std::path::{Path, PathBuf};

use outl_actions::{migrate_legacy_into_today, open_today};
use outl_core::device::DeviceStore;
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::lock::{ActorWriteLock, LockError, WorkspaceLock};
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use parking_lot::Mutex;
use tracing::{info, warn};

/// The cross-process locks a GUI client holds for as long as its
/// workspace is open — the same two every other `outl` process takes,
/// through the same [`outl_core::lock`] API the CLI and TUI use.
///
/// ## Why a GUI needs them
///
/// The Tauri clients used to take neither, which made them invisible to
/// every other `outl` process on the machine:
///
/// - [`WorkspaceLock`] is the shared advisory flock on
///   `<root>/.outl/.lock`. `outl compact --apply` answers *"is anyone in
///   this workspace?"* by trying to take that same file **exclusively**
///   ([`outl_core::storage::compact::apply_compaction`]). A GUI holding
///   nothing let that gate pass, and compaction then renamed a rewritten
///   `ops-<actor>.jsonl` out from under a live client that still held
///   in-memory byte offsets into the pre-compaction layout. Every later
///   index-driven read in that session seeks into a renumbered file —
///   "a silently dropped op on every index-driven read", in the words of
///   compaction's own comment.
/// - [`ActorWriteLock`] is the exclusive flock on
///   `<root>/ops/.lock-<actor>`. It is the stated precondition of
///   `JsonlStorage::append_ops` ("append is the SINGLE writer for its own
///   actor file, guarded by `ActorWriteLock`") and compaction's second
///   gate.
///
/// ## Lifetime
///
/// Held for as long as the workspace is open, and **only** that long.
/// The guards live in the client's `workspace_guards` slot next to its
/// `Option<Workspace>`; [`open_workspace_at`] is the single writer of
/// that slot, installing the new set only after the open succeeds (which
/// drops the previous workspace's set). The flocks are released by
/// `Drop`, and by the OS if the process dies — there is no stale-lock
/// state to clean up.
#[must_use = "the locks are released when the guards are dropped; keep them alive with the workspace"]
#[derive(Debug)]
pub struct WorkspaceGuards {
    /// Shared `<root>/.outl/.lock`. Multiple holders by design.
    _workspace: WorkspaceLock,
    /// Exclusive `<root>/ops/.lock-<actor>`.
    _actor: ActorWriteLock,
    /// Canonicalized root, so a re-pick of the workspace already open can
    /// be told from a switch to a different one.
    root: PathBuf,
    actor: ActorId,
}

impl WorkspaceGuards {
    /// Root these guards were taken for (canonicalized where possible).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Actor whose `ops-<actor>.jsonl` this process owns the write lock on.
    pub fn actor(&self) -> ActorId {
        self.actor
    }

    fn acquire(root: &Path, canonical: &Path, actor: ActorId) -> Result<Self, LockError> {
        // Shared first, then per-actor — the same order `outl-ws` uses
        // (`outl_ws::open_with`) and the order compaction checks them in.
        let workspace = WorkspaceLock::acquire(root)?;
        let write = ActorWriteLock::try_acquire(&root.join("ops"), actor)?;
        Ok(Self {
            _workspace: workspace,
            _actor: write,
            root: canonical.to_path_buf(),
            actor,
        })
    }
}

/// Best-effort canonicalization: a path that cannot be resolved (the
/// directory was just created, or the platform refuses) is compared as
/// given. Only used to recognise "the workspace I already hold".
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Take both locks for `path`, releasing the ones this process already
/// holds **for that same root** first.
///
/// The retry is not a race workaround. A POSIX `flock` is owned by an
/// open file description, so a second `open` + `LOCK_EX|LOCK_NB` of
/// `ops/.lock-<actor>` fails *even inside the process that already holds
/// it*. Re-picking the workspace that is already open (the folder dialog
/// pointed at the current root) would therefore refuse itself. Releasing
/// and retaking is correct there and nowhere else: the workspace being
/// released is the one this open is replacing.
///
/// Any other `AlreadyHeld` is a genuinely different process on this
/// actor and is surfaced, never worked around — see the module docs on
/// why the ephemeral-actor fallback is not available to a GUI.
fn acquire_guards(
    held: &Mutex<Option<WorkspaceGuards>>,
    path: &Path,
    actor: ActorId,
) -> Result<WorkspaceGuards, LockError> {
    let canonical = canonical(path);
    match WorkspaceGuards::acquire(path, &canonical, actor) {
        Err(LockError::AlreadyHeld(path_held)) => {
            // Read the slot and release it in two statements, never in a
            // match guard: `parking_lot::Mutex` is not reentrant, so a
            // guard temporary still alive over the arm body would deadlock
            // this thread against itself.
            let ours = {
                let slot = held.lock();
                slot.as_ref()
                    .is_some_and(|g| g.root == canonical && g.actor == actor)
            };
            if !ours {
                return Err(LockError::AlreadyHeld(path_held));
            }
            // Dropping here loses the guards if the reopen below fails.
            // That window is the re-pick path only, where the workspace
            // was being replaced anyway, and it degrades to exactly the
            // unguarded state every GUI was in before this existed.
            *held.lock() = None;
            WorkspaceGuards::acquire(path, &canonical, actor)
        }
        other => other,
    }
}

/// Open (or create) the workspace rooted at `path`.
///
/// Idempotent: the `ops/`, `journals/`, `pages/` directories are created
/// if missing, and `migrate_legacy_into_today` reshuffles any
/// pre-page-model blocks under today's journal (also idempotent).
///
/// **Does NOT run the orphan-md reconcile pass** — that work scales with
/// the number of pages; each client decides whether to run
/// [`reconcile_orphan_md`] inline (mobile) or on a background thread
/// (desktop).
///
/// `lru_cap` is the in-memory op cache bound (RFC #137). `0` keeps the
/// legacy unbounded behaviour; any positive value sheds cold history
/// after boot completes (so RSS stays constant regardless of workspace
/// age).
///
/// `held` is the client's [`WorkspaceGuards`] slot. This function is its
/// **single writer**: the new guards are installed only once the open has
/// succeeded, which is also what releases the previous workspace's. A
/// client never acquires or drops a lock itself — a second opinion about
/// who holds the workspace is the defect this replaces.
///
/// # Errors
///
/// Beyond the usual I/O and boot failures, this refuses to open when
/// another `outl` process on this machine already holds the write lock
/// for `actor` (`LockError::AlreadyHeld`). See the module docs for why a
/// GUI cannot take the ephemeral-actor fallback instead.
pub fn open_workspace_at(
    actor: ActorId,
    hlc: &HlcGenerator,
    path: &Path,
    lru_cap: usize,
    held: &Mutex<Option<WorkspaceGuards>>,
) -> anyhow::Result<Workspace> {
    std::fs::create_dir_all(path.join("ops"))?;
    std::fs::create_dir_all(path.join("journals"))?;
    std::fs::create_dir_all(path.join("pages"))?;

    // Before a byte is read: announce this process to every other `outl`
    // on the machine. Held in `guards` until the open succeeds, so a
    // failed open leaves no lock behind.
    let guards = acquire_guards(held, path, actor)?;

    let storage = JsonlStorage::open(path.join("ops"), actor)?;
    let mut workspace =
        Workspace::open_with_storage(actor, Box::new(storage), Some(path.to_path_buf()))?;

    // Register per-page shards + reboot BEFORE running boot helpers so
    // the materialized tree is complete (migrated workspaces have an
    // empty global log — the initial boot sees nothing without shards).
    outl_actions::storage_scope::register_per_page_storages(
        &mut workspace,
        &path.join("ops"),
        actor,
        path,
    );
    if workspace.has_page_storages() {
        workspace.reboot_with_all_storages()?;
    }

    // Raise the generator above everything the log already holds, before
    // the boot helpers below emit their first op (see
    // `Workspace::seed_clock`). This is the GUI clients' only seeding
    // point: their `HlcGenerator` is built in `setup()`, before a
    // workspace is picked, and a GUI can close one workspace and open
    // another — so it has to happen on *every* open, not once at start.
    // Re-seeding is safe by construction: `seed` never lowers the clock.
    //
    // Propagated, unlike the best-effort helpers below it: those are
    // repairs whose failure leaves a workspace that still works, while a
    // storage read failing here says the boot that just succeeded cannot
    // be re-read a statement later.
    workspace.seed_clock(hlc)?;

    if let Err(e) = migrate_legacy_into_today(&mut workspace, hlc) {
        warn!("legacy migration: {e}");
    }
    // Repair split-brain page/journal roots (two roots sharing one slug, e.g. a
    // sidecar-less `.md` reconciled to a fresh id before the deterministic-id
    // fix). Merges every duplicate's children under the canonical root and
    // trashes the emptied duplicates — via Ops, so it converges across devices.
    // Idempotent; a clean workspace is a no-op.
    match outl_actions::merge_duplicate_slug_roots(&mut workspace, hlc) {
        Ok(0) => {}
        Ok(n) => warn!("merged {n} duplicate slug root(s) on boot"),
        Err(e) => warn!("duplicate-slug-root repair: {e}"),
    }
    if let Err(e) = open_today(&mut workspace, hlc) {
        warn!("could not pre-open today: {e}");
    }

    // Shed cold history AFTER boot + helpers finish. Boot needs every
    // op in RAM to rebuild Yrs `Doc`s; afterwards cold ops come back
    // from disk via the offset index.
    workspace.apply_lru_cap(lru_cap);

    // Snapshot boot-cache policy (#128/#109): as a long-lived client the
    // GUI writes background snapshots so the next open (this app, the CLI,
    // or a peer) boots from one instead of replaying the whole op log.
    // Defaults (enabled, 10k) unless `[snapshot]` overrides them.
    let snap_cfg = outl_config::load().snapshot;
    workspace.set_snapshot_policy(snap_cfg.enabled, snap_cfg.op_threshold);

    // Write-through snapshot after a cold full replay.
    //
    // A receive-only device (mobile paired to a desktop) gets its ops from
    // sync ingest, which writes `ops-*.jsonl` straight to disk — never
    // through `Workspace::apply` — so the background snapshot writer (which
    // only fires from `apply` crossing the threshold) never runs. Every boot
    // then full-replays the entire log (200k+ ops → tens of seconds on a
    // phone), and the post-`workspace-ready` reload replays it AGAIN.
    // Persist one snapshot here, after the first replay that found none on
    // disk, so the next boot and that reload are O(delta). Best-effort; a
    // stale/corrupt snapshot is always safe — boot silently falls back to a
    // full replay — so this can never corrupt state, only save work.
    // Re-persist a fresh snapshot whenever this boot FULL-REPLAYED (snapshot
    // absent, stale, or rejected by the convergence guard). A stale snapshot
    // the guard keeps rejecting would otherwise full-replay on every open, and
    // the resident 200k-op log is fine to render now (block_text is index-
    // driven) — but the NEXT boot should adopt a snapshot instead of replaying.
    // `save_snapshot` is O(log) now (the block-text index makes
    // `force_materialize_pending` cheap), not the old O(blocks × log), so this
    // is safe to do after a full replay.
    if snap_cfg.enabled
        && !workspace.booted_from_snapshot()
        && workspace.log().len() as u32 >= snap_cfg.op_threshold
    {
        if let Err(e) = workspace.save_snapshot() {
            warn!("boot: could not persist snapshot: {e}");
        }
    }

    // The open succeeded, so publish the guards — which is what releases
    // the previous workspace's. Doing it here rather than at acquisition
    // means a failed open never parks a lock on a workspace nobody has
    // open (compaction would then refuse forever with nothing running).
    info!(
        "workspace locks held for {} (actor {actor})",
        guards.root().display()
    );
    *held.lock() = Some(guards);

    Ok(workspace)
}

/// Load (or generate-and-persist) the device's actor id.
///
/// The actor identifies the device, not the workspace — it's reused
/// across whatever directory the user picks. Lives at
/// `<local_dir>/actor` as a plain ULID string.
///
/// `local_dir` is **outside** any workspace (`~/.config/outl` on the
/// desktop, the app sandbox's data dir on mobile), which is what keeps
/// the GUI clients clear of the cross-device actor collision described
/// in [`outl_core::device`]: nothing here ever rides the file-sync
/// surface. Being device-wide rather than per-workspace is deliberate —
/// the `HlcGenerator` is built at app start, before a workspace is
/// picked — so these clients stay on
/// [`DeviceStore::device_actor`] while the CLI / TUI key theirs by
/// workspace id.
pub fn load_or_create_actor(local_dir: &Path) -> std::io::Result<ActorId> {
    let actor = DeviceStore::at(local_dir)
        .device_actor()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    info!("device actor id {actor}");
    Ok(actor)
}

/// Scan `<root>/journals/` and `<root>/pages/` for `.md` files that are
/// not represented in the op log yet — either no sidecar exists (file
/// was just imported, dropped in by vim, or written by a peer that only
/// shipped the projection) or the sidecar's `last_synced_hash` is stale
/// (the file was edited externally since the last reconcile). Runs
/// `reconcile_md` on each so the workspace, the sidecar, and `.md`
/// converge.
///
/// Then runs the **desynced-projection** pass: pages whose sidecar is
/// hash-in-sync with the `.md` but references block ids no op log ever
/// created (projection written, ops append lost — e.g. the OS killed
/// the app right after an offline edit). The hash gate above can't see
/// those; `recover_desynced_projection` re-emits the lost ops with the
/// sidecar ids preserved so the blocks finally reach the log and sync.
///
/// Both passes run **`.md → tree`**: nothing here renders the tree over a
/// `.md`, so neither can delete on-disk content, whatever the hash gate
/// they select on says. The re-projection at the end of the desync
/// recovery is the one write, and it carries its own guard.
/// What this inherits from `scan_for_orphans` is its blind spot, not a
/// hazard: a hash-faithful `.md` holding content that exists in no op is
/// never queued here, so it stays local and unsynced until
/// `outl reconcile --ahead-of-log` runs (see `needs_reconcile` in
/// `outl-actions::sync`, RFC 0210).
pub fn reconcile_orphan_md(workspace: &mut Workspace, hlc: &HlcGenerator, storage_root: &Path) {
    let engine = outl_actions::SyncEngine::new(storage_root.to_path_buf(), hlc.actor());
    // `Some(..)`, never `None`: a block that fails to match here drops
    // to matching level 3, which trashes it. `outl-md`'s hard rule is
    // that such a block is recorded before it goes. Booting with the
    // log off made every GUI client delete silently — exactly the case
    // a half-synced `.md` from iCloud produces.
    let orphans = engine.orphans_log();
    for path in &engine.scan_for_orphans() {
        if let Err(e) = outl_md::reconcile::reconcile_md(workspace, hlc, path, Some(&orphans)) {
            warn!("orphan reconcile failed for {}: {e}", path.display());
        }
    }
    for path in &engine.scan_for_desynced_projections(workspace) {
        match outl_actions::recover_desynced_projection(workspace, hlc, storage_root, path) {
            Ok(n) if n > 0 => info!(
                "recovered {n} lost op(s) from desynced projection {}",
                path.display()
            ),
            Ok(_) => {}
            Err(e) => warn!("desync recovery failed for {}: {e}", path.display()),
        }
    }
}

/// Yield the disk for as long as the work just took, so a background
/// pass never competes with the user for it.
///
/// ## Why this exists
///
/// outl's premise is that it opens fast and is ready for input, and that
/// promise is about the *device*, not about thread count. A batch pass
/// on a worker thread still breaks it if it saturates I/O, because the
/// UI reads the same disk to paint.
///
/// Measured on the boot after a `CURRENT_PIPELINE_VERSION` bump, which
/// makes every sidecar stale by pipeline: 2,827 files, **24.7 seconds at
/// 8% CPU**. Not computation — `write_atomic`'s two `fsync`s per
/// sidecar, 5,656 of them back to back, for 44 ops of actual content.
///
/// ## Why not the cheaper fix
///
/// Dropping the sidecar `fsync` takes those 24.7s to 0.3s, and it is the
/// wrong trade. A rename that lands before its data leaves a sidecar of
/// garbage, which reads as a *missing* one, which mints a fresh ULID per
/// block: the page duplicates and every `((blk-…))` handle breaks.
/// Skipping the migration is worse still, since the parser fix then
/// never reaches pages nobody opens.
///
/// ## What this does instead
///
/// Nobody needs the migration to be *fast*. It needs to be invisible.
/// Sleeping for exactly as long as the last unit of work took holds the
/// pass to about half the device and leaves the rest to whoever is
/// typing. A slow disk makes it yield more, not stutter more, because
/// the ratio is what stays fixed, not the delay.
///
/// Call it **outside** the workspace lock — sleeping while holding it is
/// the same stall with extra steps.
pub fn yield_to_user(work: std::time::Duration) {
    std::thread::sleep(work);
}

#[cfg(test)]
mod pace_tests {
    use super::yield_to_user;
    use std::time::{Duration, Instant};

    /// The pass yields as long as it worked, so it uses about half the
    /// device and leaves the rest to whoever is typing.
    #[test]
    fn the_pass_yields_as_long_as_it_worked() {
        let started = Instant::now();
        yield_to_user(Duration::from_millis(20));
        let slept = started.elapsed();
        assert!(
            slept >= Duration::from_millis(15),
            "yielded {slept:?}, expected roughly the 20ms it worked"
        );
        assert!(
            slept < Duration::from_millis(120),
            "yielded {slept:?}, far past the work — the ratio is what stays fixed, not the delay"
        );
    }

    /// A slow disk must make the pass yield *more*, not stutter more.
    #[test]
    fn the_yield_scales_with_the_work_not_a_fixed_delay() {
        let quick = Instant::now();
        yield_to_user(Duration::from_millis(4));
        let quick = quick.elapsed();

        let slow = Instant::now();
        yield_to_user(Duration::from_millis(40));
        let slow = slow.elapsed();

        assert!(
            slow > quick * 3,
            "a 10x slower page yielded {slow:?} vs {quick:?} — the share is not being held"
        );
    }
}
