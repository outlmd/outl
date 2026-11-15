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

use outl_tauri_shared::{FirstPaintGate, WorkspaceReady};

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

/// How long the interactive phase waits for a painted frame before
/// starting anyway, counted **from the moment the workspace is
/// published**.
///
/// It exists because the painted signal arrives over IPC: a webview
/// that failed to load, or a frontend built before `boot_mark` existed,
/// leaves it unsent, and without a deadline that is a window which
/// syncs with nobody and says nothing.
///
/// **Armed at publish, not at `setup`.** Armed at `setup` it raced the
/// *open* rather than the paint: on the 722,943-op workspace this work
/// was measured against, the open takes 9s, so a 4s deadline fired
/// while the gate was still empty and every job the opener registered
/// afterwards ran immediately — in front of the first paint, which is
/// the stall the gate exists to move. Counting from publish measures
/// the thing the gate is actually waiting for: how long after the
/// workspace is available the client takes to draw.
const FIRST_PAINT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(4);

/// Build the two boot signals `AppState` carries.
///
/// The gate's deadline is **not** armed here; [`spawn_interactive_phase`]
/// arms it once the jobs exist, for the reason on
/// [`FIRST_PAINT_DEADLINE`]. A workspace that never opens never arms
/// one, which is correct: there is nothing deferred to release.
pub(crate) fn start_boot_phase() -> (Arc<FirstPaintGate>, Arc<WorkspaceReady>) {
    (FirstPaintGate::new(), Arc::new(WorkspaceReady::new()))
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
/// painting immediately.
///
/// # What publishes, and what waits
///
/// Three things happen the moment the open returns, and nothing else:
/// the slots are filled, `workspace-ready` is emitted, and
/// [`outl_tauri_shared::WorkspaceReady`] is latched so a frontend
/// blocked in `wait_workspace_ready` wakes. That is the whole set the
/// first frame depends on.
///
/// Everything else is registered on the [`FirstPaintGate`] and starts
/// once the frontend reports `first_content` (or the gate's deadline
/// elapses): the filesystem watcher, the iroh transport, the orphan
/// reconcile, and the snapshot refresh. All four read the same disk and
/// take the same workspace lock the first journal paint needs, and
/// running them on a worker thread does not make them invisible — the
/// premise is the *device*, which is the rule
/// [`outl_tauri_shared::reconcile`] already carries for the sweep.
///
/// **Deferred is not cancelled**, and the gate's deadline is what makes
/// that true rather than aspirational.
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
    first_paint: Arc<FirstPaintGate>,
    workspace_ready: Arc<WorkspaceReady>,
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
        // Today's journal, off disk, **before** the open.
        //
        // This is the whole point of the page-first boot: the `.md` is
        // the projection of exactly the page the user is waiting for,
        // and reading it with its sidecar costs 8-11ms whatever the
        // workspace holds — against 163ms of snapshot boot plus the
        // index load on a real 730k-op graph, and seconds on a cold
        // one. The client paints real text here and swaps it for the
        // live view when the open lands. See `page_preview`.
        outl_tauri_shared::page_preview::emit_today_journal_preview(&app, &last_workspace);

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
        *workspace_slot.lock() = Some(workspace);
        *storage_root_slot.lock() = Some(last_workspace.clone());
        if let Err(e) = app.emit("workspace-ready", ()) {
            warn!("emit workspace-ready: {e}");
        }
        // After the slot, never before: a waiter that wakes to
        // `ERR_LOADING` is the poll this replaces, one line earlier.
        workspace_ready.publish();
        info!("background workspace opener complete");

        spawn_interactive_phase(
            &first_paint,
            InteractivePhase {
                workspace_slot,
                fs_watcher_slot,
                iroh_transport_slot,
                iroh_pairing_slot,
                root: last_workspace,
                actor,
                hlc,
                app,
            },
        );
    });
}

