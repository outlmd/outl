//! The backlink index: who points at the open page, and which way that
//! list is sorted.
//!
//! Nothing here navigates. Moving the cursor *into* a backlink is
//! `super::selection`; this module owns the index itself — the two read
//! accessors every call site routes through, the background build, the
//! incremental per-page reindex, and the persisted sort direction.

use crate::state::App;

impl App {
    /// Compute the backlinks pointing at `slug` from the pre-built
    /// index. **This is the single source for backlinks across the
    /// TUI** — every call site (panel render, navigation, keyboard
    /// handlers) routes through here.
    ///
    /// The index is built **on a worker thread**
    /// ([`Self::spawn_backlink_index_rebuild`]); until the first build
    /// lands the index is `None` and this returns empty (the panel and
    /// footer count just show nothing for a beat). Reading 2800+ `.md`
    /// inline on the event loop was the open/Esc freeze — see the field
    /// doc on [`crate::state::App::backlink_index`]. A local edit patches
    /// the current page in place ([`Self::reindex_backlinks_for_slug`]);
    /// whole-workspace changes re-spawn the background build.
    pub(crate) fn backlinks_for_slug(&self, slug: &str) -> Vec<outl_actions::Backlink> {
        let borrow = self.backlink_index.borrow();
        let Some(index) = borrow.as_ref() else {
            return Vec::new();
        };
        let Some(id) = outl_actions::find_by_slug(&self.workspace, slug) else {
            return Vec::new();
        };
        let Some(meta) = outl_actions::page_meta(&self.workspace, id) else {
            return Vec::new();
        };
        let mut links = index.for_page(&self.workspace, &meta);
        // Order per the user's preference (issue #142).
        outl_actions::sort_backlinks(&mut links, self.backlinks_newest_first);
        links
    }

    /// Kick off a whole-workspace backlink-index build on a worker
    /// thread, mirroring [`Self::spawn_index_rebuild`]. Reads every `.md`
    /// (`build_backlink_index_from_disk`) — `Send`, no `Workspace`, no
    /// lock. Inline it froze the open on a large vault (2800+ `.md` on
    /// the event-loop thread); a worker keeps the journal paintable and
    /// fills the panel a beat later. Replaces any in-flight build (the
    /// previous thread's result is dropped on arrival); the **old** index
    /// stays live until the new one lands, so the panel doesn't blank.
    /// Also drops the nested-pages memo: it goes stale at the same moments.
    pub(crate) fn spawn_backlink_index_rebuild(&mut self) {
        self.invalidate_namespace_children();
        let metas = outl_actions::list_pages(&self.workspace);
        let root = self.workspace_root.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("outl-backlinks".into())
            .spawn(move || {
                let idx = outl_actions::build_backlink_index_from_disk(&metas, &root);
                // Err just means a newer spawn dropped the receiver.
                let _ = tx.send(idx);
            })
            .expect("spawning the backlink-index worker thread should not fail");
        self.backlink_index_rx = Some(rx);
    }

    /// `true` while a backlink-index build is in flight on a worker
    /// thread. The event loop shortens its `event::poll` timeout so the
    /// freshly-built index shows up within a frame.
    pub(crate) fn has_pending_backlink_index(&self) -> bool {
        self.backlink_index_rx.is_some()
    }

    /// Non-blocking check: if the background backlink-index build has
    /// finished, swap the result into `self.backlink_index`. Returns
    /// `true` when a swap happened so the event loop can redraw.
    pub(crate) fn poll_backlink_index_updates(&mut self) -> bool {
        let Some(rx) = &self.backlink_index_rx else {
            return false;
        };
        match rx.try_recv() {
            Ok(idx) => {
                *self.backlink_index.borrow_mut() = Some(idx);
                self.backlink_index_rx = None;
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // Worker died; stop polling. The TUI keeps the current
                // (possibly empty) index.
                self.backlink_index_rx = None;
                false
            }
        }
    }

