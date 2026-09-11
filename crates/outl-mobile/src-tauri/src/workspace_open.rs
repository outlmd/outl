//! Mobile workspace resolution + background opener.
//!
//! The open / actor-id / orphan-reconcile primitives live in
//! `outl_tauri_shared::workspace_open` (re-exported below). This module
//! keeps what is genuinely mobile:
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
    storage_root: PathBuf,
    hlc: HlcGenerator,
    app: tauri::AppHandle,
) {
    let actor = hlc.actor();
    // Mobile pins the LRU cap to 5k ops (≈ 1 MB of cache) regardless of
    // what `outl.toml` says. The home page + its backlink set fit
    // comfortably in 5k ops; the rest is rebuildable from the offset
    // index. Keeps the long-lived client well under iOS jetsam.
    let lru_cap = outl_config::load().storage.lru_cap.min(5_000);
    thread::spawn(move || {
        let workspace =
            match open_workspace_at(actor, &hlc, &storage_root, lru_cap, &workspace_guards) {
                Ok(w) => w,
                Err(e) => {
                    warn!("background open failed for {}: {e}", storage_root.display());
                    return;
                }
            };
        // Publish + fire `workspace-ready` FIRST so today's journal paints
        // immediately; the orphan reconcile runs after (see below).
        *workspace_slot.lock() = Some(workspace);
        if let Err(e) = app.emit("workspace-ready", ()) {
            warn!("emit workspace-ready: {e}");
        }
        info!("background workspace opener complete");

        // Reconcile orphan `.md` + repair doubled journal titles off the
        // boot path, on its own thread (mirrors desktop). The workspace is
        // already live; pages materialised by the reconcile surface on the
        // next read.
        spawn_background_reconcile(workspace_slot, storage_root, hlc);
    });
}

/// Deferred boot work: reconcile orphan `.md` files and repair doubled
/// journal titles, on a worker thread so first paint is never blocked.
///
/// Runs after [`spawn_workspace_opener`] has published the workspace and
/// fired `workspace-ready`. Best-effort: a workspace closed between
/// publish and this pass (user picked another root) aborts cleanly.
fn spawn_background_reconcile(
    workspace_slot: Arc<Mutex<Option<Workspace>>>,
    storage_root: PathBuf,
    hlc: HlcGenerator,
) {
    thread::spawn(move || {
        let engine = outl_actions::SyncEngine::new(storage_root.clone(), hlc.actor());
        // Orphan `.md` reconcile — imported journals (Roam / Logseq),
        // peer-written `.md` without a sidecar, or files edited externally.
        // Lock PER PAGE and drop between iterations so the frontend can
        // grab the workspace between reconciles: holding one lock across a
        // 2800-page walk blocks the first journal paint (the reason this
        // runs off the boot thread at all). Mirrors the desktop's
        // `spawn_background_reconcile`.
        for path in &engine.scan_for_orphans() {
            let mut slot = workspace_slot.lock();
            let Some(ws) = slot.as_mut() else {
                return;
            };
            // Never `None` — level-3 fallout is a delete, and it has to
            // be recorded before it happens (`outl-md` hard rule).
            if let Err(e) =
                outl_md::reconcile::reconcile_md(ws, &hlc, path, Some(&engine.orphans_log()))
            {
                warn!("orphan reconcile failed for {}: {e}", path.display());
            }
        }
        // Desynced projections: snapshot the list under one brief lock,
        // then recover each under its own so first paint isn't blocked.
        let desynced = {
            let mut slot = workspace_slot.lock();
            let Some(ws) = slot.as_mut() else {
                return;
            };
            engine.scan_for_desynced_projections(ws)
        };
        for path in &desynced {
            let mut slot = workspace_slot.lock();
            let Some(ws) = slot.as_mut() else {
                return;
            };
            match outl_actions::recover_desynced_projection(ws, &hlc, &storage_root, path) {
                Ok(n) if n > 0 => info!(
                    "recovered {n} lost op(s) from desynced projection {}",
                    path.display()
                ),
                Ok(_) => {}
                Err(e) => warn!("desync recovery failed for {}: {e}", path.display()),
            }
        }
        // Repair journal titles doubled by concurrent offline creation.
        // Cheap no-op once clean; converges via the op log. One brief lock.
        {
            let mut slot = workspace_slot.lock();
            let Some(ws) = slot.as_mut() else {
                return;
            };
            match outl_actions::repair_doubled_journal_titles(ws, &hlc) {
                Ok(0) => {}
                Ok(n) => info!("repaired {n} doubled journal title(s)"),
                Err(e) => warn!("doubled-title repair: {e}"),
            }
        }
    });
}
