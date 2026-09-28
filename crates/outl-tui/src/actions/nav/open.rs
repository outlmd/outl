//! Changing which page or journal is open.
//!
//! Every entry point that swaps `App::view` lives here: the journal
//! jumps, the `[[ref]]` / `#tag` / `((blk-X))` under the cursor, and
//! opening a page by name or by on-disk slug. Reading *which* view is
//! open is the parent module; this one is the write side.

use crate::state::{App, View};
use anyhow::Result;
use chrono::Duration;
use outl_actions::clock;
use outl_md::inline::{ref_at_cursor, RefTarget};
use outl_md::reconcile::reconcile_md;
use std::fs;

impl App {
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
mod open_page_tests {
    use crate::state::App;
    use outl_core::{ActorId, Workspace};
    use tempfile::TempDir;

    fn fresh_app() -> (App, TempDir) {
        let dir = TempDir::new().unwrap();
        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
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
