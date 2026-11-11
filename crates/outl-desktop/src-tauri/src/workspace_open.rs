//! Desktop boot opener + background reconcile.
//!
//! The open / actor-id / orphan-reconcile primitives live in
//! `outl_tauri_shared::workspace_open` (re-exported below). This module
//! keeps the two desktop-specific orchestrations:
//!
//! - [`spawn_workspace_opener`] — background thread, run at boot when
//!   `settings.last_workspace` is already set, so the WebView starts
//!   painting before we touch the filesystem. Starts the FS watcher and
//!   wires the iroh transport into the swap-capable `AppState` slots.
//! - [`spawn_background_reconcile`] — the desktop's one-line adapter onto
//!   `outl_tauri_shared::reconcile`, which owns the pass itself for both
//!   clients (the two copies had drifted; see that module's docs).

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use outl_actions::backup::BackupRoot;
use outl_actions::SyncTransport;
use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use parking_lot::Mutex;
use tauri::Emitter;
use tracing::{info, warn};

use crate::fs_watcher::{self, WatcherHandle};

pub(crate) use outl_tauri_shared::workspace_open::{
    load_or_create_actor, open_workspace_at, WorkspaceGuards,
};

/// Run the shared background reconcile sweep and emit
/// `workspace-reconciled` if it moved the tree.
///
/// The pass — which phases chunk their lock, which two cannot and why,
/// and why the whole-workspace ones wait out the launch window — belongs
/// to [`outl_tauri_shared::reconcile`]. Mobile runs the identical sweep;
/// the only thing that was ever desktop-specific is this event.
///
/// `AppShell.tsx` (`onReconciled`) re-reads the active page and its lazy
/// backlinks reply on it — the repaired `title::` values and the
/// nested-pages rows would otherwise stay stale on the open screen until
/// the next navigation. It fires once on completion of the sweep, not
/// per-page: keystroke-grained refreshes would be noisier than they help.
pub(crate) fn spawn_background_reconcile(
    workspace_slot: Arc<Mutex<Option<Workspace>>>,
    storage_root: PathBuf,
    hlc: HlcGenerator,
    app: tauri::AppHandle,
) {
    let _ = outl_tauri_shared::reconcile::spawn_background_reconcile(
        workspace_slot,
        storage_root,
        hlc,
        move || {
            if let Err(e) = app.emit("workspace-reconciled", ()) {
                warn!("emit workspace-reconciled: {e}");
            }
        },
    );
}

/// Start the process's one automatic-backup thread and return the cell
/// that aims it.
///
/// Called from `setup` unconditionally: `enabled` is checked inside
/// `spawn_auto_pass`, and the root is not known yet. Every successful
/// `open_workspace_at` retargets the returned [`BackupRoot`], so a
/// workspace switch moves the one thread instead of adding another.
pub(crate) fn start_backup(cfg: &outl_config::BackupCfg) -> BackupRoot {
    let root = BackupRoot::new();
    outl_actions::backup::spawn_auto_pass(root.clone(), cfg.enabled, cfg.interval_minutes);
    root
}

/// Background projection writer: mutations queue their page here so the
/// `.md` + sidecar write happens off the IPC thread (async-writes
/// default). Shares the same workspace slot + swap-capable storage root
/// the commands lock. Lives beside the opener, not in `lib.rs`, to keep
/// `setup` under the file-size ceiling.
pub(crate) fn spawn_projection_writer(
    workspace: &Arc<Mutex<Option<Workspace>>>,
    storage_root: &Arc<Mutex<Option<PathBuf>>>,
    app: &tauri::AppHandle,
) -> outl_tauri_shared::ProjectionWriter {
    let app = app.clone();
    outl_tauri_shared::ProjectionWriter::spawn(
        workspace.clone(),
        storage_root.clone(),
        move |failure| {
            if let Err(error) = app.emit(outl_tauri_shared::PROJECTION_WRITE_FAILED_EVENT, failure)
            {
                warn!("failed to emit projection failure: {error}");
            }
        },
    )
}

/// Background opener used at boot when `settings.last_workspace` is
/// already set. Runs on a worker thread so the WebView starts
/// painting immediately; the frontend polls `workspace_stats` /
/// listens for the `workspace-ready` event.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_workspace_opener(
    workspace_slot: Arc<Mutex<Option<Workspace>>>,
    storage_root_slot: Arc<Mutex<Option<PathBuf>>>,
    workspace_guards: Arc<Mutex<Option<WorkspaceGuards>>>,
    backup_root: BackupRoot,
    fs_watcher_slot: Arc<Mutex<Option<WatcherHandle>>>,
    iroh_transport_slot: Arc<Mutex<Option<Arc<dyn SyncTransport>>>>,
    iroh_pairing_slot: Arc<Mutex<Option<outl_sync_iroh::IrohSyncTransport>>>,
    last_workspace: PathBuf,
    hlc: HlcGenerator,
    app: tauri::AppHandle,
) {
    let actor = hlc.actor();
    let lru_cap = outl_config::load().storage.lru_cap;
    thread::spawn(move || {
        if !last_workspace.exists() {
            warn!(
                "last_workspace {} no longer exists; user must re-pick",
                last_workspace.display()
            );
            return;
        }
        // Fast open: ops/, journals/, pages/ exist; today's journal is
        // resolved; legacy blocks moved under it. **No orphan
        // reconcile here** — that runs after we publish the workspace
        // so the user sees today's journal immediately.
        let workspace = match open_workspace_at(
            actor,
            &hlc,
            &last_workspace,
            lru_cap,
            &workspace_guards,
            &backup_root,
        ) {
            Ok(w) => w,
            Err(e) => {
                warn!(
                    "background open failed for {}: {e}",
                    last_workspace.display()
                );
                return;
            }
        };
        // Start the FS watcher BEFORE we swap the slot so the first
        // peer change after boot is captured. Failure is logged but
        // non-fatal — the user can still work, just without
        // automatic reload.
        match fs_watcher::start_watcher(&last_workspace, actor, app.clone()) {
            Ok(handle) => fs_watcher::swap_watcher(&fs_watcher_slot, Some(handle)),
            Err(e) => warn!("fs watcher failed to start: {e}"),
        }
        *workspace_slot.lock() = Some(workspace);
        *storage_root_slot.lock() = Some(last_workspace.clone());
        if let Err(e) = app.emit("workspace-ready", ()) {
            warn!("emit workspace-ready: {e}");
        }
        info!("background workspace opener complete");

        // Wire the iroh P2P transport (best-effort, gated on
        // `[sync] transport = "iroh"`). It runs ALONGSIDE the `notify`
        // watcher started above: both deliver peer ops to `<root>/ops/`
        // and both surface as `peer-ops-changed`, so the frontend
        // reload path is identical whichever wins the race. A `File`
        // config or any build failure is a silent no-op here.
        crate::iroh_sync::wire_iroh_transport(
            &iroh_transport_slot,
            &iroh_pairing_slot,
            last_workspace.clone(),
            actor,
            app.clone(),
        );

        // Background reconcile: scan and reconcile orphan `.md` files in yet
        // another thread, releasing the workspace lock between each
        // page. The frontend can already query the workspace; pages
        // that get materialised by the reconcile become visible on
        // the next read (no full reload required). The reconcile
        // emits `workspace-reconciled` on completion if a client
        // wants to explicitly refresh the current view.
        spawn_background_reconcile(workspace_slot, last_workspace, hlc, app);
    });
}
