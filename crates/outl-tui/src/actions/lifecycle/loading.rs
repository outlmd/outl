//! Switch the currently-opened view (journal or page) and pull its
//! state from disk: parse the `.md`, rehydrate the sidecar's stable
//! ids, refresh the page list, push the path through the recent-LRU.
//!
//! Loading also resets the focused `Focus` (a stale `Focus::Backlink`
//! would point at the previous page's backlinks list). The backlink
//! index itself is whole-workspace, not per-view, so a plain view
//! switch does **not** touch it — it's keyed by slug on read.

use crate::outline_ops::flat_count;
use crate::state::{App, Focus, ToastKind};
use anyhow::{Context, Result};
use outl_md::parse::parse;
use outl_md::reconcile::reconcile_md;
use std::fs;
use std::path::{Path, PathBuf};

use super::file_mtime;

impl App {
    /// Walk `pages/` and capture every `.md` (skipping dotfiles) into
    /// `page_list`. Used by the quick-switcher and the recent-LRU
    /// merger.
    pub(crate) fn refresh_page_list(&mut self) {
        let pages_dir = self.workspace_root.join("pages");
        let mut entries: Vec<PathBuf> = walkdir::WalkDir::new(&pages_dir)
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
        entries.sort();
        self.page_list = entries;
    }

    /// Create the underlying `.md` file (with a single empty block) if it
    /// doesn't already exist. Ensures the editor always has a target.
    pub(crate) fn ensure_view_file_exists(&mut self) -> Result<()> {
        let path = self.current_path();
        if path.exists() {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
        }
        // Seed an empty outline (one empty bullet so cursor has a home).
        outl_md::write_atomic(&path, b"- \n")
            .with_context(|| format!("create {}", path.display()))?;
        // Reconcile so the sidecar exists with stable IDs.
        let _ = reconcile_md(
            &mut self.workspace,
            &self.hlc,
            &path,
            Some(&self.orphans_log),
        );
        self.refresh_page_list();
        Ok(())
    }

    /// Reparse the current page from disk + (re)trigger auto-run.
    ///
    /// Most navigation paths call this. The auto-run pass is what
    /// makes `auto-run::` blocks "feel live" — open a journal, the
    /// computed cells run themselves.
    pub(crate) fn load_current(&mut self) {
        self.load_current_no_autorun();
        self.run_auto_run_blocks();
    }

    /// Bare reparse from disk, **without** the auto-run pass.
    ///
    /// Internal: used by `run_auto_run_blocks` itself after it writes
    /// new result subblocks, to refresh the in-memory AST without
    /// firing another round of auto-runs (which would loop forever
    /// when something stamps a fresh hash but the hash doesn't stick
    /// for some reason).
    pub(crate) fn load_current_no_autorun(&mut self) {
        // Reparsing from disk would clobber a coalesced edit that only
        // lives in the in-memory AST, so persist it first. Reentrant-
        // safe: `flush_pending_save` clears the dirty mark before it
        // runs `persist` → `run_auto_run_blocks` → back here, so the
        // nested call is a no-op.
        self.flush_pending_save();
        // Navigation does NOT invalidate the backlink index: it's a
        // whole-workspace index, so the new page is just a different
        // lookup in the same index — no rebuild. (A peer reload, which
        // does change the workspace, invalidates separately in
        // `peer_sync`.) Invalidating here rebuilt the index from every
        // `.md` on every page switch, inline.
        let path = self.current_path();
        // A page that isn't on disk yet is legitimately empty; a page
        // we *can't read* is not. Treating the latter as `""` would
        // parse to an empty AST that the next commit renders straight
        // back over the user's file. `load_failed` fences that off —
        // see the field's docs and the guard at the top of `persist`.
        let text = match outl_md::read_for_rewrite(&path) {
            Ok(t) => {
                self.load_failed = false;
                t
            }
            Err(e) => {
                self.load_failed = true;
                self.toast(
                    ToastKind::Error,
                    format!("cannot read {}: {e} — editing disabled", path.display()),
                );
                String::new()
            }
        };
        self.page = parse(&text);
        self.parse_warnings = self.page.warnings.clone();
        // Surface a brief chip in the status line so the user notices
        // even if they're not looking at the outline. We OWN the chip
        // text (any status starting with the marker below is ours),
        // so we can both refresh and clear it across reloads without
        // touching messages other code paths set (save error, chord
        // prompt, etc.).
        //
        // Refresh — warnings present and the slot is either empty or
        //   our own chip from a previous load.
        // Clear — warnings gone and the slot is our chip (otherwise
        //   we'd erase someone else's message).
        // Untouched — slot has a non-chip status (save error etc.):
        //   the user reads that first, the banner above the outline
        //   stays as the persistent warning signal.
        let chip_marker = format!("{} ", self.icons.warning);
        let chip_is_ours =
            self.status.starts_with(&chip_marker) && self.status.contains("outside outl dialect");
        if self.parse_warnings.is_empty() {
            if chip_is_ours {
                self.status.clear();
            }
        } else if self.status.is_empty() || chip_is_ours {
            self.status = format!(
                "{}{} line(s) outside outl dialect — preserved (open :warnings to see)",
                chip_marker,
                self.parse_warnings.len()
            );
        }
        self.flat_len = flat_count(&self.page.blocks);
        if self.selected >= self.flat_len {
            self.selected = self.flat_len.saturating_sub(1);
        }
        // Any view change snaps focus back to the outline. Carrying a
        // stale `Focus::Backlink { idx, … }` across pages would point
        // at the wrong backlink list (the new page has its own).
        self.focus = Focus::Outline;
        // A zoom root is a path into the *previous* page's AST — it
        // means nothing on the freshly-loaded one, so every view switch
        // resets the zoom back to the whole page.
        self.zoom_stack.clear();
        // Rebuild the flat-index → NodeId mapping from the sidecar
        // (sidecar blocks are already DFS-preorder, so they line up
        // with the render walk's `cursor`) and hydrate the collapsed
        // mirror from `workspace.tree().is_collapsed(_)`. The op log
        // is the single source of truth across devices — see
        // `outl_core::op::Op::SetCollapsed`. The sidecar contributes
        // only the id mapping; a missing or unreadable sidecar
        // leaves both structures empty until the next reconcile
        // populates `.outl`.
        self.id_by_flat.clear();
        self.collapsed.clear();
        let sidecar_path = outl_md::resolve_sidecar_path(&path);
        if let Ok(sc) = outl_md::sidecar::read(&sidecar_path) {
            self.id_by_flat.reserve(sc.blocks.len());
            for b in &sc.blocks {
                self.id_by_flat.push(b.id);
                if self.workspace.tree().is_collapsed(b.id) {
                    self.collapsed.insert(b.id);
                }
            }
        }
        self.recompute_hidden_by_collapse();
        // Re-run content transformers for the freshly-parsed AST. Needs
        // `id_by_flat` (built just above) since the cache is keyed by
        // NodeId. No-op when no plugin host / no text transformers.
        self.recompute_transforms();
        // Snapshot the file's mtime so the polling loop can tell when
        // an *external* edit lands (vs. our own save).
        self.last_mtime = file_mtime(&path);
        // Anchor the header's freshness chip ("⟳ 2s ago") to the load
        // instant: from the user's perspective, what's on screen *is*
        // what's on disk at this moment.
        self.last_saved_at = Some(std::time::Instant::now());
        self.touch_recent(&path);
    }

