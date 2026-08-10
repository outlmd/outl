//! Navigation: between pages, between journals, between blocks, and
//! inside a block's text. Also `[[ref]]` / `#tag` / date-link
//! resolution.
//!
//! Nothing in this file persists — `nav` only mutates `self.view`,
//! `self.selected`, and `self.cursor_col`. The lifecycle module is
//! the one that touches disk.

use crate::outline_ops::{node_at_path, path_for_index};
use crate::state::{App, Focus, Mode, View};
use anyhow::Result;
use chrono::Duration;
use outl_actions::{clock, flatten_subtree_paths};
use outl_md::inline::{ref_at_cursor, RefTarget};
use outl_md::reconcile::reconcile_md;
use std::fs;
use std::path::PathBuf;

impl App {
    pub(crate) fn current_path(&self) -> PathBuf {
        match &self.view {
            View::Journal(date) => self
                .workspace_root
                .join("journals")
                .join(format!("{}.md", date.format("%Y-%m-%d"))),
            View::Page(p) => p.clone(),
        }
    }

    #[allow(dead_code)] // header now uses chrome::breadcrumb; kept for future reuse
    pub(crate) fn current_title(&self) -> String {
        let mode_tag = match self.mode {
            Mode::Normal => "NORMAL",
            Mode::Insert { .. } => "INSERT",
            Mode::Visual { .. } => "VISUAL",
        };
        match &self.view {
            View::Journal(date) => {
                format!("Journal · {} · [{}]", date.format("%A, %Y-%m-%d"), mode_tag)
            }
            View::Page(p) => {
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
                // Pull title + icon from the workspace index. Falls
                // back to the slug when the index doesn't know about
                // this page yet (just-created file, before the next
                // rebuild). Title is preferred over slug because it's
                // what the user wrote — `Page · CTO` reads better than
                // `Page · cto`.
                let entry = self.index.by_slug(stem);
                let display_name = entry
                    .map(|e| e.title.clone())
                    .unwrap_or_else(|| stem.to_string());
                let icon_prefix = entry
                    .and_then(|e| e.icon.as_deref())
                    .map(|i| format!("{i} "))
                    .unwrap_or_default();
                format!("Page · {icon_prefix}{display_name} · [{mode_tag}]")
            }
        }
    }

