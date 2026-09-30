//! Sidebar navigation: open/close, focus moves, item activation.
//!
//! The renderer in `view::sidebar` is read-only — it derives lists
//! from `app.index` and `app.recent_paths`. This module owns the
//! *behavior*: which section has focus, where the cursor sits inside
//! it, and what happens on `Enter`.
//!
//! Public surface called from `input.rs`:
//! - [`App::sidebar_open_focused`]  — `\` while closed
//! - [`App::sidebar_close`]         — `\` while open (any focus)
//! - [`App::sidebar_blur`]          — `Esc` to return focus to the outline
//! - [`App::sidebar_cycle_section`] — `Tab` / `Shift-Tab`
//! - [`App::sidebar_move`]          — `j`/`k` inside the focused section
//! - [`App::sidebar_activate`]      — `Enter` on the focused item
//!
//! Calendar focus is intentionally **stubbed for now**: the section
//! takes focus and the user can `Tab` through it, but `Enter` does
//! nothing. Day-by-day navigation needs its own cursor state (which
//! date is highlighted) — a follow-up patch.

use crate::state::{App, PendingSidebarDelete, SidebarSection, View};
use anyhow::Result;
use chrono::NaiveDate;
use std::path::PathBuf;

impl App {
    /// `\` while the sidebar is closed: open it and drop focus onto
    /// the first non-empty section.
    ///
    /// Pinned wins by default (the user explicitly curated those).
    /// If Pinned is empty, fall back to Recent so `Enter` still does
    /// something useful. If both are empty, focus Calendar — the
    /// user at least gets a visual cue that the sidebar exists.
    pub(crate) fn sidebar_open_focused(&mut self) {
        self.show_sidebar = true;
        let initial = if self.pinned_slugs_sorted().is_empty() {
            if self.recent_paths.is_empty() {
                SidebarSection::Calendar
            } else {
                SidebarSection::Recent
            }
        } else {
            SidebarSection::Pinned
        };
        self.sidebar_focus = Some(initial);
        self.sidebar_cursor = 0;
    }

    /// `\` while the sidebar is open: hide it and drop focus.
    pub(crate) fn sidebar_close(&mut self) {
        self.show_sidebar = false;
        self.sidebar_focus = None;
        self.sidebar_cursor = 0;
    }

    /// `Esc` inside a focused sidebar: keep the sidebar visible but
    /// hand the keyboard back to the outline. The cursor remembers
    /// its position so Tab can re-enter the same item.
    pub(crate) fn sidebar_blur(&mut self) {
        self.sidebar_focus = None;
    }

    /// `Tab` / `Shift-Tab` inside the sidebar: rotate through the
    /// three sections. Resets the cursor to 0 so the new section
    /// starts at its first item — coming back to a previous section
    /// with a stale cursor was confusing in early prototypes.
    pub(crate) fn sidebar_cycle_section(&mut self, forward: bool) {
        let Some(cur) = self.sidebar_focus else {
            return;
        };
        let order = [
            SidebarSection::Pinned,
            SidebarSection::Recent,
            SidebarSection::Calendar,
        ];
        let idx = order.iter().position(|s| *s == cur).unwrap_or(0);
        let next = if forward {
            (idx + 1) % order.len()
        } else {
            (idx + order.len() - 1) % order.len()
        };
        self.sidebar_focus = Some(order[next]);
        self.sidebar_cursor = 0;
    }

    /// `j` / `k` inside a focused section: advance or retreat by
    /// `delta`. Clamps at both ends (no wrap-around) so the user
    /// always knows when they're at a boundary.
    pub(crate) fn sidebar_move(&mut self, delta: i32) {
        let Some(section) = self.sidebar_focus else {
            return;
        };
        let count = self.sidebar_item_count(section);
        if count == 0 {
            self.sidebar_cursor = 0;
            return;
        }
        let max = count - 1;
        let cur = self.sidebar_cursor as i32;
        let new = (cur + delta).max(0).min(max as i32) as usize;
        self.sidebar_cursor = new;
    }

    /// `Enter` on the focused sidebar item: open the page (or
    /// journal) it points at. No-op for Calendar until that section
    /// gains its own day cursor.
    pub(crate) fn sidebar_activate(&mut self) -> Result<()> {
        let Some(section) = self.sidebar_focus else {
            return Ok(());
        };
        match section {
            SidebarSection::Pinned => {
                let pinned = self.pinned_slugs_sorted();
                if let Some(slug) = pinned.get(self.sidebar_cursor) {
                    self.open_slug(slug)?;
                }
            }
            SidebarSection::Recent => {
                if let Some(path) = self.recent_paths.get(self.sidebar_cursor).cloned() {
                    self.open_path(path)?;
                }
            }
            SidebarSection::Calendar => {
                // No-op until calendar grows its own date cursor.
            }
        }
        Ok(())
    }

