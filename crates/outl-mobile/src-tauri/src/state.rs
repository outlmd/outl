//! Mobile `AppState` + its [`AppHost`] projection.
//!
//! The wire types (`PageView`, `CreateBlockReply`, `WorkspaceSummary`,
//! `ERR_LOADING`) live in `outl-tauri-shared` and are re-exported here
//! so the rest of the crate keeps importing them from `crate::state`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{BacklinkIndex, HistoryStacks, SyncTransport};
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_sync_iroh::IrohSyncTransport;
use outl_tauri_shared::{AppHost, ProjectionWriter};
use parking_lot::Mutex;

pub(crate) use outl_tauri_shared::{CreateBlockReply, CutBlockReply, PageView, WorkspaceSummary};

/// Shared mutable state held by Tauri.
///
/// Note: `storage_root` is a plain `PathBuf` (resolved on boot inside
/// `workspace_open::resolve_storage_root`) — unlike the desktop crate,
/// the mobile client always has a workspace path: the folder the user
/// previously chose (`WorkspaceCfg.last`), or the app-local default
/// `<app-data-dir>/outl/` on a fresh install. Don't copy desktop's
/// `Arc<Mutex<Option<PathBuf>>>` shape — the single-root divergence is
/// deliberate and absorbed by the shared `AppHost` trait. Switching
/// folders at runtime is a `workspace-reopen-required` event + relaunch
/// today (see `workspace_picker`); a live swap would be what flips this
/// to the desktop shape.
pub(crate) struct AppState {
    /// `None` until the background opener completes.
    pub(crate) workspace: Arc<Mutex<Option<Workspace>>>,
    pub(crate) hlc: HlcGenerator,
    pub(crate) storage_root: PathBuf,
    /// Code-block runtimes built once at startup. Shared between
    /// every `run_code_block` invocation. Kept behind `Arc` so future
    /// `spawn_blocking` callers can clone-and-move without holding
    /// the workspace mutex.
    pub(crate) registry: Arc<RuntimeRegistry>,
    /// The running iroh P2P sync transport, when wired at boot.
    ///
    /// This is a **lifetime guard**: the transport's `start()` spawns a
    /// detached `outl-iroh-sync` thread that owns the tokio runtime via
    /// internal `Arc`s, but the `shutdown_tx` / `announce_tx` handles
    /// live on this struct. Keeping the value here means the app holds a
    /// handle for its whole lifetime; if we dropped it the transport
    /// would stay running but lose any future `shutdown()` / announce
    /// path. `None` when iroh is disabled in config or failed to bind
    /// (the app then simply runs without P2P sync until a later relaunch
    /// retries).
    ///
    /// Read through `AppHost::sync_transport` by the shared peer-status /
    /// force-sync / announce paths, cloned by the pairing commands, and
    /// shut down gracefully in `Drop`.
    pub(crate) iroh: Option<IrohSyncTransport>,
    /// Per-page undo / redo stacks of rendered `.md` snapshots
    /// (`outl_actions::history::HistoryStacks`), mirroring the desktop's
    /// field 1:1 (RFC 0254 phase 1 — the undo/redo commands were shared
    /// logic trapped behind a desktop-only registration, see
    /// `commands::history`). `finish_in_page_with` records the
    /// pre-mutation render; `undo_page` / `redo_page` restore one
    /// through `outl_actions::restore_page_md` (new ops in the log,
    /// never a rewrite). Cleared wholesale on the one event mobile has
    /// that's equivalent to desktop's workspace switch: there isn't one
    /// today (a folder swap is a relaunch, see the struct doc above), so
    /// in practice these stacks live for the whole process lifetime.
    pub(crate) history: Mutex<HashMap<NodeId, HistoryStacks<String>>>,
    /// Pre-computed backlinks index over the whole workspace. `None`
    /// while stale — `finish_in_page` clears it after a mutation and the
    /// peer reload clears it too, so the next `page_backlinks` rebuilds
    /// it (off the IPC thread) and every later navigation is an
    /// `O(refs)` lookup instead of a full `O(blocks)` re-scan.
    pub(crate) backlink_index: Arc<Mutex<Option<BacklinkIndex>>>,
    /// Background `.md` + sidecar projection writer. Mutations queue
    /// their page here instead of rendering + hashing + writing inline,
    /// so a commit never blocks the next tap on disk I/O (the outl
    /// async-writes default). See [`outl_tauri_shared::projection`].
    pub(crate) projection_writer: ProjectionWriter,
}

impl Drop for AppState {
    fn drop(&mut self) {
        // Make the "graceful shutdown() on app exit" the field doc promises real.
        // The detached transport thread would die with the process regardless,
        // but shutdown() releases the relay route right away instead of waiting
        // for the OS to reap the socket — so another process on this device
        // reclaims the route immediately.
        if let Some(transport) = &self.iroh {
            transport.shutdown();
        }
    }
}

/// The mobile projection onto the shared command surface: the root is
/// fixed for the process lifetime (`storage_root()` never errors), the
/// concrete transport is wrapped as `Arc<dyn SyncTransport>` on demand,
/// and `history()` wires the same per-page undo/redo stacks the desktop
/// has (RFC 0254 phase 1).
impl AppHost for AppState {
    fn workspace(&self) -> &Mutex<Option<Workspace>> {
        &self.workspace
    }

    fn workspace_arc(&self) -> std::sync::Arc<Mutex<Option<Workspace>>> {
        self.workspace.clone()
    }

    fn hlc(&self) -> &HlcGenerator {
        &self.hlc
    }

    fn storage_root(&self) -> Result<PathBuf, String> {
        Ok(self.storage_root.clone())
    }

    fn sync_transport(&self) -> Option<Arc<dyn SyncTransport>> {
        self.iroh
            .clone()
            .map(|t| Arc::new(t) as Arc<dyn SyncTransport>)
    }

    fn exec_registry(&self) -> Arc<RuntimeRegistry> {
        self.registry.clone()
    }

    fn history(&self) -> Option<&Mutex<HashMap<NodeId, HistoryStacks<String>>>> {
        Some(&self.history)
    }

    fn backlink_index(&self) -> Option<Arc<Mutex<Option<BacklinkIndex>>>> {
        Some(self.backlink_index.clone())
    }

    fn projection_writer(&self) -> Option<&ProjectionWriter> {
        Some(&self.projection_writer)
    }
}
