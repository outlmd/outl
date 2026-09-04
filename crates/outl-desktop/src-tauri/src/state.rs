//! Desktop `AppState` + its [`AppHost`] projection.
//!
//! The wire types (`PageView`, `CreateBlockReply`, `WorkspaceSummary`,
//! `ERR_LOADING`) live in `outl-tauri-shared` and are re-exported here so
//! the rest of the crate keeps importing them from `crate::state`.
//!
//! `AppState` is created once at `setup` time and managed by Tauri so
//! every command has access via `State<'_, AppState>`. The mutable
//! fields are wrapped in `parking_lot::Mutex` because the user can
//! swap workspaces at runtime and the background opener thread writes
//! into them.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{BacklinkIndex, HistoryStacks, SyncTransport};
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::{AppHost, ProjectionWriter};
use parking_lot::Mutex;

use crate::fs_watcher::WatcherHandle;
use crate::settings::Settings;

pub(crate) use outl_tauri_shared::{
    CreateBlockReply, CutBlockReply, PageView, WorkspaceSummary, ERR_LOADING,
};

/// Shared mutable state held by Tauri.
pub(crate) struct AppState {
    /// The active workspace. `None` until [`crate::commands::workspace::set_workspace`]
    /// or the background opener completes.
    pub workspace: Arc<Mutex<Option<Workspace>>>,
    /// Filesystem root of the active workspace (parent of `ops/`,
    /// `journals/`, `pages/`). Tracks `workspace` 1:1.
    pub storage_root: Arc<Mutex<Option<PathBuf>>>,
    /// Per-device HLC generator. Static for the process lifetime —
    /// actors identify devices, not workspaces.
    pub hlc: HlcGenerator,
    /// User preferences and last opened workspace path.
    pub settings: Arc<Mutex<Settings>>,
    /// Where `settings.json` and `actor` live
    /// (`app.path().app_config_dir()`).
    pub app_config_dir: PathBuf,
    /// Code-block runtimes built once at startup. Shared between
    /// every `run_code_block` invocation. `Arc` so the command can
    /// clone-and-move into `spawn_blocking` without locking.
    pub registry: Arc<RuntimeRegistry>,
    /// Live filesystem watcher for the active workspace. Dropped
    /// (replaced) when the user switches workspaces; `None` while
    /// the picker is still up.
    pub fs_watcher: Arc<Mutex<Option<WatcherHandle>>>,
    /// Active P2P sync transport (`outl_sync_iroh::IrohSyncTransport`),
    /// when the config opts into `[sync] transport = "iroh"`. `None`
    /// for the default filesystem transport (the `notify` watcher
    /// covers change detection in that case).
    ///
    /// Held behind a `Mutex` because it's wired in on a background
    /// thread (the boot opener) after `AppState` is constructed, and
    /// the pairing / shutdown commands read it back. Stored as the
    /// `dyn SyncTransport` trait object so `announce_local_ops` /
    /// `shutdown` are reachable without re-importing the concrete type.
    pub iroh_transport: Arc<Mutex<Option<Arc<dyn SyncTransport>>>>,
    /// The same iroh transport as `iroh_transport`, but kept as the **concrete**
    /// type so the pairing commands can call `pair_host` / `pair_join` (which
    /// reuse the live sync endpoint — there is no separate pairing endpoint).
    /// `None` for the filesystem transport. Tracks `iroh_transport` 1:1; held
    /// concretely because pairing isn't a `SyncTransport` trait concern (the
    /// trait can't return `outl_sync_iroh::PeerEntry` without a dep cycle).
    pub iroh_pairing: Arc<Mutex<Option<outl_sync_iroh::IrohSyncTransport>>>,
    /// Per-page undo / redo stacks of rendered `.md` snapshots
    /// (`outl_actions::history::HistoryStacks`). `finish_in_page_with`
    /// records the pre-mutation render; `undo_page` / `redo_page`
    /// restore one through `outl_actions::restore_page_md` (the
    /// restore is ops in the log, never a rewrite). Fully cleared on
    /// workspace **switch**; on a peer-driven **reload** only the
    /// pages whose projection changed lose their stacks (see
    /// `commands::workspace::reload_workspace`).
    pub history: Mutex<HashMap<NodeId, HistoryStacks<String>>>,
    /// Pre-computed backlinks index over the whole workspace. `None`
    /// while stale — `finish_in_page` clears it after a mutation and the
    /// peer reload clears it too, so the next `page_backlinks` rebuilds
    /// it (off the IPC thread) and every later navigation is an
    /// `O(refs)` lookup instead of a full `O(blocks)` re-scan.
    pub backlink_index: Arc<Mutex<Option<BacklinkIndex>>>,
    /// Background `.md` + sidecar projection writer. Every mutation
    /// queues its page here instead of rendering + hashing + writing
    /// inline, so the commit never blocks the next keystroke on disk I/O
    /// (the outl async-writes default). See
    /// [`outl_tauri_shared::projection`].
    pub projection_writer: ProjectionWriter,
}

/// The desktop's projection onto the shared command surface. The one
/// structural divergence from mobile — the swap-capable
/// `Arc<Mutex<Option<PathBuf>>>` storage root — is absorbed by
/// `storage_root()` returning the [`ERR_LOADING`] sentinel while no
/// workspace is open.
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
        self.storage_root
            .lock()
            .clone()
            .ok_or_else(|| ERR_LOADING.to_string())
    }

    fn sync_transport(&self) -> Option<Arc<dyn SyncTransport>> {
        self.iroh_transport.lock().clone()
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
