//! Mobile workspace resolution + background opener.
//!
//! The open / actor-id / orphan-reconcile primitives live in
//! `outl_tauri_shared::workspace_open` (re-exported below). This module
//! keeps what is genuinely mobile:
//!
//! ## No `BackupRoot` on `AppState`
//!
//! The opener takes the `BackupRoot` by value and `AppState` holds none,
//! unlike the desktop: mobile has one `spawn_workspace_opener` call and
//! no `set_workspace` equivalent, so nothing can retarget it later.
//! A workspace picker would need the field the desktop already has.
//!
//! ## Storage is a local folder, synced by iroh (no iCloud)
//!
//! The workspace root is a folder on this device; iroh P2P is the only
//! sync. A fresh install works with **no iCloud at all**: the default
//! root is `<app-data-dir>/outl/` and iroh ships the op log to paired
//! devices.
//!
//! Resolution order in [`resolve_storage_root`]:
//!
//! 1. The persisted `WorkspaceCfg.last` path, when present and usable
//!    (survives restarts — written by [`persist_workspace_path`]).
//! 2. The app-local default `<app-data-dir>/outl/`.
//!
//! Picking an arbitrary folder is the deferred native-picker concern (see
//! `workspace_picker`); until that lands the default local root is the
//! only path a fresh install ever opens.
//!
//! ## Other mobile divergences from desktop
//!
//! - Orphan `.md` reconcile runs on a **background thread**, AFTER the
//!   workspace is published and `workspace-ready` fires (mirrors the
//!   desktop). On a large workspace the filesystem walk is the
//!   cold-boot bottleneck; deferring it past first paint keeps the
//!   very first journal render off the critical path.
//! - There is no filesystem watcher: a single device needs none, and
//!   peer ops arrive through iroh, which pokes the reload itself (see
//!   `iroh_sync::wire_iroh_transport`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use outl_actions::backup::BackupRoot;
use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use parking_lot::Mutex;
use tauri::Emitter;
use tracing::{info, warn};

pub(crate) use outl_tauri_shared::workspace_open::{
    load_or_create_actor, open_workspace_at, WorkspaceGuards,
};

/// Folder name for the **local** default workspace, created under the
/// app's data dir when the user hasn't picked anything yet.
///
/// A fresh install lands here — iroh is the sync.
const LOCAL_WORKSPACE_DIR: &str = "outl";

/// The app-local default workspace root: `<app-data-dir>/outl/`.
///
/// This is what a fresh install uses — iroh syncs it P2P.
pub(crate) fn local_default_root(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(LOCAL_WORKSPACE_DIR)
}

/// Resolve the workspace root to open on boot.
///
/// Order:
///
/// 1. A previously chosen folder, if `persisted` points at something
///    usable (the user picked it; it survived this restart).
/// 2. The app-local default `<app-data-dir>/outl/` — a fresh install,
///    synced by iroh.
///
/// `persisted` is `WorkspaceCfg.last` from `outl-config`. A `None` (first
/// launch) or a path that no longer resolves falls through to the local
/// default.
pub(crate) fn resolve_storage_root(app_data_dir: &Path, persisted: Option<&Path>) -> PathBuf {
    if let Some(chosen) = persisted {
        // Accept the chosen path as long as its parent exists (the
        // workspace dir itself is created by the caller). A path whose
        // parent vanished — e.g. a removed external volume — falls through
        // to the local default instead of failing the boot.
        let usable = chosen.exists() || chosen.parent().map(|p| p.exists()).unwrap_or(false);
        if usable {
            info!("opening chosen workspace at {}", chosen.display());
            return chosen.to_path_buf();
        }
        warn!(
            "chosen workspace {} no longer resolves; using local default",
            chosen.display()
        );
    }

    let default_root = local_default_root(app_data_dir);
    info!(
        "no workspace chosen; using local default {}",
        default_root.display()
    );
    default_root
}

/// Persist the chosen workspace path so the next launch reopens it
/// instead of the default.
///
/// Writes `WorkspaceCfg.last` through `outl-config` (the single config
/// reader/writer — never hand-edit the TOML). Best-effort: a failure is
/// logged, never fatal, because the workspace is already open in memory.
///
/// One of those failures is a refusal: `outl_config::save` will not write
/// over a `config.toml` that failed to parse (issue #284). The cost of
/// that refusal here is small and recoverable — the next launch opens the
/// default workspace instead of this one — and it is the alternative that
/// is not: writing would have replaced the user's whole config with
/// defaults.
///
/// Nothing tells the user here, and on iOS there is no `outl doctor` to
/// fall back on: the reminders sheet is the only mobile surface that
/// reports this, and only when the user tries to change a setting. A boot
/// notice is the open gap (`outl-config/CLAUDE.md` → The write guard).
#[allow(dead_code)] // Wired by the folder picker; see workspace_picker.rs.
pub(crate) fn persist_workspace_path(path: &Path) {
    let mut cfg = outl_config::load();
    if cfg.workspace.last.as_deref() == Some(path) {
        return;
    }
    cfg.workspace.last = Some(path.to_path_buf());
    if let Err(e) = outl_config::save(&cfg) {
        warn!("could not persist chosen workspace {}: {e}", path.display());
    }
}