    /// Pinned slugs in the same alphabetical order the renderer
    /// uses. Mirroring the sort here means index N in the action
    /// layer matches row N on screen — no drift between what the
    /// user sees and what `Enter` opens.
    pub(crate) fn pinned_slugs_sorted(&self) -> Vec<String> {
        let mut v: Vec<(String, String)> = self
            .index
            .pages()
            .filter(|p| p.pinned)
            .map(|p| (p.slug.clone(), p.title.clone()))
            .collect();
        v.sort_by_key(|(_, t)| t.to_lowercase());
        v.into_iter().map(|(s, _)| s).collect()
    }

    /// `d` on a focused Pinned / Recent sidebar row: arm a one-shot
    /// "delete this page?" confirmation. The status line shows the
    /// prompt; the next `y` / `Y` confirms via [`Self::sidebar_confirm_delete`],
    /// any other keystroke cancels (handled in `input::normal`).
    ///
    /// Calendar rows are a no-op — deleting a journal by accident from
    /// the mini-calendar would be a hostile surprise, and there's no
    /// trash UI to recover from today.
    pub(crate) fn sidebar_delete_current(&mut self) {
        let Some(section) = self.sidebar_focus else {
            return;
        };
        let slug = match section {
            SidebarSection::Pinned => self.pinned_slugs_sorted().get(self.sidebar_cursor).cloned(),
            SidebarSection::Recent => self
                .recent_paths
                .get(self.sidebar_cursor)
                .and_then(|p| p.file_stem().and_then(|s| s.to_str()).map(String::from)),
            SidebarSection::Calendar => None,
        };
        if let Some(slug) = slug {
            // Journal guard: Pinned / Recent can surface journals
            // (a pinned daily note, a recently-opened journal).
            // Deleting one from the sidebar contradicts the calendar
            // exclusion and the chord path's journal refusal, so
            // refuse here too. No toast — a silent no-op matches the
            // Calendar section's posture.
            let is_journal = self
                .index
                .by_slug(&slug)
                .map(|e| e.is_journal)
                .unwrap_or_else(|| NaiveDate::parse_from_str(&slug, "%Y-%m-%d").is_ok());
            if is_journal {
                return;
            }
            self.arm_sidebar_delete(slug);
        }
    }

    /// `g d` chord (`Action::DeletePage`) entry point shared by the
    /// chord accumulator and the catalog dispatcher. When the sidebar
    /// owns focus, delegates to [`Self::sidebar_delete_current`] so the
    /// highlighted row is the target. Otherwise arms the confirmation
    /// against the **current page** — the page the user is viewing.
    ///
    /// Refuses to delete a journal (date-shaped slug) from the
    /// outline branch: deleting today's note by accident from `gd`
    /// would be a hostile surprise. Matches the sidebar's calendar
    /// exclusion.
    ///
    /// The guard used to rest on a second argument — "and there's no
    /// trash UI to recover from today" — which stopped being true when
    /// `outl trash restore` shipped (issue #287). It is still the wrong
    /// recovery to lean on here: restoring a **page** is exactly what
    /// that command refuses, so a journal deleted by accident is still
    /// only reachable through the op log. The guard stays; only its
    /// reasoning got narrower.
    /// `Capability::Trash` records that no client has a trash surface.
    pub(crate) fn delete_page_from_chord(&mut self) {
        if self.sidebar_focus.is_some() {
            self.sidebar_delete_current();
            return;
        }
        let slug = self.current_slug();
        if slug.is_empty() {
            return;
        }
        // Journal guard: a date-shaped slug means we're on a daily
        // note. Same exclusion the sidebar applies to Calendar rows.
        if NaiveDate::parse_from_str(&slug, "%Y-%m-%d").is_ok() {
            self.toast(
                crate::state::ToastKind::Warning,
                "can't delete a journal page",
            );
            return;
        }
        self.arm_sidebar_delete(slug);
    }