    /// Slug of the currently-opened view, used to look up backlinks.
    pub(crate) fn current_slug(&self) -> String {
        self.current_path()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string()
    }

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
    /// thread, mirroring [`Self::spawn_index_rebuild`].
    ///
    /// The build reads every page's `.md` off disk
    /// (`build_backlink_index_from_disk`) — `Send`, no `Workspace`, no
    /// lock. Doing it inline froze the open on a large vault (reading
    /// 2800+ `.md` on the event-loop thread); a worker keeps the journal
    /// paintable and fills the panel in a beat later. Replaces any
    /// in-flight build (the previous thread's result is dropped on
    /// arrival). The **old** index stays live until the new one lands,
    /// so the panel doesn't blank during a rebuild.
    pub(crate) fn spawn_backlink_index_rebuild(&mut self) {
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
    /// old `invalidate` forced the next render to rebuild the index from
    /// EVERY `.md` in the workspace, inline on the event loop — that full
    /// rescan on every keystroke-commit was the "Esc is slow in the TUI"
    /// bug. A no-op when the index isn't built yet (`None`): the first
    /// backlinks read builds it fresh. The page's `.md`/`.outl` must be
    /// projected first (the caller writes them before calling).
    pub(crate) fn reindex_backlinks_for_slug(&self, slug: &str) {
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
        self.status = match outl_config::save(&cfg) {
            Ok(()) => format!("backlinks: {label}"),
            Err(e) => format!("backlinks: {label} (not saved: {e})"),
        };
    }

    pub(crate) fn go_today(&mut self) -> Result<()> {
        self.view = View::Journal(clock::today());
        self.selected = 0;
        self.cursor_col = 0;
        self.ensure_view_file_exists()?;
        self.load_current();
        Ok(())
    }

    pub(crate) fn shift_journal(&mut self, days: i64) -> Result<()> {
        let new_date = match self.view {
            View::Journal(d) => d + Duration::days(days),
            _ => clock::today() + Duration::days(days),
        };
        self.view = View::Journal(new_date);
        self.selected = 0;
        self.cursor_col = 0;
        self.ensure_view_file_exists()?;
        self.load_current();
        Ok(())
    }

    pub(crate) fn move_selection(&mut self, delta: i32) {
        if self.flat_len == 0 && matches!(self.focus, Focus::Outline) {
            self.selected = 0;
            self.cursor_col = 0;
            return;
        }
        if delta > 0 {
            for _ in 0..delta {
                if !self.step_forward() {
                    break;
                }
            }
        } else {
            for _ in 0..(-delta) {
                if !self.step_backward() {
                    break;
                }
            }
        }
    }

    /// Advance the cursor by one position in the virtual flat list
    /// `outline blocks ++ backlink section blocks`. Returns `true` if
    /// the cursor moved, `false` when already at the bottom.
    ///
    /// Crosses the boundary between outline and backlinks transparently
    /// when the inline section is shown and non-empty.
    fn step_forward(&mut self) -> bool {
        match self.focus.clone() {
            Focus::Outline => {
                // When zoomed into a block, navigation is confined to
                // that block's subtree window `[start, end)`; otherwise
                // the window is the whole page.
                let (_, end) = self.zoom_root_window();
                // Walk forward until we hit a visible block (not
                // hidden under a collapsed ancestor) or fall off the
                // end of the (zoom-confined) outline.
                let mut next = self.selected + 1;
                while next < end && self.hidden_by_collapse.get(next).copied().unwrap_or(false) {
                    next += 1;
                }
                if next < end {
                    self.selected = next;
                    self.cursor_col = 0;
                    return true;
                }
                // Bottom of outline → try entering the backlinks zone.
                // Only when the whole page is shown: a zoomed subtree
                // ends before `flat_len`, and its backlinks aren't part
                // of the focused view, so `j` stops at the subtree edge.
                if self.zoom_stack.is_empty() && self.backlinks_navigable() {
                    self.focus = Focus::Backlink {
                        idx: 0,
                        sub_path: Vec::new(),
                    };
                    self.cursor_col = 0;
                    return true;
                }
                false
            }
            Focus::Backlink { idx, sub_path } => {
                // Borrow the backlinks slice directly instead of
                // cloning the whole `Vec<Backlink>` (each entry
                // carries an `OutlineNode` subtree — non-trivial to
                // clone per keystroke).
                let slug = self.current_slug();
                let new_focus = {
                    let backlinks = self.backlinks_for_slug(&slug);
                    let Some(bl) = backlinks.get(idx) else {
                        return false;
                    };
                    let paths = flatten_subtree_paths(&bl.source_block);
                    let cur_pos = paths.iter().position(|p| p == &sub_path).unwrap_or(0);
                    if cur_pos + 1 < paths.len() {
                        Focus::Backlink {
                            idx,
                            sub_path: paths[cur_pos + 1].clone(),
                        }
                    } else if idx + 1 < backlinks.len() {
                        Focus::Backlink {
                            idx: idx + 1,
                            sub_path: Vec::new(),
                        }
                    } else {
                        return false;
                    }
                };
                self.focus = new_focus;
                self.cursor_col = 0;
                true
            }
        }
    }

    /// Mirror of [`step_forward`], moving one position upward.
    fn step_backward(&mut self) -> bool {
        match self.focus.clone() {
            Focus::Outline => {
                // The zoom root is the top of the confined window — `k`
                // must not walk above it. Not zoomed → floor is 0.
                let (start, _) = self.zoom_root_window();
                // Walk backward over hidden subtree entries the same
                // way `step_forward` skips them going down.
                if self.selected <= start {
                    return false;
                }
                let mut prev = self.selected - 1;
                while prev > start && self.hidden_by_collapse.get(prev).copied().unwrap_or(false) {
                    prev -= 1;
                }
                if self.hidden_by_collapse.get(prev).copied().unwrap_or(false) {
                    // Reached the top still inside a collapsed
                    // subtree — no visible previous block.
                    return false;
                }
                self.selected = prev;
                self.cursor_col = 0;
                true
            }
            Focus::Backlink { idx, sub_path } => {
                let slug = self.current_slug();
                // Resolve the new focus value while only borrowing the
                // backlinks slice — no `to_vec` clone per keystroke.
                let new_focus_opt = {
                    let backlinks = self.backlinks_for_slug(&slug);
                    let Some(bl) = backlinks.get(idx) else {
                        return false;
                    };
                    let paths = flatten_subtree_paths(&bl.source_block);
                    let cur_pos = paths.iter().position(|p| p == &sub_path).unwrap_or(0);
                    if cur_pos > 0 {
                        Some(Focus::Backlink {
                            idx,
                            sub_path: paths[cur_pos - 1].clone(),
                        })
                    } else if idx > 0 {
                        // Jump to the last block of the previous backlink.
                        let prev_paths = flatten_subtree_paths(&backlinks[idx - 1].source_block);
                        let last = prev_paths.last().cloned().unwrap_or_default();
                        Some(Focus::Backlink {
                            idx: idx - 1,
                            sub_path: last,
                        })
                    } else {
                        // Topping out → fall back into the outline.
                        None
                    }
                };
                match new_focus_opt {
                    Some(f) => self.focus = f,
                    None => {
                        self.focus = Focus::Outline;
                        self.selected = self.flat_len.saturating_sub(1);
                    }
                }
                self.cursor_col = 0;
                true
            }
        }
    }

    /// `true` when the inline backlinks section is rendered *and* has
    /// at least one block the cursor can land on. Drives the cross-zone
    /// transition in `step_forward`/`step_backward`.
    fn backlinks_navigable(&self) -> bool {
        self.show_backlinks && self.backlinks_count_for_current() > 0
    }

    /// Current selected block's text (or empty if no selection).
    /// Honours `app.focus` so backlink blocks return their own text.
    pub(crate) fn current_block_text(&self) -> String {
        match &self.focus {
            Focus::Outline => {
                let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
                    return String::new();
                };
                node_at_path(&self.page.blocks, &path)
                    .map(|n| n.text.clone())
                    .unwrap_or_default()
            }
            Focus::Backlink { idx, sub_path } => {
                let backlinks = self.backlinks_for_current();
                let Some(bl) = backlinks.get(*idx) else {
                    return String::new();
                };
                let mut node = &bl.source_block;
                for &i in sub_path {
                    let Some(child) = node.children.get(i) else {
                        return String::new();
                    };
                    node = child;
                }
                node.text.clone()
            }
        }
    }

