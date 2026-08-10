//! Render `ParsedPage` → `.md` → disk → reconcile.
//!
//! Three entry points, all converging on `outl_md::write_atomic` +
//! `outl_md::reconcile_md`:
//!
//! - [`App::save`] — persist the in-memory `app.page` to the
//!   currently-opened path. The hot path: every Insert commit,
//!   structural op, paste lands here eventually.
//! - [`App::save_page_with`] — persist a *different* page than the
//!   one the user is looking at. Used by cross-page backlink edits.
//! - [`App::save_page`] — convenience wrapper over `save_page_with`
//!   that always rebuilds the index. Kept for callers that don't
//!   want to think about the optimistic-patch flag.
//!
//! After each save we patch the current page's entries in the
//! backlink index (the page just changed; `reindex_backlinks_for_slug`
//! re-reads its one `.md`) and resync `last_mtime` so the polling loop
//! doesn't mistake our own write for an external edit. A cross-page
//! save re-spawns the whole background build instead.

use crate::outline_ops::flat_count;
use crate::state::{App, ToastKind};
use outl_md::parse::{OutlineNode, ParsedPage};
use outl_md::reconcile::reconcile_md;
use outl_md::render::render;
use std::path::Path;

use super::file_mtime;

impl App {
    /// Render an arbitrary `ParsedPage` to `path` and reconcile.
    ///
    /// Used by the cross-page commit route (editing a backlink): the
    /// caller has a working copy of a *different* page than
    /// `current_path()`, and wants to persist it without disturbing
    /// `app.view` / `app.page`. Updates status on failure, rebuilds
    /// the workspace index, and keeps `last_mtime` honest when the
    /// path happens to coincide with the open view.
    #[allow(dead_code)]
    pub(crate) fn save_page(&mut self, path: &Path, page: &ParsedPage) {
        self.save_page_with(path, page, true);
    }

    /// Same as [`Self::save_page`] but lets the caller opt out of
    /// refreshing the workspace index for this page.
    ///
    /// **The `true` branch is incremental.** It calls
    /// [`outl_md::index::WorkspaceIndex::patch_page`] on `path` only,
    /// which is `O(blocks in this page)` — *not* a full workspace
    /// rescan. Pick `true` whenever the write changes anything the
    /// index tracks (block refs `((blk-XXXXXX))`, page title, icon,
    /// pinned flag); pick `false` only when you've already patched
    /// the index by hand and the call would just redo the same work.
    ///
    /// Whether or not the index is patched, `App.backlinks_cache` is
    /// invalidated unconditionally because backlinks live outside
    /// the index now.
    pub(crate) fn save_page_with(&mut self, path: &Path, page: &ParsedPage, rebuild_index: bool) {
        let md = render(page);
        if let Err(e) = outl_md::write_atomic(path, md.as_bytes()) {
            self.status = format!("save (source) failed: {e}");
            return;
        }
        if let Err(e) = reconcile_md(
            &mut self.workspace,
            &self.hlc,
            path,
            Some(&self.orphans_log),
        ) {
            self.status = format!("reconcile (source) failed: {e}");
            return;
        }
        self.status.clear();
        if rebuild_index {
            // Incremental: just re-index this one page. Full workspace
            // rescans are reserved for cold start and `Ctrl+L`.
            self.index.patch_page(path, page);
        }
        // Reconciling the source page may have touched blocks that
        // mention the current view's slug — rebuild the backlink index
        // off-thread (a cross-page save is rare, so the full rebuild is
        // fine; the incremental patch only covers the current page).
        self.spawn_backlink_index_rebuild();
        if path == self.current_path() {
            self.last_mtime = file_mtime(path);
        }
        // Post-commit hook (cross-page save): same as `save()`. Without it a
        // backlink edit committed locally but never woke peers, so the change
        // only propagated on the catch-up re-sync instead of in real time.
        // The payload slug is informational — the receiver pulls by vector
        // clock — so deriving it from the saved path keeps it honest.
        if let Some(transport) = &self.sync_transport {
            let slug = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            let hlc = self.hlc.next();
            transport.announce_local_ops(&slug, hlc);
        }
    }

