//! Cross-page edits triggered while the focus is on a backlink.
//!
//! `apply_to_backlink_source` is the workhorse: it loads the source
//! `.md`, lets the caller mutate its AST + the focused block's path,
//! then writes the result through `save_page_with`. Used by every
//! structural op in `structural.rs` when `Focus::Backlink` is
//! active, and by `toggle_todo_backlink` here.

use crate::outline_ops::node_at_path_mut;
use crate::state::{App, EditTarget, Focus, Mode};
use outl_md::parse::{parse, ParsedPage};

impl App {
    /// Run an op on the source page of the focused backlink and persist.
    ///
    /// `f` receives the source AST (mutable) and the absolute path of
    /// the currently focused block inside that AST, also mutable. The
    /// closure mutates the tree and (when the op changes where the
    /// focused block lives — indent/outdent/move) updates the path
    /// in place. After `f` returns:
    ///
    /// 1. `Focus.sub_path` is rewritten from the post-op absolute path
    ///    (relative to `source_block_path`; clamped to `[]` if the op
    ///    moved the block out of the backlink's scope).
    /// 2. The page is saved with `save_page_with(.., true)` so the
    ///    source page's index entry (block refs, page title, icon)
    ///    is patched incrementally. Backlinks themselves are
    ///    computed straight from the workspace
    ///    (`backlinks_for_current`); the post-op tree is visible on
    ///    the next render with no extra cache touch.
    ///
    /// Returns the post-op `ParsedPage` so callers that need to follow
    /// up (e.g. enter Insert on the newly created block) can reuse it
    /// without re-reading from disk.
    pub(super) fn apply_to_backlink_source<F>(
        &mut self,
        mut f: F,
    ) -> Option<(std::path::PathBuf, ParsedPage)>
    where
        F: FnMut(&mut ParsedPage, &mut Vec<usize>),
    {
        let (source_path, source_block_path) = match &self.focus {
            Focus::Backlink { idx, .. } => {
                let backlinks = self.backlinks_for_current();
                let bl = backlinks.get(*idx)?;
                (bl.source_path.clone()?, bl.source_block_path.clone())
            }
            _ => return None,
        };
        let sub_path = match &self.focus {
            Focus::Backlink { sub_path, .. } => sub_path.clone(),
            _ => return None,
        };

        // Reuse the working copy if we're already mid-edit on this
        // source — avoids re-reading the file *and* preserves the
        // user's in-flight buffer (we commit it into the AST below).
        let mut source_page = if let Mode::Insert {
            target: EditTarget::SourcePage { path, page },
            block_path,
            buffer,
            ..
        } = &self.mode
        {
            if path == &source_path {
                let mut p = page.clone();
                if let Some(node) = node_at_path_mut(&mut p.blocks, block_path) {
                    node.text = buffer.as_string();
                }
                p
            } else {
                parse(&std::fs::read_to_string(&source_path).ok()?)
            }
        } else {
            parse(&std::fs::read_to_string(&source_path).ok()?)
        };

        let mut abs_path = source_block_path.clone();
        abs_path.extend_from_slice(&sub_path);

        // Snapshot the file this op is about to change, under its own
        // path. Every structural op reachable from the backlinks panel
        // routes through here, so recording it once covers all of them
        // instead of seven call sites each remembering to
        // ([#431](https://github.com/outlmd/outl/issues/431)).
        self.snapshot_page_for_undo(source_path.clone(), source_page.clone());

        f(&mut source_page, &mut abs_path);

        // Rewrite focus.sub_path from the post-op absolute path. If
        // the op pushed the focused block out of the backlink's
        // visible scope (i.e. it's no longer a descendant of
        // source_block_path), fall back to focusing the source block
        // itself.
        let new_sub_path: Vec<usize> = if abs_path.starts_with(&source_block_path) {
            abs_path[source_block_path.len()..].to_vec()
        } else {
            Vec::new()
        };
        if let Focus::Backlink { sub_path, .. } = &mut self.focus {
            *sub_path = new_sub_path;
        }

        // `rebuild_index = true` runs `WorkspaceIndex::patch_page` on
        // the source page so block-ref resolution stays current after
        // the structural op. Workspace-driven `backlinks_for_current`
        // already sees the new tree as soon as `reconcile_md`
        // finishes, no extra cache touch needed for that side.
        self.save_page_with(&source_path, &source_page, true);

        Some((source_path, source_page))
    }