    pub(crate) fn current_block_char_count(&self) -> usize {
        self.current_block_text().chars().count()
    }

    pub(crate) fn move_cursor_col(&mut self, delta: i32) {
        let max = self.current_block_char_count() as i32;
        let next = (self.cursor_col as i32 + delta).clamp(0, max);
        self.cursor_col = next as usize;
    }

    pub(crate) fn cursor_to_home(&mut self) {
        self.cursor_col = 0;
    }

    pub(crate) fn cursor_to_end(&mut self) {
        self.cursor_col = self.current_block_char_count();
    }

    pub(crate) fn cursor_word_left(&mut self) {
        let text = self.current_block_text();
        let chars: Vec<char> = text.chars().collect();
        let mut i = self.cursor_col;
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        self.cursor_col = i;
    }

    pub(crate) fn cursor_word_right(&mut self) {
        let text = self.current_block_text();
        let chars: Vec<char> = text.chars().collect();
        let len = chars.len();
        let mut i = self.cursor_col;
        while i < len && !chars[i].is_whitespace() {
            i += 1;
        }
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }
        self.cursor_col = i;
    }

    /// `zz` — center the viewport vertically on the selected block.
    /// Adjusts `scroll_y` so the selection lands at the midpoint of
    /// `viewport_height`. Clamps at 0 so we don't scroll above the
    /// first block.
    pub(crate) fn center_viewport_on_selection(&mut self) {
        let vp = self.viewport_height.max(1) as i32;
        let target = (self.selected as i32) - vp / 2;
        self.scroll_y = target.max(0) as u16;
    }

    /// `*` / `#` — extract the word under the cursor and feed it to
    /// the workspace search. Direction is `forward = true` for `*`,
    /// `false` for `#`. No-op when the cursor isn't sitting on a word.
    pub(crate) fn search_word_under_cursor(&mut self, forward: bool) -> Result<()> {
        let text = self.current_block_text();
        let Some(word) = word_under_cursor(&text, self.cursor_col) else {
            self.status = "no word under cursor".into();
            return Ok(());
        };
        self.run_inline_search(&word, forward)
    }

    /// Build a search the same way the `/` overlay does, jump straight
    /// to the first hit, persist the rest into `last_search` so
    /// `n`/`N` can walk through them. Used by `*` / `#`.
    fn run_inline_search(&mut self, query: &str, forward: bool) -> Result<()> {
        // Reuse the overlay's machinery: open, set query, refresh,
        // accept. Cheaper than reimplementing the workspace walk.
        self.open_search();
        if let Some(crate::state::Overlay::Search(ref mut s)) = self.overlay {
            s.query = query.to_string();
        }
        self.refresh_search();
        self.accept_search()?;
        if forward {
            // `accept_search` already lands on hit 0; nothing else to do.
        } else {
            // `accept_search` set `last_search.cursor = 0`. `search_prev`
            // wraps 0 → len-1, which is exactly the last hit `#` wants.
            self.search_prev()?;
        }
        Ok(())
    }
}