    /// Mark the in-memory `page` as changed and repaint — **without**
    /// touching disk. The heavy `render → write → reconcile_md → fsync`
    /// runs later in [`Self::flush_pending_save`], drained by the event
    /// loop the instant it goes idle.
    ///
    /// This is the edit hot path: every commit boundary (`Esc`, `Enter`,
    /// `dd`, indent, paste, …) calls `save()`, and on a large workspace
    /// the reconcile+fsync is tens of milliseconds. Doing it inline made
    /// a burst of edits stutter — each `Esc` blocked the next keystroke.
    /// Coalescing it means the user always sees the result on the next
    /// frame and keeps typing; the persist happens in the gap when they
    /// pause (or is forced by [`crate::runtime::MAX_SAVE_DEFER`] / any
    /// read of persisted state).
    ///
    /// Callers that need the op log / workspace to reflect the edit
    /// *right now* (a `call:` re-run, cross-page navigation, quit) must
    /// call [`Self::flush_pending_save`] first — those paths already do.
    pub(crate) fn save(&mut self) {
        self.dirty_since.get_or_insert_with(std::time::Instant::now);
    }

    /// `true` when an edit is committed to the in-memory AST but not yet
    /// persisted to disk / the op log. The event loop polls this to
    /// drain the save on idle and to keep its `event::poll` timeout
    /// short while a save is outstanding.
    pub(crate) fn has_pending_save(&self) -> bool {
        self.dirty_since.is_some()
    }

    /// How long the pending edit has waited unpersisted, or `None` when
    /// nothing is pending. The event loop uses this to force a flush
    /// once the wait exceeds `MAX_SAVE_DEFER`, even mid-burst.
    pub(crate) fn pending_save_age(&self) -> Option<std::time::Duration> {
        self.dirty_since.map(|t| t.elapsed())
    }

    /// Persist the pending edit if there is one, then clear the dirty
    /// mark. A no-op when nothing changed since the last persist. This
    /// is the barrier every path that reads persisted state runs first
    /// (navigation, peer reload, quit, `call:` re-run) so no reader ever
    /// sees a stale `.md` / op log.
    pub(crate) fn flush_pending_save(&mut self) {
        if self.dirty_since.take().is_some() {
            self.persist();
        }
    }

