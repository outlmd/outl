//! The traits that absorb the desktop ↔ mobile structural divergence.
//!
//! The only real difference between the two `src-tauri` backends is how
//! the storage root is held: the desktop can swap workspaces at runtime
//! (`Arc<Mutex<Option<PathBuf>>>`), the mobile resolves one folder at boot
//! (`PathBuf`; switching is a relaunch). Everything else the shared
//! command bodies need is uniform and exposed through [`AppHost`].

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{BacklinkIndex, HistoryStacks, SyncTransport};
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use parking_lot::Mutex;

/// Implemented by each client's `AppState` so the shared command bodies
/// (generic over `S: AppHost`) can run against either backend.
pub trait AppHost: Send + Sync {
    /// The active workspace slot. `None` until the background opener (or
    /// the user's picker) publishes one.
    fn workspace(&self) -> &Mutex<Option<Workspace>>;

    /// A cloneable handle to the same workspace slot, so a heavy read can
    /// be moved onto a blocking pool thread instead of running on the
    /// Tauri IPC/main thread. The backlinks path (`page_backlinks`) needs
    /// an owned `Arc` to take a brief lock off the IPC thread for the page
    /// list + lookup; the `O(blocks)` index rebuild itself runs from disk
    /// with no lock at all (see `commands::page::compute_backlinks_offloaded`).
    fn workspace_arc(&self) -> Arc<Mutex<Option<Workspace>>>;

    /// Per-device HLC generator (actors identify devices, not workspaces).
    fn hlc(&self) -> &HlcGenerator;

    /// Filesystem root of the active workspace (parent of `ops/`,
    /// `journals/`, `pages/`). Errors with the [`crate::ERR_LOADING`]
    /// sentinel while unresolved (desktop before a workspace is picked);
    /// infallible on clients that resolve the root at boot (mobile).
    fn storage_root(&self) -> Result<PathBuf, String>;

    /// The live P2P sync transport, when wired. Used by the post-commit
    /// announce and by the peer status / force-sync commands. `None` for
    /// the file transport (or before the transport comes up).
    fn sync_transport(&self) -> Option<Arc<dyn SyncTransport>>;

    /// Code-block runtimes built once at startup.
    fn exec_registry(&self) -> Arc<RuntimeRegistry>;

    /// Per-page undo / redo stacks, when the client supports block-level
    /// undo (desktop). `None` (the default) skips snapshot recording in
    /// [`crate::helpers::finish_in_page_with`] entirely — no per-mutation
    /// render is paid on clients without undo.
    fn history(&self) -> Option<&Mutex<HashMap<NodeId, HistoryStacks<String>>>> {
        None
    }

    /// Pre-computed backlinks index slot, when the client caches one.
    ///
    /// `Some` lets `page_backlinks` serve a page's backlinks as an
    /// `O(refs)` lookup and rebuild the `O(blocks)` index only when it's
    /// stale (`None` inside the slot), instead of re-scanning the whole
    /// workspace on every navigation. `finish_in_page` invalidates it
    /// after a mutation. The default `None` keeps the pre-index
    /// behaviour (scan per `page_backlinks` call) for any host that
    /// hasn't wired the slot.
    fn backlink_index(&self) -> Option<Arc<Mutex<Option<BacklinkIndex>>>> {
        None
    }

    /// Background `.md` + sidecar projection writer, when the client
    /// wires one. `Some` makes [`crate::helpers::finish_in_page_with`]
    /// build the reply view from the tree and **queue** the projection
    /// off-thread instead of rendering + hashing + writing it inline —
    /// the async-writes default (see [`crate::projection`]). The default
    /// `None` keeps the synchronous projection for any host that hasn't
    /// wired the worker.
    fn projection_writer(&self) -> Option<&crate::projection::ProjectionWriter> {
        None
    }

    /// The gate that holds back everything a launch does not need on
    /// screen — the iroh endpoint, the filesystem watcher, the plugin
    /// host, the backup pass, the orphan sweep, the snapshot refresh.
    ///
    /// `Some` lets [`crate::commands::boot::boot_mark`] release that work
    /// the moment the frontend reports a painted frame. The default
    /// `None` means a host wires none of it through the gate and every
    /// job starts as soon as the opener publishes — the pre-gate
    /// behaviour, which is correct and slower, never silently broken.
    ///
    /// A host that returns `Some` **must** have armed the gate's deadline
    /// ([`crate::first_paint::FirstPaintGate::arm_deadline`]): a client
    /// whose webview never reports is otherwise an app that syncs with
    /// nobody and says nothing.
    fn first_paint_gate(&self) -> Option<&Arc<crate::first_paint::FirstPaintGate>> {
        None
    }

    /// The latch a client waits on instead of polling for the workspace.
    ///
    /// `Some` makes [`crate::commands::boot::wait_workspace_ready`] a
    /// real wait; the default `None` makes it a single slot read, which
    /// is what a client that still polls gets. The opener is the only
    /// writer — see [`crate::boot_ready::WorkspaceReady`].
    fn workspace_ready(&self) -> Option<&Arc<crate::boot_ready::WorkspaceReady>> {
        None
    }
}

/// Owned handle to "the current storage root" the plugin thread can move
/// into itself. Mirrors the [`AppHost::storage_root`] divergence for code
/// that runs off the Tauri state (the dedicated plugin thread).
pub trait StorageRootProvider: Send + 'static {
    /// The root right now; `None` while no workspace is open. A provider
    /// whose value can change between calls (the desktop's swap-capable
    /// slot) makes the plugin host reload against the new root.
    fn current(&self) -> Option<PathBuf>;
}

/// Mobile: one root for the process lifetime.
impl StorageRootProvider for PathBuf {
    fn current(&self) -> Option<PathBuf> {
        Some(self.clone())
    }
}

/// Desktop: the same swap-capable slot `AppState` holds.
impl StorageRootProvider for Arc<Mutex<Option<PathBuf>>> {
    fn current(&self) -> Option<PathBuf> {
        self.lock().clone()
    }
}