    /// Resolve `slug` to its display title (workspace index, falling
    /// back to the slug itself), stash a [`PendingSidebarDelete`], and
    /// surface the `y/n` prompt in the status line. Shared arm path
    /// for the sidebar `d` key and the `gd` chord so the two entry
    /// points never drift on title resolution or prompt wording.
    fn arm_sidebar_delete(&mut self, slug: String) {
        let title = self
            .index
            .by_slug(&slug)
            .map(|e| e.title.clone())
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| slug.clone());
        self.status = format!("delete page '{title}'? y/n");
        self.pending_sidebar_delete = Some(PendingSidebarDelete { slug, title });
    }

    /// Resolve an armed `pending_sidebar_delete`: run the CRDT delete
    /// via `outl_actions::page::delete`, drop the on-disk projection,
    /// rebuild the index, navigate away if we just deleted the current
    /// view, and announce the op to peers. Returns `Err` only on a
    /// workspace / I/O failure the caller can't ignore.
    pub(crate) fn sidebar_confirm_delete(&mut self) -> Result<()> {
        let pending = self.pending_sidebar_delete.take();
        let Some(pending) = pending else {
            return Ok(());
        };
        let is_current = self.current_slug() == pending.slug;

        let meta = match outl_actions::delete_page(&mut self.workspace, &self.hlc, &pending.slug) {
            Ok(meta) => meta,
            Err(outl_actions::ActionError::PageNotFound(_)) => {
                // A peer (or a previous delete in this session) beat
                // us to it. Re-sync the index and tell the user.
                self.spawn_index_rebuild();
                self.toast(crate::state::ToastKind::Warning, "page already gone");
                return Ok(());
            }
            Err(e) => {
                self.toast(
                    crate::state::ToastKind::Error,
                    format!("delete failed: {e}"),
                );
                return Ok(());
            }
        };

        if let Err(e) = outl_actions::remove_page_projection(&self.workspace_root, &meta) {
            tracing::warn!(
                target: "outl::tui::sidebar",
                "could not remove page projection for {}: {e}",
                meta.slug
            );
        }

        // The page list just changed — re-scan so the sidebar drops
        // the deleted row on the next render. Spawning (not sync)
        // keeps the event loop responsive on big workspaces.
        self.spawn_index_rebuild();
        // Deleting a page drops it from every other page's backlinks —
        // rebuild the index off-thread, same as the workspace index.
        self.spawn_backlink_index_rebuild();

        if is_current {
            // Never leave the user staring at a deleted page — jump
            // to today's journal, same as boot.
            self.go_today()?;
        }

        // Announce to peers so the delete propagates over iroh
        // immediately instead of waiting for the catch-up re-sync.
        if let Some(transport) = &self.sync_transport {
            let hlc = self.hlc.next();
            transport.announce_local_ops(&meta.slug, hlc);
        }

        self.toast(
            crate::state::ToastKind::Info,
            format!("deleted: {}", pending.title),
        );
        self.status.clear();
        Ok(())
    }

    fn sidebar_item_count(&self, section: SidebarSection) -> usize {
        match section {
            SidebarSection::Pinned => self.pinned_slugs_sorted().len(),
            SidebarSection::Recent => self.recent_paths.len().min(20),
            SidebarSection::Calendar => 0, // navigated as a single block for now
        }
    }

    /// Open a page or journal by slug. Journals live under
    /// `journals/`, pages under `pages/`; the index knows the
    /// difference via `PageEntry.is_journal`.
    fn open_slug(&mut self, slug: &str) -> Result<()> {
        let Some(entry) = self.index.by_slug(slug).cloned() else {
            return Ok(());
        };
        if entry.is_journal {
            if let Ok(date) = NaiveDate::parse_from_str(&entry.slug, "%Y-%m-%d") {
                self.view = View::Journal(date);
                self.selected = 0;
                self.cursor_col = 0;
                self.ensure_view_file_exists()?;
                self.load_current();
                return Ok(());
            }
        }
        self.open_path(entry.path.clone())
    }

    /// Switch the view to an arbitrary `.md` path. Treats the file
    /// as a Page (not a Journal) — recent paths come from anywhere
    /// the user touched, but the journal branch is taken via the
    /// `open_slug` route above so we don't mistake a page in
    /// `journals/` for a date.
    pub(crate) fn open_path(&mut self, path: PathBuf) -> Result<()> {
        // Detect journal-shaped filenames (`YYYY-MM-DD.md`) and route
        // through the journal view so navigation (`[`, `]`, `t`) stays
        // consistent with how the user opened it.
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            if let Ok(date) = NaiveDate::parse_from_str(stem, "%Y-%m-%d") {
                self.view = View::Journal(date);
                self.selected = 0;
                self.cursor_col = 0;
                self.ensure_view_file_exists()?;
                self.load_current();
                return Ok(());
            }
        }
        self.view = View::Page(path);
        self.selected = 0;
        self.cursor_col = 0;
        self.load_current();
        Ok(())
    }
}