    /// Incrementally re-index the backlinks of a single edited page.
    ///
    /// The commit path (`save`) calls this instead of dropping the whole
    /// index: editing one page only changes that page's referencing
    /// blocks, so re-reading its one `.md` (`O(one page)`) is enough. The
    /// old `invalidate` rebuilt the index from EVERY `.md` inline on the
    /// event loop — the "Esc is slow in the TUI" bug. A no-op when the
    /// index isn't built yet (`None`). The page's `.md`/`.outl` must be
    /// projected first. The nested-pages memo is dropped unconditionally:
    /// a commit can rewrite this page's `title::`.
    pub(crate) fn reindex_backlinks_for_slug(&self, slug: &str) {
        self.invalidate_namespace_children();
        let mut guard = self.backlink_index.borrow_mut();
        let Some(index) = guard.as_mut() else {
            return;
        };
        let Some(id) = outl_actions::find_by_slug(&self.workspace, slug) else {
            return;
        };
        let Some(meta) = outl_actions::page_meta(&self.workspace, id) else {
            return;
        };
        index.reindex_page_from_disk(&meta, &self.workspace_root);
    }

    /// Convenience: backlinks for the currently-opened page/journal.
    pub(crate) fn backlinks_for_current(&self) -> Vec<outl_actions::Backlink> {
        self.backlinks_for_slug(&self.current_slug())
    }

    /// Number of backlinks pointing at `slug`, without cloning the
    /// list.
    ///
    /// Callers that only need a count (the footer chip in
    /// `view::chrome`, the navigability probe in
    /// [`Self::backlinks_navigable`]) take this instead of
    /// [`Self::backlinks_for_slug`]. The rich `Backlink` struct
    /// carries `source_block: OutlineNode` plus its subtree, so the
    /// clone the full accessor performs is non-trivial. This counts via
    /// the index's `count_for_page`, which dedupes the hit positions
    /// without cloning a single `Backlink`.
    pub(crate) fn backlinks_count_for_slug(&self, slug: &str) -> usize {
        let borrow = self.backlink_index.borrow();
        let Some(index) = borrow.as_ref() else {
            return 0;
        };
        let Some(id) = outl_actions::find_by_slug(&self.workspace, slug) else {
            return 0;
        };
        let Some(meta) = outl_actions::page_meta(&self.workspace, id) else {
            return 0;
        };
        index.count_for_page(&self.workspace, &meta)
    }

    /// Convenience: number of backlinks for the currently-opened
    /// page/journal. See [`Self::backlinks_count_for_slug`].
    pub(crate) fn backlinks_count_for_current(&self) -> usize {
        self.backlinks_count_for_slug(&self.current_slug())
    }

    /// Flip the backlinks list direction (newest ⇄ oldest, issue #142)
    /// and persist it to `~/.config/outl/config.toml` so it survives a
    /// restart. Persistence is best-effort — a write failure only shows
    /// in the status line, the in-session flip still takes effect. No
    /// index rebuild: `for_page` applies `sort_backlinks` on every read,
    /// so the next render already re-sorts with the flipped flag.
    pub(crate) fn toggle_backlinks_order(&mut self) {
        self.backlinks_newest_first = !self.backlinks_newest_first;

        let mut cfg = outl_config::load();
        cfg.display.backlinks_order = if self.backlinks_newest_first {
            outl_config::BacklinksOrder::Newest
        } else {
            outl_config::BacklinksOrder::Oldest
        };
        let label = if self.backlinks_newest_first {
            "newest first"
        } else {
            "oldest first"
        };
        // The error already reads "settings not saved: <path> could not be
        // read (line N: …). Fix that file, then try again" — don't wrap it
        // in a second "not saved", and don't swallow it: this status line
        // is the only place a TUI user finds out that `config.toml` stopped
        // being writable (issue #284).
        self.status = match outl_config::save(&cfg) {
            Ok(()) => format!("backlinks: {label}"),
            Err(e) => format!("backlinks: {label} — {e}"),
        };
    }
}
