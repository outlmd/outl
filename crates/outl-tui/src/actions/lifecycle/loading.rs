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
use outl_actions::refusal::Surface;
use outl_actions::ProjectionRefusal;
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
        // Refresh the `.md` from the tree **before** reading it back.
        //
        // A peer's ops land in the op log, not in the `.md`, so without
        // this the page renders whatever the projection held when it was
        // last written — stale, or empty (issue #166). Every GUI open
        // path has done this since; the TUI never did, which also made it
        // the one client that could not tell the user when the refresh is
        // *refused* (issue #292). Both are the same call.
        self.refresh_md_from_tree();
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

    /// Re-project the open page's `.md` when the tree has moved past
    /// it, and record the one refusal the **user** has to hear about.
    ///
    /// `outl_actions::reproject_stale_md` owns both halves — the write
    /// gate (root `CLAUDE.md` invariant 8) and the split between "this
    /// page stopped syncing" and "retry later". The TUI only decides
    /// where each one goes:
    ///
    /// - [`ProjectionRefusal::AheadOfLog`] → `md_ahead_of_log`, which
    ///   the banner above the outline renders until a load clears it.
    ///   The page still opens, showing exactly what is on disk: the
    ///   guard withheld a *write*, so trading a stale page for no page
    ///   would be the worse half of the deal.
    /// - [`ProjectionRefusal::Retryable`] → a log line. An I/O error or
    ///   an unreadable sidecar is picked up by the next open or the
    ///   orphan scanner, and nagging about it would train the user past
    ///   the banner that matters.
    ///
    /// Silent on a page the workspace has no node for. A page the user
    /// just created is reconciled a moment later by
    /// [`Self::ensure_view_file_exists`] / `open_page_slug`, and a page
    /// whose `.md` arrived with no sidecar belongs to the orphan
    /// scanner — neither is something to put on screen.
    fn refresh_md_from_tree(&mut self) {
        let Some(root) = self.workspace.root.clone() else {
            return;
        };
        let slug = self.current_slug();
        let Some(page_id) = outl_actions::find_by_slug(&self.workspace, &slug) else {
            return;
        };
        let refusal = outl_actions::reproject_stale_md(&self.workspace, &root, page_id);
        self.md_ahead_of_log = match refusal {
            Some(ProjectionRefusal::AheadOfLog { lines, sample, .. }) => Some(
                outl_actions::refusal::ahead_of_log_notice(Surface::Tui, lines, &sample),
            ),
            Some(ProjectionRefusal::Retryable(message)) => {
                tracing::warn!("{}: re-projection failed: {message}", slug);
                None
            }
            None => None,
        };
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
    use outl_core::storage::JsonlStorage;
    use outl_core::{ActorId, Workspace};
    use tempfile::TempDir;

    fn today_journal_path(dir: &std::path::Path) -> std::path::PathBuf {
        dir.join("journals").join(format!(
            "{}.md",
            outl_actions::clock::today().format("%Y-%m-%d")
        ))
    }

    /// An app on a real workspace root, so the page-open re-projection
    /// has somewhere to write. `Workspace::open_in_memory` has
    /// `root: None`, which makes every assertion here vacuous.
    fn app_with_root() -> (App, TempDir) {
        let dir = TempDir::new().unwrap();
        let actor = ActorId::new();
        let storage = JsonlStorage::open(dir.path().join("ops"), actor).unwrap();
        let ws =
            Workspace::open_with_storage(actor, Box::new(storage), Some(dir.path().to_path_buf()))
                .unwrap();
        let app = App::new_for_tests(
            dir.path().to_path_buf(),
            ws,
            actor,
            crate::theme::default_theme(),
            false,
        )
        .unwrap();
        (app, dir)
    }

    /// Give the open journal one real block and a faithful projection.
    ///
    /// The block matters: a sidecar whose every block carries `text: ""`
    /// is read as pre-0.11 and cannot vouch for the file, so the guard
    /// stands down and nothing would be refused. See
    /// `outl_md::unlogged::sidecar_can_vouch_for`.
    fn seed_projected_block(app: &mut App) {
        let slug = app.current_slug();
        let page = outl_actions::find_by_slug(&app.workspace, &slug).unwrap();
        outl_actions::append_block(&mut app.workspace, &app.hlc, Some(page), Some("first"))
            .unwrap();
        outl_actions::apply_page_md_with_sidecar(&app.workspace, &app.workspace_root, page)
            .unwrap();
        app.load_current_no_autorun();
    }

    /// Append a line the op log never saw, and re-stamp the sidecar so
    /// the hash gate calls those exact bytes faithful — the state
    /// invariant 8 refuses to overwrite.
    fn freeze_open_page(app: &mut App) -> std::path::PathBuf {
        let md_path = app.current_path();
        let mut md = std::fs::read_to_string(&md_path).unwrap();
        md.push_str("- only ever on disk\n");
        std::fs::write(&md_path, &md).unwrap();
        let sidecar_path = outl_md::sidecar::sidecar_path_for(&md_path);
        let mut sc = outl_md::sidecar::read(&sidecar_path).unwrap();
        sc.last_synced_hash = outl_md::sidecar::file_hash(&md);
        outl_md::sidecar::write(&sidecar_path, &sc).unwrap();
        // The in-memory AST is a revision behind the bytes we just
        // wrote, and `load_current_no_autorun` flushes before it reads.
        // Parsing them in keeps that flush a true no-op, so the page is
        // still ahead of the log when the gate asks.
        app.page = outl_md::parse::parse(&md);
        md_path
    }

    /// The #292 bug: opening a page that has stopped syncing told the
    /// user nothing. Reading it looked completely normal — the `.md` is
    /// on disk and renders fine; what is missing is on the *other*
    /// devices, which is why the machine you are sitting at has no way
    /// to hint at it.
    #[test]
    fn opening_a_frozen_page_says_so() {
        let (mut app, _dir) = app_with_root();
        seed_projected_block(&mut app);
        assert!(
            app.md_ahead_of_log.is_none(),
            "a healthy page must not be flagged before the freeze"
        );

        let md_path = freeze_open_page(&mut app);
        app.load_current_no_autorun();

        let notice = app
            .md_ahead_of_log
            .as_ref()
            .expect("opening a frozen page must name the condition");
        assert_eq!(
            notice.command,
            outl_actions::error::AHEAD_OF_LOG_RECOVERY_COMMAND
        );
        assert!(
            notice.sample.contains("only ever on disk"),
            "the notice must name what is at risk, got {:?}",
            notice.sample
        );
        assert!(
            notice.body.starts_with("1 line in"),
            "the count comes from the guard, got {:?}",
            notice.body
        );
        assert!(
            std::fs::read_to_string(&md_path)
                .unwrap()
                .contains("only ever on disk"),
            "the refused write must leave the bytes alone"
        );
    }

    /// The page still opens, showing exactly what is on disk. The guard
    /// withheld a *write*; trading a readable stale page for no page at
    /// all would be the worse half of the deal, on the hottest path in
    /// the client.
    #[test]
    fn a_frozen_page_still_opens_with_its_content() {
        let (mut app, _dir) = app_with_root();
        seed_projected_block(&mut app);
        freeze_open_page(&mut app);

        app.load_current_no_autorun();

        let texts: Vec<&str> = app.page.blocks.iter().map(|b| b.text.as_str()).collect();
        assert!(
            texts.contains(&"first") && texts.contains(&"only ever on disk"),
            "the frozen page must render its on-disk content: {texts:?}"
        );
        assert!(!app.load_failed, "a refused write is not a read failure");
    }

    /// The notice is cleared by the next load that gets a projection
    /// through — i.e. the next open after the recovery ran. A banner
    /// that outlives the condition trains the user past it.
    #[test]
    fn a_page_that_is_no_longer_frozen_clears_the_notice() {
        let (mut app, _dir) = app_with_root();
        seed_projected_block(&mut app);
        freeze_open_page(&mut app);
        app.load_current_no_autorun();
        assert!(
            app.md_ahead_of_log.is_some(),
            "fixture must freeze the page"
        );

        // Stand in for `outl reconcile --ahead-of-log`, including the
        // part that makes it a separate flag: a frozen page is
        // hash-faithful, so `reconcile_md` short-circuits on the
        // recorded hash and never looks at it. The recovery clears that
        // hash first; without this the test asserts the wrong thing and
        // passes for the wrong reason.
        let md_path = app.current_path();
        let sidecar_path = outl_md::sidecar::sidecar_path_for(&md_path);
        let mut sc = outl_md::sidecar::read(&sidecar_path).unwrap();
        sc.last_synced_hash.clear();
        outl_md::sidecar::write(&sidecar_path, &sc).unwrap();
        outl_md::reconcile::reconcile_md(&mut app.workspace, &app.hlc, &md_path, None).unwrap();
        app.load_current_no_autorun();

        assert!(
            app.md_ahead_of_log.is_none(),
            "a page whose lines reached the log must stop being flagged"
        );
    }

    /// The other half of the same call, and a bug of its own: the TUI
    /// never re-projected on open, so a peer's ops sat in the op log
    /// while the page kept rendering the projection from before they
    /// arrived (issue #166, fixed in the GUI clients only).
    #[test]
    fn opening_a_page_the_tree_outran_refreshes_the_md() {
        let (mut app, _dir) = app_with_root();
        seed_projected_block(&mut app);

        // What a peer's ops look like locally: in the tree, not in the
        // `.md`.
        let slug = app.current_slug();
        let page = outl_actions::find_by_slug(&app.workspace, &slug).unwrap();
        outl_actions::append_block(&mut app.workspace, &app.hlc, Some(page), Some("synced-in"))
            .unwrap();
        let md_path = app.current_path();
        assert!(!std::fs::read_to_string(&md_path)
            .unwrap()
            .contains("synced-in"));

        app.load_current_no_autorun();

        assert!(
            std::fs::read_to_string(&md_path)
                .unwrap()
                .contains("synced-in"),
            "opening the page must refresh its projection from the tree"
        );
        let texts: Vec<&str> = app.page.blocks.iter().map(|b| b.text.as_str()).collect();
        assert!(
            texts.contains(&"synced-in"),
            "and the view must show it: {texts:?}"
        );
        assert!(app.md_ahead_of_log.is_none());
    }

    /// A page the workspace has no node for is not a refusal to report.
    /// `find_by_slug` comes back empty for a page the user just typed
    /// into existence, a split second before `open_page_slug` reconciles
    /// it — putting a banner on that would flag every new page.
    #[test]
    fn a_page_the_tree_does_not_know_is_not_flagged() {
        let (mut app, _dir) = app_with_root();
        app.open_page_by_name("brand new").unwrap();
        assert!(app.md_ahead_of_log.is_none());
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