/// What the opener hands the gate. A struct because the five slots plus
/// the root, the actor, the clock and the app handle is past the point
/// where positional arguments read.
pub(crate) struct InteractivePhase {
    pub workspace_slot: Arc<Mutex<Option<Workspace>>>,
    pub fs_watcher_slot: Arc<Mutex<Option<WatcherHandle>>>,
    pub iroh_transport_slot: Arc<Mutex<Option<Arc<dyn SyncTransport>>>>,
    pub iroh_pairing_slot: Arc<Mutex<Option<outl_sync_iroh::IrohSyncTransport>>>,
    pub root: PathBuf,
    pub actor: outl_core::id::ActorId,
    pub hlc: HlcGenerator,
    pub app: tauri::AppHandle,
}

/// Register the four deferred boot jobs on `gate`, in the order they
/// should start.
///
/// Watcher first: it is the cheapest and the only one whose *delay* can
/// lose a notification, so it closes its own window (below) and then
/// the expensive three follow.
pub(crate) fn spawn_interactive_phase(gate: &Arc<FirstPaintGate>, phase: InteractivePhase) {
    // Armed first, so a panic registering a job below cannot leave the
    // gate closed forever. Idempotent if the client already painted.
    gate.arm_deadline(FIRST_PAINT_DEADLINE);
    let InteractivePhase {
        workspace_slot,
        fs_watcher_slot,
        iroh_transport_slot,
        iroh_pairing_slot,
        root,
        actor,
        hlc,
        app,
    } = phase;

    // The watcher used to start *before* the slot was filled, so that
    // "the first peer change after boot is captured". Deferring it opens
    // a window where a peer writes and no `notify` event is delivered.
    //
    // That window is closed here rather than by starting earlier: a
    // watcher only reports what happens *after* it starts, so the
    // question "did the log move while nobody was watching" has to be
    // asked of the log. `ops_watermark` is that question, and the boot
    // already answered it once (`open_workspace_at` calls
    // `note_current`), so a `Some` here means a peer appended between
    // the open and this moment — exactly what the watcher would have
    // told us.
    //
    // A missed notification was never data loss (the ops are on disk and
    // the next reload reads them); it was a page that does not refresh
    // until the user does something. This keeps it from being either.
    {
        let root = root.clone();
        let app = app.clone();
        gate.on_open("fs_watcher", move || {
            match fs_watcher::start_watcher(&root, actor, app.clone()) {
                Ok(handle) => fs_watcher::swap_watcher(&fs_watcher_slot, Some(handle)),
                // Non-fatal: the user can still work, just without
                // automatic reload.
                Err(e) => warn!("fs watcher failed to start: {e}"),
            }
            let ops = root.join("ops");
            if outl_tauri_shared::ops_watermark::read_needed(&ops, actor).is_some() {
                info!("boot: op log moved before the watcher started; signalling a reload");
                if let Err(e) = app.emit("peer-ops-changed", ()) {
                    warn!("emit peer-ops-changed: {e}");
                }
            }
        });
    }

    // Re-persist the boot cache. Queued before iroh and the reconcile
    // because it is the shortest of the three and the next launch is
    // what pays for skipping it; `boot` deliberately no longer does this
    // inline (see `outl_tauri_shared::workspace_open`).
    {
        let slot = workspace_slot.clone();
        gate.on_open("snapshot_refresh", move || {
            outl_tauri_shared::workspace_open::snapshot_after_first_paint(&slot);
        });
    }

    // Wire the iroh P2P transport (best-effort, gated on
    // `[sync] transport = "iroh"`). It runs ALONGSIDE the `notify`
    // watcher started above: both deliver peer ops to `<root>/ops/`
    // and both surface as `peer-ops-changed`, so the frontend
    // reload path is identical whichever wins the race. A `File`
    // config or any build failure is a silent no-op here.
    {
        let root = root.clone();
        let app = app.clone();
        gate.on_open("iroh_transport", move || {
            crate::iroh_sync::wire_iroh_transport(
                &iroh_transport_slot,
                &iroh_pairing_slot,
                root,
                actor,
                app,
            );
        });
    }

    // Background reconcile: scan and reconcile orphan `.md` files in yet
    // another thread, releasing the workspace lock between each
    // page. The frontend can already query the workspace; pages
    // that get materialised by the reconcile become visible on
    // the next read (no full reload required). The reconcile
    // emits `workspace-reconciled` on completion if a client
    // wants to explicitly refresh the current view.
    gate.on_open("orphan_reconcile", move || {
        spawn_background_reconcile(workspace_slot, root, hlc, app);
    });
}