/// Extract the word under `cursor` from `text`. A "word" is a
/// contiguous run of non-whitespace chars. Returns `None` on
/// whitespace / empty text.
fn word_under_cursor(text: &str, cursor: usize) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let pos = cursor.min(chars.len().saturating_sub(1));
    if chars[pos].is_whitespace() {
        return None;
    }
    let mut start = pos;
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }
    let mut end = pos;
    while end < chars.len() && !chars[end].is_whitespace() {
        end += 1;
    }
    let word: String = chars[start..end].iter().collect();
    if word.is_empty() {
        None
    } else {
        Some(word)
    }
}

impl App {
    /// If the cursor is sitting on a `[[ref]]`, `#tag`, or `[[YYYY-MM-DD]]`,
    /// open the corresponding page or journal. Returns `true` when an
    /// open happened so the caller can suppress the fallback (entering
    /// Insert mode on Enter).
    pub(crate) fn try_open_under_cursor(&mut self) -> Result<bool> {
        let text = self.current_block_text();
        let Some(target) = ref_at_cursor(&text, self.cursor_col) else {
            return Ok(false);
        };
        match target {
            RefTarget::Journal(date) => {
                self.view = View::Journal(date);
                self.selected = 0;
                self.cursor_col = 0;
                self.ensure_view_file_exists()?;
                self.load_current();
            }
            RefTarget::Page(name) | RefTarget::Tag(name) => {
                self.open_page_by_name(&name)?;
            }
            RefTarget::Block(handle) => {
                self.open_block_ref(&handle)?;
            }
        }
        Ok(true)
    }

    /// Open the source page of a `((blk-XXXXXX))` reference and put
    /// the selection on the referenced block.
    ///
    /// Resolution path:
    ///   1. Look the handle up in `WorkspaceIndex::resolve_block_ref`.
    ///      Orphan handles (no resolution) leave a status message and
    ///      otherwise no-op — the user keeps their current view.
    ///   2. Switch `view` to the source page (journal or regular page,
    ///      detected by the `journals/` ancestor segment in the path).
    ///   3. Load the page and translate `source_block_path` (a DFS
    ///      path) into a flat block index via
    ///      [`crate::outline_ops::index_for_path`]. Falls back to the
    ///      top of the page if the path no longer resolves (block
    ///      moved/deleted since the index was built).
    pub(crate) fn open_block_ref(&mut self, handle: &str) -> Result<()> {
        let Some(entry) = self.index.resolve_block_ref(handle) else {
            self.status = format!("ref (({handle})) does not resolve");
            return Ok(());
        };
        let source_path = entry.source_path.clone();
        let source_block_path = entry.source_block_path.clone();

        // Detect journal vs page from the path layout. Workspace
        // layout pins journals under `journals/` and pages under
        // `pages/`; everything else falls back to a page view.
        let is_journal = source_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            == Some("journals");
        if is_journal {
            if let Some(stem) = source_path.file_stem().and_then(|s| s.to_str()) {
                if let Ok(date) = chrono::NaiveDate::parse_from_str(stem, "%Y-%m-%d") {
                    self.view = View::Journal(date);
                    self.selected = 0;
                    self.cursor_col = 0;
                    self.load_current();
                    self.selected =
                        crate::outline_ops::index_for_path(&self.page.blocks, &source_block_path)
                            .unwrap_or(0);
                    return Ok(());
                }
            }
        }
        self.view = View::Page(source_path);
        self.selected = 0;
        self.cursor_col = 0;
        self.load_current();
        self.selected =
            crate::outline_ops::index_for_path(&self.page.blocks, &source_block_path).unwrap_or(0);
        Ok(())
    }