/// Background opener. Runs once per process; sets the inner
/// `Option<Workspace>` and emits the `workspace-ready` event when done.
///
/// Publishes the workspace and fires `workspace-ready` **before** the
/// orphan-md reconcile, then defers that filesystem walk to
/// [`spawn_background_reconcile`]. The first `build_page_view` (today's
/// journal) paints without waiting on the walk — on a large workspace it
/// is the cold-boot bottleneck. Backlinks are fetched lazily off the
/// page-open path anyway (`page_backlinks`), so they no longer depend on
/// the reconcile finishing before first paint.
pub(crate) fn spawn_workspace_opener(
    workspace_slot: Arc<Mutex<Option<Workspace>>>,
    workspace_guards: Arc<Mutex<Option<WorkspaceGuards>>>,
    backup_root: BackupRoot,
    storage_root: PathBuf,
    hlc: HlcGenerator,
    app: tauri::AppHandle,
    open_done: std::sync::mpsc::Sender<()>,
) {
    let actor = hlc.actor();
    // Mobile pins the LRU cap to 5k ops (≈ 1 MB of cache) regardless of
    // what `outl.toml` says. The home page + its backlink set fit
    // comfortably in 5k ops; the rest is rebuildable from the offset
    // index. Keeps the long-lived client well under iOS jetsam.
    let lru_cap = outl_config::load().storage.lru_cap.min(5_000);
    thread::spawn(move || {
        let workspace = match open_workspace_at(
            actor,
            &hlc,
            &storage_root,
            lru_cap,
            &workspace_guards,
            &backup_root,
        ) {
            Ok(w) => w,
            Err(e) => {
                warn!("background open failed for {}: {e}", storage_root.display());
                return;
            }
        };
        // Held until here on purpose: dropping it is what releases the sync
        // transport, and a failed open above drops it on the way out.
        // Publish + fire `workspace-ready` FIRST so today's journal paints
        // immediately; the orphan reconcile runs after (see below).
        *workspace_slot.lock() = Some(workspace);
        if let Err(e) = app.emit("workspace-ready", ()) {
            warn!("emit workspace-ready: {e}");
        }
        info!("background workspace opener complete");
        // Let the sync transport start. It is a MEMORY gate, not a readiness
        // one: the open and a first sync both peak in the hundreds of MB, and
        // on a 723k-op workspace the two running together is what killed the
        // Android app — a launch with no `boot phases:` line in its log,
        // because the open never finished. A `Sender` rather than the
        // `workspace-ready` event because a channel cannot be missed: the
        // signal is buffered if the transport is not waiting yet, and a
        // panicking opener drops it, which wakes the waiter just the same.
        let _ = open_done.send(());

        // Reconcile orphan `.md` + repair doubled journal titles off the
        // boot path, on its own thread (mirrors desktop). The workspace is
        // already live; pages materialised by the reconcile surface on the
        // next read.
        spawn_background_reconcile(workspace_slot, storage_root, hlc);
    });
}

/// Deferred boot work: the shared background reconcile sweep, on a worker
/// thread so first paint is never blocked.
///
/// Runs after [`spawn_workspace_opener`] has published the workspace and
/// fired `workspace-ready`. The pass itself — which phases chunk their
/// lock, which two cannot and why, and why the whole-workspace ones wait
/// out the launch window — belongs to [`outl_tauri_shared::reconcile`],
/// which the desktop runs too. This client used to keep its own copy and
/// the two had drifted: that copy took every lock with a blocking
/// `lock()` and never yielded between pages.
///
/// **The completion callback is a no-op here, and that is a gap rather
/// than a decision.** The desktop emits `workspace-reconciled` and
/// `AppShell.tsx` re-reads the open page on it; mobile has no listener
/// for that event (`Journal.listeners.ts` covers `workspace-ready`), so a
/// page repaired by the sweep stays stale here until the next navigation
/// or pull-to-refresh. Closing it is frontend work.
fn spawn_background_reconcile(
    workspace_slot: Arc<Mutex<Option<Workspace>>>,
    storage_root: PathBuf,
    hlc: HlcGenerator,
) {
    let _ = outl_tauri_shared::reconcile::spawn_background_reconcile(
        workspace_slot,
        storage_root,
        hlc,
        || {},
    );
}