    /// Render the in-memory `page` back to disk and reconcile.
    ///
    /// All writes go through [`outl_md::write_atomic`] so a crash
    /// between render and rename cannot leave a half-written `.md`.
    /// Not called directly by edit paths — they go through [`Self::save`]
    /// (mark dirty) and this is drained by [`Self::flush_pending_save`].
    pub(crate) fn persist(&mut self) {
        // The page on screen came from a failed read, not from disk.
        // Rendering this AST back would replace a file we could not
        // even read with an empty one, and the reconcile that follows
        // would trash every block on every device. Refuse, loudly.
        if self.load_failed {
            self.toast(
                ToastKind::Error,
                "not saving: this page could not be read from disk".to_string(),
            );
            return;
        }
        // Timing instrumentation: run with `RUST_LOG=outl_tui=debug` and
        // read `<workspace>/.outl/tui.log` to see where a slow commit
        // spends its time (reconcile vs backlinks vs auto-run).
        let t0 = std::time::Instant::now();
        let path = self.current_path();
        let md = render(&self.page);
        if let Err(e) = outl_md::write_atomic(&path, md.as_bytes()) {
            self.toast(ToastKind::Error, format!("save failed: {e}"));
            return;
        }
        let t_reconcile = std::time::Instant::now();
        let mut ops_applied = 0usize;
        match reconcile_md(
            &mut self.workspace,
            &self.hlc,
            &path,
            Some(&self.orphans_log),
        ) {
            Ok(report) => {
                ops_applied = report.ops_applied;
                self.status.clear();
            }
            Err(e) => self.toast(ToastKind::Error, format!("reconcile failed: {e}")),
        }
        let reconcile_ms = t_reconcile.elapsed().as_secs_f64() * 1000.0;
        self.flat_len = flat_count(&self.page.blocks);
        if self.flat_len == 0 {
            // Always keep at least one empty bullet so the cursor has a home.
            self.page.blocks.push(OutlineNode::default());
            self.flat_len = 1;
            let _ = outl_md::write_atomic(&path, render(&self.page).as_bytes());
        }
        self.selected = self.selected.min(self.flat_len.saturating_sub(1));
        // Incremental re-index: only this page changed, so just
        // refresh its entries (backlinks, title, icon). Cheap and
        // synchronous — no waiting for a worker to rescan the whole
        // workspace.
        let t_index = std::time::Instant::now();
        self.index.patch_page(&path, &self.page);
        // Only THIS page changed, so patch just its backlink entries from
        // the freshly-projected `.md` — O(one page). Rebuilding the index
        // from EVERY `.md` in the workspace per commit, inline on the
        // event loop, was the "Esc is slow in the TUI" bug.
        self.reindex_backlinks_for_slug(&self.current_slug());
        let index_ms = t_index.elapsed().as_secs_f64() * 1000.0;
        // Update mtime AFTER the write so the polling loop doesn't
        // mistake our own save for an external edit.
        self.last_mtime = file_mtime(&path);
        self.last_saved_at = Some(std::time::Instant::now());
        // Re-run query / auto-run blocks — workspace state changed,
        // so results may be different now.
        let t_autorun = std::time::Instant::now();
        self.run_auto_run_blocks();
        let autorun_ms = t_autorun.elapsed().as_secs_f64() * 1000.0;
        tracing::debug!(
            blocks = self.flat_len,
            ops_applied,
            total_ms = t0.elapsed().as_secs_f64() * 1000.0,
            reconcile_ms,
            index_ms,
            autorun_ms,
            "tui commit (save)"
        );
        // Post-commit hook: tell the transport new local ops landed.
        // No-op for FileSyncTransport (the file is already on disk and
        // a peer's poller will notice it); IrohSyncTransport gossips
        // the HLC so connected peers pull the new ops over QUIC.
        if let Some(transport) = &self.sync_transport {
            let workspace_id = self.current_slug();
            // The committed ops carry HLCs already minted by
            // `reconcile_md`; `next()` returns a fresh HLC that sorts
            // after all of them, which is what peers need to know the
            // high-water mark to pull up to.
            let hlc = self.hlc.next();
            transport.announce_local_ops(&workspace_id, hlc);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::state::App;
    use outl_core::{ActorId, Workspace};
    use tempfile::TempDir;

    fn fresh_app() -> (App, TempDir) {
        let dir = TempDir::new().unwrap();
        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
        let app = App::new(
            dir.path().to_path_buf(),
            ws,
            actor,
            crate::theme::default_theme(),
            false,
        )
        .unwrap();
        (app, dir)
    }

    fn seed_single_block(app: &mut App, text: &str) {
        app.page.blocks.clear();
        app.page.blocks.push(outl_md::parse::OutlineNode {
            text: text.to_string(),
            children: vec![],
            properties: vec![],
        });
        app.flat_len = 1;
        app.selected = 0;
    }

    // `save()` is the edit hot path — it must NOT touch disk, only mark
    // the page dirty. The heavy persist is deferred to `flush`.
    #[test]
    fn save_marks_dirty_without_touching_disk() {
        let (mut app, _dir) = fresh_app();
        seed_single_block(&mut app, "coalesced body");
        let path = app.current_path();

        app.save();
        assert!(app.has_pending_save(), "save() must set the dirty mark");
        let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            !on_disk.contains("coalesced body"),
            "save() must not write the .md — persist is deferred"
        );
    }

    // `flush_pending_save` drains the coalesced edit to disk and clears
    // the dirty mark; a second flush is a no-op.
    #[test]
    fn flush_persists_then_is_idempotent() {
        let (mut app, _dir) = fresh_app();
        seed_single_block(&mut app, "flush me to disk");
        let path = app.current_path();

        app.save();
        app.flush_pending_save();
        assert!(!app.has_pending_save(), "flush clears the dirty mark");
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(
            on_disk.contains("flush me to disk"),
            "flush must persist the .md, got: {on_disk:?}"
        );

        // Nothing pending → a second flush changes nothing.
        app.flush_pending_save();
        assert!(!app.has_pending_save());
    }

    // The load-bearing guarantee: navigation reparses the page from
    // disk, so it must flush a pending edit first. Without the flush in
    // `load_current_no_autorun`, the reparse would silently drop the
    // in-memory edit — data loss.
    #[test]
    fn navigation_flushes_before_reparsing_from_disk() {
        let (mut app, _dir) = fresh_app();
        seed_single_block(&mut app, "must survive nav");
        app.save();
        assert!(app.has_pending_save());

        // Reparses `current_path()` from disk; must persist first.
        app.load_current();

        assert!(!app.has_pending_save(), "nav drained the pending save");
        let on_disk = std::fs::read_to_string(app.current_path()).unwrap();
        assert!(
            on_disk.contains("must survive nav"),
            "the edit must reach disk before the reparse, got: {on_disk:?}"
        );
        assert!(
            app.page.blocks.iter().any(|b| b.text == "must survive nav"),
            "the reparsed AST still carries the edit"
        );
    }
}