    /// Open (or create) the page corresponding to a user-visible name.
    /// Files live under `pages/{slug}.md`; the original `name` is
    /// preserved in the page's `title::` property.
    pub(crate) fn open_page_by_name(&mut self, name: &str) -> Result<()> {
        let slug = outl_md::slug::slugify(name);
        self.open_page_slug(&slug, name)
    }

    /// Open (or create) a page whose on-disk slug is already known.
    /// On-disk slugs are not always slugify-idempotent (MCP/CLI `page
    /// create` writes the caller's slug verbatim, so `~`, `%`, or
    /// uppercase can appear in the filename); re-slugifying one here
    /// would resolve to a different path and silently create an empty
    /// duplicate page.
    pub(crate) fn open_page_by_slug(&mut self, slug: &str) -> Result<()> {
        self.open_page_slug(slug, slug)
    }

    fn open_page_slug(&mut self, slug: &str, name: &str) -> Result<()> {
        let path = self.workspace_root.join("pages").join(format!("{slug}.md"));
        let created_new = !path.exists();
        if created_new {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            // Seed with title:: <name> + a single empty bullet so the
            // editor has a cursor home.
            let seed = format!("title:: {name}\n\n- \n");
            outl_md::write_atomic(&path, seed.as_bytes())?;
            // Reconcile to establish stable IDs.
            let _ = reconcile_md(
                &mut self.workspace,
                &self.hlc,
                &path,
                Some(&self.orphans_log),
            );
        }
        self.view = View::Page(path);
        self.selected = 0;
        self.cursor_col = 0;
        self.load_current();
        self.refresh_page_list();
        if created_new {
            self.status = format!("created page \"{name}\"");
        }
        Ok(())
    }
}

#[cfg(test)]
mod word_tests {
    use super::word_under_cursor;

    #[test]
    fn returns_word_under_cursor() {
        assert_eq!(
            word_under_cursor("hello world", 2),
            Some("hello".to_string())
        );
        assert_eq!(
            word_under_cursor("hello world", 6),
            Some("world".to_string())
        );
    }

    #[test]
    fn returns_none_on_whitespace_or_empty() {
        assert_eq!(word_under_cursor("hello world", 5), None);
        assert_eq!(word_under_cursor("", 0), None);
        assert_eq!(word_under_cursor("   ", 1), None);
    }

    #[test]
    fn clamps_past_end() {
        assert_eq!(
            word_under_cursor("hello", 99),
            Some("hello".to_string()),
            "cursor past EOL still picks up the last word"
        );
    }
}

#[cfg(test)]
mod open_page_tests {
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

    // Regression for the quick-switcher "preview shows content, open is
    // empty" bug: an on-disk slug that isn't slugify-idempotent (`~`,
    // `%` — MCP-written) must open verbatim, not be re-slugified into a
    // fresh empty duplicate page.
    #[test]
    fn open_by_slug_uses_the_literal_stem() {
        let (mut app, dir) = fresh_app();
        let pages = dir.path().join("pages");
        std::fs::create_dir_all(&pages).unwrap();
        let slug = "ai-memory~abc~sessions%2Fdef.md";
        let real = pages.join(format!("{slug}.md"));
        std::fs::write(&real, "- real content\n").unwrap();

        app.open_page_by_slug(slug).unwrap();

        assert_eq!(app.current_path(), real);
        assert_eq!(app.page.blocks[0].text, "real content");
        let slugified = pages.join(format!("{}.md", outl_md::slug::slugify(slug)));
        assert!(
            !slugified.exists(),
            "opening by literal slug must not mint a slugified duplicate"
        );
    }

    // `open_page_by_name` keeps its semantics: a user-visible name is
    // slugified before hitting disk.
    #[test]
    fn open_by_name_still_slugifies() {
        let (mut app, dir) = fresh_app();
        app.open_page_by_name("My Fancy Page").unwrap();
        let expected = dir.path().join("pages").join("my-fancy-page.md");
        assert_eq!(app.current_path(), expected);
    }
}
