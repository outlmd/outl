//! Workspace-index rebuild lifecycle.
//!
//! The index ([`outl_md::index::WorkspaceIndex`]) powers page-title
//! autocomplete, the quick-switcher icon column, and block-ref
//! resolution (`((blk-XXXXXX))`). Backlinks no longer live here — they
//! come from a separate `outl_actions::BacklinkIndex` built on its own
//! worker thread (`App::spawn_backlink_index_rebuild`), mirroring the
//! pattern in this module.
//!
//! Building the index walks every `.md` under `pages/` + `journals/`,
//! which is cheap for small workspaces but blocks the UI thread on
//! large ones — so production callers always go through the spawn
//! path and poll the channel from the event loop.

use crate::state::App;
use outl_md::index::WorkspaceIndex;

impl App {
    /// Synchronous workspace-index build. Kept around as escape hatch
    /// for code paths that genuinely need the index *right now*
    /// (none today — production callers use
    /// [`Self::spawn_index_rebuild`]). Avoid in hot paths; it blocks
    /// the event loop while it walks the whole workspace.
    #[allow(dead_code)]
    pub(crate) fn rebuild_index(&mut self) {
        self.index = WorkspaceIndex::build(&self.workspace_root);
        // Cancel any pending background build — we just produced a
        // fresher result. The thread keeps running but its send goes
        // to a dropped receiver.
        self.index_rx = None;
    }

    /// Kick off a workspace-index rebuild on a worker thread.
    ///
    /// Replaces any in-flight build (the previous thread's result is
    /// dropped on arrival). The next call to
    /// [`Self::poll_index_updates`] swaps in the result when ready.
    pub(crate) fn spawn_index_rebuild(&mut self) {
        let root = self.workspace_root.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("outl-index".into())
            .spawn(move || {
                let idx = WorkspaceIndex::build(&root);
                // If the receiver was dropped (newer spawn superseded
                // us), this just returns Err — fine.
                let _ = tx.send(idx);
            })
            .expect("spawning the index worker thread should not fail");
        self.index_rx = Some(rx);
    }

    /// `true` while a workspace-index rebuild is in flight on a worker
    /// thread. The event loop uses this to shorten its
    /// `event::poll` timeout so the freshly-built index shows up in
    /// the UI within a frame, not after the next 750 ms key timeout.
    pub(crate) fn has_pending_index(&self) -> bool {
        self.index_rx.is_some()
    }

    /// Non-blocking check: if the background index build has finished,
    /// swap the result into `self.index`. Returns `true` when a swap
    /// happened so the event loop can request a redraw.
    pub(crate) fn poll_index_updates(&mut self) -> bool {
        let Some(rx) = &self.index_rx else {
            return false;
        };
        match rx.try_recv() {
            Ok(idx) => {
                self.index = idx;
                self.index_rx = None;
                // A query typed while the build was in flight was
                // filtered against the old index; re-run it.
                self.refresh_search();
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // Worker died (panic, OOM, ...). Stop polling; the
                // TUI keeps working with the current (possibly empty)
                // index.
                self.index_rx = None;
                false
            }
        }
    }

    /// Full workspace re-read: page list, current view's `.md` from
    /// disk, and the derived index. Used when an external process
    /// (another editor, `outl serve`) may have changed files under us.
    pub(crate) fn refresh_workspace(&mut self) {
        self.refresh_page_list();
        self.load_current();
        self.spawn_index_rebuild();
    }
}