    /// Cross-page TODO cycle: load the source page, mutate the
    /// referencing block (resolved via `source_block_path + sub_path`),
    /// and save it through `save_page_with` so reconcile keeps IDs stable.
    ///
    /// Snapshots the source page under its own path, like the
    /// structural path above. This used to record nothing, on the
    /// grounds that "undo in this view shouldn't silently flip TODOs in
    /// a different file" — true while a popped snapshot was applied
    /// blind, and no longer the case now that `top_applies_here` keys
    /// restoration on the page being open
    /// ([#430](https://github.com/outlmd/outl/issues/430),
    /// [#431](https://github.com/outlmd/outl/issues/431)).
    /// `commit_insert` on a `SourcePage` still records nothing and is
    /// the remaining instance of the old reasoning.
    pub(super) fn toggle_todo_backlink(&mut self, idx: usize, sub_path: &[usize]) {
        let (source_path, abs_path) = {
            let backlinks = self.backlinks_for_current();
            let Some(bl) = backlinks.get(idx) else {
                return;
            };
            let Some(path) = bl.source_path.clone() else {
                return;
            };
            let mut full = bl.source_block_path.clone();
            full.extend_from_slice(sub_path);
            (path, full)
        };
        let Ok(text) = std::fs::read_to_string(&source_path) else {
            self.status = format!("cannot read backlink source: {}", source_path.display());
            return;
        };
        let mut source_page = parse(&text);
        // Captured before the mutation and only recorded once the
        // target block is known to exist, so a stale index cannot push
        // a snapshot for an op that did nothing.
        let before = source_page.clone();
        let Some(node) = node_at_path_mut(&mut source_page.blocks, &abs_path) else {
            self.status = "backlink source block missing — index may be stale".into();
            return;
        };
        node.text = super::cycle_todo_state(&node.text);
        self.snapshot_page_for_undo(source_path.clone(), before);
        // Same rationale as the structural cross-page path: patch the
        // source page's index entry so `((blk-XXXXXX))` resolution
        // doesn't lag the TODO/DONE flip until a full rebuild.
        self.save_page_with(&source_path, &source_page, true);
    }
}

#[cfg(test)]
mod tests {
    //! A structural op driven from the backlinks panel mutates a file
    //! that is not on screen, and used to leave no undo entry at all
    //! ([#431](https://github.com/outlmd/outl/issues/431)).
    //!
    //! These build a real two-page workspace and a real backlink index
    //! rather than poking `App` state: the op resolves its target
    //! through `backlinks_for_current`, so a test that fakes that has
    //! not exercised the path that loses the snapshot.

    use crate::state::{App, Focus, View};
    use outl_core::hlc::HlcGenerator;
    use outl_core::{ActorId, Workspace};
    use outl_md::reconcile::reconcile_md;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// `pages/target.md` is on screen; `pages/source.md` mentions it,
    /// so it shows up in the backlinks panel below.
    fn workspace_with_a_backlink() -> (App, TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("pages")).unwrap();
        let target = dir.path().join("pages").join("target.md");
        let source = dir.path().join("pages").join("source.md");
        fs::write(&target, "- target body\n").unwrap();
        fs::write(&source, "- mentions [[target]]\n- second line\n").unwrap();

        let actor = ActorId::new();
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let hlc = HlcGenerator::new(actor);
        reconcile_md(&mut ws, &hlc, &target, None).unwrap();
        reconcile_md(&mut ws, &hlc, &source, None).unwrap();

        let metas: Vec<_> = ["target", "source"]
            .iter()
            .filter_map(|s| outl_actions::find_by_slug(&ws, s))
            .filter_map(|id| outl_actions::page_meta(&ws, id))
            .collect();
        assert_eq!(metas.len(), 2, "setup: both pages must be in the workspace");
        let index = outl_actions::build_backlink_index_from_disk(&metas, dir.path());

        let mut app = App::new_for_tests(
            dir.path().to_path_buf(),
            ws,
            actor,
            crate::theme::default_theme(),
            false,
        )
        .unwrap();
        *app.backlink_index.borrow_mut() = Some(index);
        app.view = View::Page(target);
        app.load_current_no_autorun();
        assert!(
            !app.backlinks_for_current().is_empty(),
            "setup: target must have a backlink from source"
        );
        app.focus = Focus::Backlink {
            idx: 0,
            sub_path: vec![],
        };
        (app, dir, source)
    }

    #[test]
    fn deleting_a_block_from_the_backlinks_panel_leaves_an_undo_entry() {
        let (mut app, _d, source) = workspace_with_a_backlink();
        app.undo.clear();

        app.delete_current();

        assert!(
            app.undo.iter().any(|s| s.view_path == source),
            "the cross-page delete left no undo entry for {}; stack holds {:?}",
            source.display(),
            app.undo.iter().map(|s| &s.view_path).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_snapshot_holds_the_source_page_as_it_was_before_the_delete() {
        let (mut app, _d, source) = workspace_with_a_backlink();
        app.undo.clear();

        app.delete_current();

        let snap = app
            .undo
            .iter()
            .find(|s| s.view_path == source)
            .expect("a snapshot for the source page");
        assert!(
            snap.page
                .blocks
                .iter()
                .any(|b| b.text.contains("mentions [[target]]")),
            "the snapshot must carry the pre-delete text, got {:?}",
            snap.page.blocks.iter().map(|b| &b.text).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_snapshot_does_not_apply_to_the_page_on_screen() {
        // It belongs to the source page, so pressing `u` while still
        // looking at the target must not restore it here. That guard is
        // `top_applies_here` (#430), and it is what makes carrying a
        // cross-page snapshot safe in the first place.
        let (mut app, _d, source) = workspace_with_a_backlink();
        app.undo.clear();
        app.delete_current();

        let before = app.undo.len();
        app.undo();

        assert_eq!(
            app.undo.len(),
            before,
            "the cross-page snapshot was consumed by an undo on the wrong page"
        );
        assert_eq!(app.status, "no undo for this page");
        assert!(app.undo.iter().any(|s| s.view_path == source));
    }
}
