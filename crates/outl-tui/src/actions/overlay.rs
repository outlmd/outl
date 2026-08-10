//! Modal overlays: quick switcher, workspace search, command palette,
//! and the Insert-mode inline `[[`/`#` autocomplete popup.
//!
//! Each overlay has the same shape — `open_*` initialises it,
//! `refresh_*` updates the candidate list as the user types,
//! `accept_*` commits the selection.

use crate::state::{
    hit_count, App, CommandState, ErrorState, LastSearch, Overlay, QuickSwitchState, SearchHit,
    SearchState, SlashCommand, SlashOrigin, SlashState, SwitchCandidate, SwitchKind,
    TemplatePickerState, View,
};
use anyhow::Result;
use outl_md::parse::OutlineNode;
use std::path::{Path, PathBuf};

impl App {
    /// Build the universe of switchable items: every page in `pages/`,
    /// every existing journal in `journals/`, plus today's date even if
    /// the journal file doesn't exist yet.
    fn collect_switch_candidates(&self) -> Vec<SwitchCandidate> {
        let mut out = Vec::new();

        // Pages.
        for path in &self.page_list {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            // Page title comes from the `title::` property if set, else slug.
            // Prefer the index — it already parsed everything once at
            // load time and reading from disk on every keystroke would
            // make the switcher noticeably laggy on big workspaces.
            let (title, icon) = self
                .index
                .by_slug(&stem)
                .map(|p| (p.title.clone(), p.icon.clone()))
                .unwrap_or_else(|| (read_page_title(path).unwrap_or_else(|| stem.clone()), None));
            let label = match icon {
                Some(i) => format!("{i} {title}"),
                None => title,
            };
            out.push(SwitchCandidate {
                label,
                key: stem,
                kind: SwitchKind::Page,
                score: 0,
            });
        }

        // Journals.
        let journals_dir = self.workspace_root.join("journals");
        if journals_dir.is_dir() {
            let mut journals: Vec<PathBuf> = walkdir::WalkDir::new(&journals_dir)
                .max_depth(1)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|e| {
                    e.file_type().is_file()
                        && e.path().extension().is_some_and(|x| x == "md")
                        && !e.file_name().to_string_lossy().starts_with('.')
                })
                .map(|e| e.path().to_path_buf())
                .collect();
            journals.sort_by(|a, b| b.cmp(a)); // newest first
            for p in journals {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    out.push(SwitchCandidate {
                        label: stem.to_string(),
                        key: stem.to_string(),
                        kind: SwitchKind::Journal,
                        score: 0,
                    });
                }
            }
        }

        // Always include today even if no file exists yet — typing
        // "today" always lands somewhere.
        let today_str = outl_actions::clock::today().format("%Y-%m-%d").to_string();
        if !out
            .iter()
            .any(|c| c.kind == SwitchKind::Journal && c.key == today_str)
        {
            out.push(SwitchCandidate {
                label: format!("{today_str} (new)"),
                key: today_str,
                kind: SwitchKind::Journal,
                score: 0,
            });
        }

        out
    }

    pub(crate) fn open_quick_switch(&mut self) {
        let candidates = self.collect_switch_candidates();
        self.overlay = Some(Overlay::QuickSwitch(QuickSwitchState {
            query: String::new(),
            all_candidates: candidates.clone(),
            candidates,
            selected: 0,
            preview_cache: std::cell::RefCell::new(None),
        }));
    }

    pub(crate) fn refresh_quick_switch(&mut self) {
        if let Some(Overlay::QuickSwitch(ref mut qs)) = self.overlay {
            // Score every candidate by the current query; drop misses.
            // Always filter from the full, immutable `all_candidates`
            // set — never from the already-narrowed `candidates`, or
            // deleting query characters could never restore items that
            // an earlier, longer query had dropped.
            let mut filtered: Vec<SwitchCandidate> = qs
                .all_candidates
                .iter()
                .filter_map(|c| {
                    let primary = crate::fuzzy::fuzzy_score(&qs.query, &c.label);
                    let secondary = crate::fuzzy::fuzzy_score(&qs.query, &c.key);
                    let best = match (primary, secondary) {
                        (Some(a), Some(b)) => Some(a.max(b)),
                        (a, b) => a.or(b),
                    };
                    best.map(|s| SwitchCandidate {
                        score: s,
                        ..c.clone()
                    })
                })
                .collect();
            filtered.sort_by(|a, b| b.score.cmp(&a.score).then(a.label.cmp(&b.label)));
            qs.candidates = filtered;
            qs.selected = qs.selected.min(qs.candidates.len().saturating_sub(1));
        }
    }

    pub(crate) fn accept_quick_switch(&mut self) -> Result<()> {
        let pick = if let Some(Overlay::QuickSwitch(ref qs)) = self.overlay {
            qs.candidates.get(qs.selected).cloned()
        } else {
            None
        };
        self.overlay = None;
        let Some(c) = pick else {
            return Ok(());
        };
        match c.kind {
            // `key` is the literal on-disk file stem, not a user-typed
            // name — never re-slugify it (see `open_page_by_slug`).
            SwitchKind::Page => self.open_page_by_slug(&c.key)?,
            SwitchKind::Journal => {
                if let Ok(date) = chrono::NaiveDate::parse_from_str(&c.key, "%Y-%m-%d") {
                    self.view = View::Journal(date);
                    self.selected = 0;
                    self.cursor_col = 0;
                    self.ensure_view_file_exists()?;
                    self.load_current();
                }
            }
        }
        Ok(())
    }

    pub(crate) fn open_search(&mut self) {
        self.overlay = Some(Overlay::Search(SearchState {
            query: String::new(),
            hits: Vec::new(),
            selected: 0,
        }));
    }

    pub(crate) fn refresh_search(&mut self) {
        let Some(Overlay::Search(ref state)) = self.overlay else {
            return;
        };
        let query = state.query.clone();
        if query.trim().is_empty() {
            if let Some(Overlay::Search(ref mut s)) = self.overlay {
                s.hits.clear();
            }
            return;
        }
        // Scan all .md files in pages/ and journals/. For each block,
        // run fuzzy match; keep top 30 hits by score.
        let mut hits = Vec::new();
        for dir in ["pages", "journals"] {
            let base = self.workspace_root.join(dir);
            if !base.is_dir() {
                continue;
            }
            for entry in walkdir::WalkDir::new(&base).max_depth(1) {
                let Ok(entry) = entry else {
                    continue;
                };
                if !entry.file_type().is_file() {
                    continue;
                }
                if entry.path().extension().and_then(|x| x.to_str()) != Some("md") {
                    continue;
                }
                if entry.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(entry.path()) else {
                    continue;
                };
                let parsed = outl_md::parse::parse(&text);
                let page_label = parsed
                    .properties
                    .iter()
                    .find(|(k, _)| k == "title")
                    .map(|(_, v)| v.clone())
                    .or_else(|| {
                        entry
                            .path()
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .map(String::from)
                    })
                    .unwrap_or_default();
                let page_icon = parsed
                    .properties
                    .iter()
                    .find(|(k, _)| k == "icon")
                    .map(|(_, v)| v.trim().to_string())
                    .filter(|s| !s.is_empty());
                let mut block_idx = 0usize;
                collect_block_hits(
                    &parsed.blocks,
                    &mut block_idx,
                    entry.path(),
                    &page_label,
                    page_icon.as_deref(),
                    &query,
                    &mut hits,
                );
            }
        }
        hits.sort_by_key(|b| std::cmp::Reverse(b.score));
        hits.truncate(30);
        if let Some(Overlay::Search(ref mut s)) = self.overlay {
            s.hits = hits;
            s.selected = s.selected.min(s.hits.len().saturating_sub(1));
        }
    }

    pub(crate) fn accept_search(&mut self) -> Result<()> {
        // Snapshot the entire result list onto the App so `n`/`N`
        // can walk through them after the overlay closes.
        let (hits, selected) = if let Some(Overlay::Search(ref s)) = self.overlay {
            (s.hits.clone(), s.selected)
        } else {
            return Ok(());
        };
        self.overlay = None;
        self.last_search = Some(LastSearch {
            hits: hits.clone(),
            cursor: selected,
        });
        let Some(h) = hits.get(selected).cloned() else {
            return Ok(());
        };
        self.jump_to_search_hit(&h)
    }

    /// Move to the n-th hit in the persisted last-search list.
    fn jump_to_search_hit(&mut self, h: &SearchHit) -> Result<()> {
        if h.md_path.starts_with(self.workspace_root.join("journals")) {
            if let Some(stem) = h.md_path.file_stem().and_then(|s| s.to_str()) {
                if let Ok(date) = chrono::NaiveDate::parse_from_str(stem, "%Y-%m-%d") {
                    self.view = View::Journal(date);
                }
            }
        } else {
            self.view = View::Page(h.md_path.clone());
        }
        self.selected = h.block_index;
        self.cursor_col = 0;
        self.ensure_view_file_exists()?;
        self.load_current();
        Ok(())
    }

    /// Jump to the next hit in the last `/` search. No-op if there's
    /// no saved search or the cursor is at the end.
    pub(crate) fn search_next(&mut self) -> Result<()> {
        let Some(ref ls) = self.last_search else {
            self.status = "no previous search".into();
            return Ok(());
        };
        if ls.hits.is_empty() {
            return Ok(());
        }
        let next = (ls.cursor + 1) % ls.hits.len();
        let hit = ls.hits[next].clone();
        if let Some(ls_mut) = self.last_search.as_mut() {
            ls_mut.cursor = next;
        }
        self.status = format!("hit {}/{}", next + 1, hit_count(&self.last_search));
        self.jump_to_search_hit(&hit)
    }

    /// Jump to the previous hit. Wraps to the last on underflow.
    pub(crate) fn search_prev(&mut self) -> Result<()> {
        let Some(ref ls) = self.last_search else {
            self.status = "no previous search".into();
            return Ok(());
        };
        if ls.hits.is_empty() {
            return Ok(());
        }
        let prev = if ls.cursor == 0 {
            ls.hits.len() - 1
        } else {
            ls.cursor - 1
        };
        let hit = ls.hits[prev].clone();
        if let Some(ls_mut) = self.last_search.as_mut() {
            ls_mut.cursor = prev;
        }
        self.status = format!("hit {}/{}", prev + 1, hit_count(&self.last_search));
        self.jump_to_search_hit(&hit)
    }

    /// Pop a modal error/warning over everything. Use for failures
    /// the status line can't fit — compile errors, multi-line traps,
    /// "rustc not found" toolchain hints. Any key dismisses.
    pub(crate) fn show_error(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.overlay = Some(Overlay::Error(ErrorState {
            title: title.into(),
            body: body.into(),
        }));
    }

    /// Open the Notion-style slash command menu.
    ///
    /// Bound to `/` in Normal mode. Replaces the old `/` =
    /// workspace-search binding — search is now a *command* inside
    /// the menu (one extra keystroke, full discoverability and
    /// future plugin commands appear here automatically).
    pub(crate) fn open_slash(&mut self) {
        // Candidates stay in registry order; `visual_order` in the
        // renderer (and `slash_select_*` in the nav) applies
        // category-first grouping without duplicating sort logic here.
        let candidates = self.slash_candidates();
        // Start the highlight on the first item in visual (paint) order
        // so the initial selection matches what the user sees.
        let first_visual = crate::view::overlays::visual_order(&candidates)
            .into_iter()
            .next()
            .unwrap_or(0);
        self.overlay = Some(Overlay::Slash(SlashState {
            query: String::new(),
            candidates,
            selected: first_visual,
        }));
    }

    /// The full slash-menu universe: every shipped command from the
    /// registry plus every command a loaded plugin contributes (whose
    /// `slash-command` capability this client grants). Both live in one
    /// filterable list so a plugin command is discoverable next to
    /// `/search` with no second surface to keep in sync.
    fn slash_candidates(&self) -> Vec<SlashCommand> {
        let mut out: Vec<SlashCommand> = self
            .command_registry
            .all()
            .map(|c| SlashCommand {
                name: c.name().to_string(),
                description: c.description().to_string(),
                needs_args: c.needs_args(),
                origin: SlashOrigin::Builtin,
            })
            .collect();
        if let Some(host) = &self.plugin_host {
            let mut seen = std::collections::HashSet::new();
            for cmd in host.commands() {
                seen.insert((cmd.plugin_id.clone(), cmd.command_id.clone()));
                // `name` is what the user types against and what shows in
                // the list, so it must be the command **id** (`stats`), not
                // the human title — that mirrors the CLI (`/stats`) and the
                // built-ins (`iso-date-today`), and a `/stats` query then
                // fuzzy-matches the id directly instead of ranking far below
                // the date built-ins (a title like "Workspace statistics"
                // scores ~42 on "sta" and falls off-screen; the id scores
                // ~150 and lands on top). The title moves to the description.
                out.push(SlashCommand {
                    name: cmd.command_id.clone(),
                    description: format!("{} · plugin", cmd.title),
                    needs_args: false,
                    origin: SlashOrigin::Plugin {
                        plugin_id: cmd.plugin_id,
                        command_id: cmd.command_id,
                    },
                });
            }
            // A toolbar button is a runnable command; the terminal has no chrome
            // bar, so it lands in the slash menu too. Skip ones already surfaced
            // as a slash command (a plugin can declare both for the same id).
            for tb in host.toolbar_buttons("tui") {
                if seen.insert((tb.plugin_id.clone(), tb.command_id.clone())) {
                    let label = tb.title.unwrap_or_else(|| tb.command_id.clone());
                    out.push(SlashCommand {
                        name: format!("{} {label}", tb.icon),
                        description: format!("plugin · {}", tb.plugin_id),
                        needs_args: false,
                        origin: SlashOrigin::Plugin {
                            plugin_id: tb.plugin_id,
                            command_id: tb.command_id,
                        },
                    });
                }
            }
        }
        out
    }

    /// Recompute the slash overlay's candidate list against the
    /// current query (fuzzy match on `name`).
    ///
    /// Query empty → registry order (no sort; `visual_order` handles
    /// grouping at render/nav time).
    /// Query non-empty → score desc, `name` as stable tiebreaker.
    /// Category grouping is always the renderer's job via `visual_order`;
    /// we never sort `candidates` by category here.
    pub(crate) fn refresh_slash(&mut self) {
        let Some(Overlay::Slash(ref state)) = self.overlay else {
            return;
        };
        let query = state.query.clone();
        let mut filtered: Vec<(i32, SlashCommand)> = self
            .slash_candidates()
            .into_iter()
            .filter_map(|entry| {
                if query.is_empty() {
                    Some((0, entry))
                } else {
                    crate::fuzzy::fuzzy_score(&query, &entry.name).map(|score| (score, entry))
                }
            })
            .collect();
        // With a filter: sort by score desc so the most relevant result
        // leads within each category bucket. Name is the stable tiebreaker.
        // Without a filter: preserve registry order (score is 0 for all,
        // so sorting would be a no-op anyway — skip it).
        if !query.is_empty() {
            filtered.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.name.cmp(&b.1.name)));
        }
        if let Some(Overlay::Slash(ref mut s)) = self.overlay {
            s.candidates = filtered.into_iter().map(|(_, c)| c).collect();
            // Reset selection to the first item in visual order so the
            // highlight is always on the first visible row after a filter
            // change. (The old clamp kept the index if it was still in
            // range, which diverged from visual order when categories
            // shifted under the new filter.)
            let first = crate::view::overlays::visual_order(&s.candidates)
                .into_iter()
                .next()
                .unwrap_or(0);
            s.selected = first;
        }
    }

    /// Move the slash overlay selection one step **down** in visual order.
    ///
    /// Uses [`crate::view::overlays::visual_order`] as the authoritative
    /// traversal sequence so navigation always matches the rendered layout,
    /// regardless of how `s.candidates` happens to be sorted internally.
    pub(crate) fn slash_select_next(&mut self) {
        let Some(Overlay::Slash(ref mut s)) = self.overlay else {
            return;
        };
        let order = crate::view::overlays::visual_order(&s.candidates);
        if order.is_empty() {
            return;
        }
        // Find where the current selection sits in the visual sequence,
        // then advance to the next position (clamped at the end).
        let pos = order.iter().position(|&i| i == s.selected).unwrap_or(0);
        if pos + 1 < order.len() {
            s.selected = order[pos + 1];
        }
    }

    /// Move the slash overlay selection one step **up** in visual order.
    ///
    /// Mirror of [`Self::slash_select_next`].
    pub(crate) fn slash_select_prev(&mut self) {
        let Some(Overlay::Slash(ref mut s)) = self.overlay else {
            return;
        };
        let order = crate::view::overlays::visual_order(&s.candidates);
        if order.is_empty() {
            return;
        }
        let pos = order.iter().position(|&i| i == s.selected).unwrap_or(0);
        if pos > 0 {
            s.selected = order[pos - 1];
        }
    }

    /// Accept the highlighted slash command. Commands with `needs_args`
    /// hand off to the vim palette pre-filled for arg entry; the
    /// arg-less ones dispatch through the registry directly.
    pub(crate) fn accept_slash(&mut self) -> Result<bool> {
        let pick = match &self.overlay {
            Some(Overlay::Slash(s)) => s.candidates.get(s.selected).cloned(),
            _ => None,
        };
        self.overlay = None;
        let Some(cmd) = pick else {
            return Ok(false);
        };
        match cmd.origin {
            SlashOrigin::Plugin {
                plugin_id,
                command_id,
            } => {
                self.run_plugin_command(&plugin_id, &command_id);
                Ok(false)
            }
            SlashOrigin::Builtin => {
                if cmd.needs_args {
                    self.overlay = Some(Overlay::Command(CommandState {
                        buffer: format!("{} ", cmd.name),
                    }));
                    return Ok(false);
                }
                let registry = self.command_registry.clone();
                registry.dispatch(self, &cmd.name)
            }
        }
    }

    pub(crate) fn open_command(&mut self) {
        self.overlay = Some(Overlay::Command(CommandState {
            buffer: String::new(),
        }));
    }

    // --- template picker ------------------------------------------------

    /// Open the template picker overlay. Lists every template in the
    /// workspace with fuzzy filtering. On accept, structural templates
    /// are instantiated at the cursor; callable templates are executed
    /// and the output is inserted as a block.
    pub(crate) fn open_template_picker(&mut self) {
        let all = outl_actions::list_templates(&self.workspace);
        let filtered: Vec<usize> = (0..all.len()).collect();
        self.overlay = Some(Overlay::TemplatePicker(TemplatePickerState {
            query: String::new(),
            all,
            filtered,
            selected: 0,
        }));
    }

    /// Refresh the filtered list as the user types.
    pub(crate) fn refresh_template_picker(&mut self) {
        let Some(Overlay::TemplatePicker(ref mut tp)) = self.overlay else {
            return;
        };
        let q = tp.query.to_lowercase();
        if q.is_empty() {
            tp.filtered = (0..tp.all.len()).collect();
        } else {
            tp.filtered = tp
                .all
                .iter()
                .enumerate()
                .filter(|(_, t)| {
                    t.name.to_lowercase().contains(&q) || t.slug.to_lowercase().contains(&q)
                })
                .map(|(i, _)| i)
                .collect();
        }
        tp.selected = 0;
    }

    /// Accept the highlighted template. Structural templates deep-copy
    /// under the cursor; callable templates execute and insert stdout.
    pub(crate) fn accept_template_picker(&mut self) -> Result<bool> {
        let pick = match &self.overlay {
            Some(Overlay::TemplatePicker(tp)) => tp
                .filtered
                .get(tp.selected)
                .and_then(|&i| tp.all.get(i).cloned()),
            _ => None,
        };
        self.overlay = None;
        let Some(tpl) = pick else {
            return Ok(false);
        };

        if tpl.params.is_empty() {
            self.instantiate_template_at_cursor(&tpl.name);
        } else {
            self.overlay = Some(Overlay::Command(CommandState {
                buffer: format!("template {} ", tpl.name),
            }));
        }
        Ok(false)
    }

    // --- inline autocomplete --------------------------------------------
    // `maybe_update_autocomplete`, `accept_autocomplete`, the
    // `candidates_for_*` helpers, and the `detect_trigger` free
    // function live in the sibling `actions::autocomplete` module so
    // each file owns one concern. Search this crate for those names
    // when wiring a new trigger.
}