    /// Move `path` to the front of the recent-paths LRU. Used by
    /// `load_current_no_autorun` so any view switch (journal, page,
    /// switch-overlay open) keeps the sidebar's `Recent` list in
    /// sync with what the user actually touched.
    ///
    /// Capped at 20 entries — anything past that drops off the back,
    /// which is enough for a session's worth of context without
    /// turning into infinite scroll.
    pub(crate) fn touch_recent(&mut self, path: &Path) {
        const RECENT_MAX: usize = 20;
        self.recent_paths.retain(|p| p != path);
        self.recent_paths.insert(0, path.to_path_buf());
        self.recent_paths.truncate(RECENT_MAX);
    }
}

#[cfg(test)]
mod tests {
    use crate::state::App;
    use outl_core::{ActorId, Workspace};
    use tempfile::TempDir;

    fn today_journal_path(dir: &std::path::Path) -> std::path::PathBuf {
        dir.join("journals").join(format!(
            "{}.md",
            outl_actions::clock::today().format("%Y-%m-%d")
        ))
    }

    /// Regression for the icon-style boot race: the parse-warning
    /// status chip is stamped with `icons.warning` inside the first
    /// `load_current` (from `App::new`), and the clear/refresh path
    /// only recognises a chip carrying *its own* marker. When the
    /// runtime assigned the configured `IconSet` only *after*
    /// `App::new` returned, a nerd-font launch booted with an emoji
    /// `⚠` chip that no later reload could clear.
    #[test]
    fn boot_stamps_the_warning_chip_with_the_configured_icon_set() {
        let dir = TempDir::new().unwrap();
        let path = today_journal_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "a line outside the outline dialect\n").unwrap();

        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
        let mut app = App::new(
            dir.path().to_path_buf(),
            ws,
            actor,
            crate::theme::default_theme(),
            false,
            outl_config::TuiIconStyle::NerdFont,
        )
        .unwrap();

        assert!(!app.parse_warnings.is_empty(), "fixture must warn");
        assert_eq!(app.icons.warning, "\u{f071}");
        let nerd_chip = format!("{} ", app.icons.warning);
        assert!(
            app.status.starts_with(&nerd_chip),
            "the boot chip must carry the configured icon set, got {:?}",
            app.status
        );

        // The clear path only trusts a chip stamped with its own
        // marker, so the fix is only real if the second load can
        // actually retire the first one.
        std::fs::write(&path, "- back inside the dialect\n").unwrap();
        app.load_current();
        assert!(app.parse_warnings.is_empty());
        assert_eq!(app.status, "", "the chip must clear on the next load");
    }
}