/// Recursively walk an outline scoring each block's text against `query`.
pub(crate) fn collect_block_hits(
    blocks: &[OutlineNode],
    cursor: &mut usize,
    md_path: &Path,
    page_label: &str,
    page_icon: Option<&str>,
    query: &str,
    hits: &mut Vec<SearchHit>,
) {
    for b in blocks {
        if let Some(score) = crate::fuzzy::fuzzy_score(query, &b.text) {
            hits.push(SearchHit {
                page_label: page_label.to_string(),
                page_icon: page_icon.map(String::from),
                md_path: md_path.to_path_buf(),
                snippet: truncate_for_snippet(&b.text, 80),
                block_index: *cursor,
                score,
            });
        }
        *cursor += 1;
        collect_block_hits(
            &b.children,
            cursor,
            md_path,
            page_label,
            page_icon,
            query,
            hits,
        );
    }
}

pub(crate) fn truncate_for_snippet(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max_chars - 1).collect();
    out.push('…');
    out
}

/// Best-effort read of `title::` from a `.md` file. Returns the
/// (possibly trimmed) value or `None` if the file is unreadable or has
/// no title.
pub(crate) fn read_page_title(md: &Path) -> Option<String> {
    let text = std::fs::read_to_string(md).ok()?;
    let parsed = outl_md::parse::parse(&text);
    parsed
        .properties
        .iter()
        .find(|(k, _)| k == "title")
        .map(|(_, v)| v.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use crate::state::{App, Overlay, QuickSwitchState, SwitchCandidate, SwitchKind};
    use outl_core::id::ActorId;
    use outl_core::workspace::Workspace;
    use tempfile::TempDir;

    fn page(label: &str) -> SwitchCandidate {
        SwitchCandidate {
            label: label.to_string(),
            key: label.to_string(),
            kind: SwitchKind::Page,
            score: 0,
        }
    }

    fn test_app() -> (App, TempDir) {
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

    /// Regression: deleting query characters must restore candidates an
    /// earlier, longer query had filtered out. The bug was that
    /// `refresh_quick_switch` narrowed `candidates` in place, destroying
    /// the source set, so backspacing could never bring matches back.
    #[test]
    fn quick_switch_widens_when_query_shrinks() {
        let (mut app, _dir) = test_app();
        let all = vec![page("foo"), page("food"), page("bar")];
        app.overlay = Some(Overlay::QuickSwitch(QuickSwitchState {
            query: String::new(),
            all_candidates: all.clone(),
            candidates: all,
            selected: 0,
            preview_cache: std::cell::RefCell::new(None),
        }));

        // Type "food": only the "food" page survives.
        if let Some(Overlay::QuickSwitch(ref mut qs)) = app.overlay {
            qs.query = "food".into();
        }
        app.refresh_quick_switch();
        let after_food: Vec<String> = match &app.overlay {
            Some(Overlay::QuickSwitch(qs)) => {
                qs.candidates.iter().map(|c| c.label.clone()).collect()
            }
            _ => unreachable!(),
        };
        assert_eq!(after_food, vec!["food".to_string()]);

        // Backspace down to "foo": both "foo" and "food" must come back.
        if let Some(Overlay::QuickSwitch(ref mut qs)) = app.overlay {
            qs.query = "foo".into();
        }
        app.refresh_quick_switch();
        let mut after_foo: Vec<String> = match &app.overlay {
            Some(Overlay::QuickSwitch(qs)) => {
                qs.candidates.iter().map(|c| c.label.clone()).collect()
            }
            _ => unreachable!(),
        };
        after_foo.sort();
        assert_eq!(
            after_foo,
            vec!["foo".to_string(), "food".to_string()],
            "shrinking the query must restore candidates the longer query dropped"
        );
    }

    // `detect_trigger_mention_*` tests live in
    // `actions::autocomplete::tests` (same module as the function).

    // ---------------------------------------------------------------
    // Slash palette navigation — cross-category regression
    // ---------------------------------------------------------------

    /// Helper: open the slash palette, set the query, refresh, and
    /// return the list of command names in visual (paint) order.
    fn slash_visual_names(app: &mut App, query: &str) -> Vec<String> {
        app.open_slash();
        if !query.is_empty() {
            if let Some(Overlay::Slash(ref mut s)) = app.overlay {
                s.query = query.to_string();
            }
            app.refresh_slash();
        }
        let candidates = match &app.overlay {
            Some(Overlay::Slash(s)) => s.candidates.clone(),
            _ => unreachable!(),
        };
        let order = crate::view::overlays::visual_order(&candidates);
        order
            .into_iter()
            .map(|i| candidates[i].name.clone())
            .collect()
    }

    /// `slash_select_next` must walk items in the same order that the
    /// renderer paints them (category-first), not in raw `s.candidates`
    /// order. Repro: query `t` matches commands from at least three
    /// categories (Actions, Settings, Dates & time); without the fix
    /// the highlighted row jumped instead of moving to the next
    /// visible item.
    #[test]
    fn slash_nav_follows_visual_order_under_filter() {
        let (mut app, _dir) = test_app();
        let visual = slash_visual_names(&mut app, "t");
        // Must have matched something across categories — sanity check.
        assert!(
            visual.len() >= 2,
            "expected multiple matches for 't', got {visual:?}"
        );

        // Re-open with the same query, then walk with slash_select_next
        // and verify each step lands on the next item in visual order.
        app.open_slash();
        if let Some(Overlay::Slash(ref mut s)) = app.overlay {
            s.query = "t".to_string();
        }
        app.refresh_slash();

        // The initial selection must be the first item in visual order.
        let initial_selected = match &app.overlay {
            Some(Overlay::Slash(s)) => s.selected,
            _ => unreachable!(),
        };
        let candidates = match &app.overlay {
            Some(Overlay::Slash(s)) => s.candidates.clone(),
            _ => unreachable!(),
        };
        let order = crate::view::overlays::visual_order(&candidates);
        assert_eq!(
            initial_selected, order[0],
            "initial selection must be first in visual order"
        );

        // Walk forward through the whole list and confirm every step
        // matches the next position in `visual_order`, never jumping.
        let mut walked: Vec<String> = vec![candidates[initial_selected].name.clone()];
        for _ in 1..order.len() {
            app.slash_select_next();
            let sel = match &app.overlay {
                Some(Overlay::Slash(s)) => s.selected,
                _ => unreachable!(),
            };
            let cands = match &app.overlay {
                Some(Overlay::Slash(s)) => s.candidates.clone(),
                _ => unreachable!(),
            };
            let cur_order = crate::view::overlays::visual_order(&cands);
            let pos = cur_order
                .iter()
                .position(|&i| i == sel)
                .expect("selected must be in visual_order");
            assert_eq!(
                walked.len(),
                pos,
                "after {} next-presses, selection should be at visual position {}, got {pos}",
                walked.len(),
                walked.len()
            );
            walked.push(cands[sel].name.clone());
        }
        // Walked names must equal the visual order computed upfront.
        assert_eq!(walked, visual, "walked sequence must match visual order");
    }

    /// `slash_select_prev` mirrors `slash_select_next` and must never
    /// jump past the first item in visual order.
    #[test]
    fn slash_nav_prev_clamps_at_first_visual_item() {
        let (mut app, _dir) = test_app();
        app.open_slash();
        // No filter — full list.
        let first_visual_idx = {
            let candidates = match &app.overlay {
                Some(Overlay::Slash(s)) => s.candidates.clone(),
                _ => unreachable!(),
            };
            crate::view::overlays::visual_order(&candidates)[0]
        };
        // Start is already at first; pressing prev must stay there.
        app.slash_select_prev();
        let sel = match &app.overlay {
            Some(Overlay::Slash(s)) => s.selected,
            _ => unreachable!(),
        };
        assert_eq!(
            sel, first_visual_idx,
            "prev at the first item must not move"
        );
    }

    /// Query-empty palette: `open_slash` + `slash_select_next` walking
    /// the whole list must cover every registered command exactly once,
    /// in canonical category order (Actions → Navigation → Search →
    /// Settings → Dates & time).
    #[test]
    fn slash_nav_empty_query_covers_all_commands_in_category_order() {
        let (mut app, _dir) = test_app();
        app.open_slash();
        let candidates = match &app.overlay {
            Some(Overlay::Slash(s)) => s.candidates.clone(),
            _ => unreachable!(),
        };
        let order = crate::view::overlays::visual_order(&candidates);
        // Walk every item.
        let mut walked_names: Vec<String> = {
            let sel = match &app.overlay {
                Some(Overlay::Slash(s)) => s.selected,
                _ => unreachable!(),
            };
            vec![candidates[sel].name.clone()]
        };
        for _ in 1..order.len() {
            app.slash_select_next();
            let sel = match &app.overlay {
                Some(Overlay::Slash(s)) => s.selected,
                _ => unreachable!(),
            };
            walked_names.push(candidates[sel].name.clone());
        }
        // Deduplicate to check all commands were visited.
        let mut sorted_walked = walked_names.clone();
        sorted_walked.sort();
        sorted_walked.dedup();
        let mut sorted_expected: Vec<String> = candidates.iter().map(|c| c.name.clone()).collect();
        sorted_expected.sort();
        assert_eq!(
            sorted_walked, sorted_expected,
            "walking the full list must visit every command exactly once"
        );
        // Category order must be non-decreasing.
        let cat_orders: Vec<u8> = walked_names
            .iter()
            .map(|n| crate::view::overlays::category_order_for(n))
            .collect();
        let is_non_decreasing = cat_orders.windows(2).all(|w| w[0] <= w[1]);
        assert!(
            is_non_decreasing,
            "category order must be non-decreasing while walking; got {cat_orders:?}"
        );
    }
}
