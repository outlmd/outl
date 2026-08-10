//! Zoom in / focus on a block (Roam / Workflowy "zoom into a block").
//!
//! `z i` makes the selected block the render root: only its subtree is
//! drawn and `j` / `k` are confined to it. `z o` pops one level back
//! out. The whole page is shown when the [`App::zoom_stack`] is empty.
//!
//! The zoom is a **render + navigation window only** — it never touches
//! the AST or disk. `selected` and `id_by_flat` stay whole-page flat
//! indices, so edits / inserts / TODO toggles keep operating on the
//! full [`outl_md::parse::ParsedPage`] regardless of the zoom. The
//! render walk in `view::outline` and the navigation clamps in
//! `actions::nav` consult [`App::zoom_root_window`] to know which slice
//! of the flat index is currently visible.
//!
//! The zoom root is a **path** (`Vec<usize>` of child indices), not an
//! id: the TUI already navigates the in-flight AST by path via
//! [`outl_md::outline_ops`], so a path is the native handle and the
//! breadcrumb is a plain walk down the ancestors of that path. This is
//! the same "in-flight AST" exception `outline_ops` is granted (see the
//! crate CLAUDE.md).

use crate::outline_ops::{index_for_path, node_at_path, path_for_index};
use crate::state::App;
use outl_md::outline_ops::flat_count;

impl App {
    /// Zoom into the currently selected block (`z i`).
    ///
    /// Pushes the block's DFS path onto [`App::zoom_stack`] and moves
    /// the selection to that block (the new render root). Zooming a leaf
    /// is allowed — Workflowy shows just that block — so we never refuse
    /// based on whether the block has children. No-op only when there's
    /// no resolvable selection (empty page).
    pub(crate) fn zoom_in_on_selected(&mut self) {
        let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
            return;
        };
        // Guard against a duplicate push if the user hits `z i` twice on
        // the same already-focused root — keeps the stack meaningful as
        // a breadcrumb.
        if self.zoom_stack.last() == Some(&path) {
            return;
        }
        self.zoom_stack.push(path);
        self.selected = self.zoom_root_index().unwrap_or(self.selected);
        self.cursor_col = 0;
        self.status = "zoomed in".into();
    }

    /// Zoom back out one level (`z o`). No-op (silent) at the page root.
    pub(crate) fn zoom_out(&mut self) {
        if self.zoom_stack.pop().is_none() {
            return;
        }
        // Land the selection on the new root (the parent we just
        // surfaced), or the top of the page when fully zoomed out.
        self.selected = self.zoom_root_index().unwrap_or(0);
        self.cursor_col = 0;
        self.status = "zoomed out".into();
    }

    /// Flat DFS index of the current zoom root, or `None` when the whole
    /// page is shown (stack empty) or the stored path no longer resolves
    /// (block moved / deleted since zoom — the caller falls back to the
    /// whole page).
    pub(crate) fn zoom_root_index(&self) -> Option<usize> {
        let path = self.zoom_stack.last()?;
        index_for_path(&self.page.blocks, path)
    }

    /// The current zoom root node and its whole-page flat index, or
    /// `None` when the whole page is shown (stack empty) or the stored
    /// path no longer resolves. The render walk uses this to draw only
    /// the focused subtree while keeping `cursor` on whole-page indices.
    pub(crate) fn zoom_root_node(&self) -> Option<(&outl_md::parse::OutlineNode, usize)> {
        let path = self.zoom_stack.last()?;
        let node = node_at_path(&self.page.blocks, path)?;
        let index = index_for_path(&self.page.blocks, path)?;
        Some((node, index))
    }

    /// The half-open `[start, end)` flat-index window the zoom currently
    /// confines the view to: the root block plus every descendant. When
    /// no zoom is active (or the root path is stale) it spans the whole
    /// page so callers can use it unconditionally.
    pub(crate) fn zoom_root_window(&self) -> (usize, usize) {
        match self.zoom_stack.last() {
            Some(path) => match (
                index_for_path(&self.page.blocks, path),
                node_at_path(&self.page.blocks, path),
            ) {
                (Some(start), Some(node)) => (start, start + 1 + flat_count(&node.children)),
                // Stale path: behave as if not zoomed.
                _ => (0, self.flat_len),
            },
            None => (0, self.flat_len),
        }
    }

    /// Breadcrumb of the ancestors leading to the current zoom root,
    /// outermost first, ending with the root block itself. Empty when no
    /// zoom is active. Each entry is the block's (trimmed) first line of
    /// text, so the header can render `page ▸ parent ▸ block`.
    pub(crate) fn zoom_breadcrumb(&self) -> Vec<String> {
        let Some(path) = self.zoom_stack.last() else {
            return Vec::new();
        };
        let mut crumbs = Vec::with_capacity(path.len());
        for depth in 1..=path.len() {
            if let Some(node) = node_at_path(&self.page.blocks, &path[..depth]) {
                crumbs.push(crumb_label(&node.text));
            }
        }
        crumbs
    }
}

/// One breadcrumb label: the first line of the block's text, trimmed,
/// clipped so a long block doesn't blow out the header. Empty blocks
/// render as a placeholder so the crumb is still clickable-looking.
fn crumb_label(text: &str) -> String {
    const MAX: usize = 24;
    let first = text.lines().next().unwrap_or("").trim();
    if first.is_empty() {
        return "·".to_string();
    }
    // Clip to MAX chars without allocating a `Vec<char>`: the byte
    // offset of the (MAX+1)-th char is where to cut. This runs on every
    // header render while zoomed, so the extra allocation is worth
    // avoiding. `None` means the string is already <= MAX chars.
    match first.char_indices().nth(MAX) {
        Some((byte_idx, _)) => format!("{}…", &first[..byte_idx]),
        None => first.to_string(),
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

    /// Seed a fixed nested page:
    ///   0 a
    ///   1   a1
    ///   2   a2
    ///   3 b
    ///   4   b1
    fn seed_nested(app: &mut App) {
        app.page = outl_md::parse::parse("- a\n  - a1\n  - a2\n- b\n  - b1\n");
        app.flat_len = outl_md::outline_ops::flat_count(&app.page.blocks);
        app.selected = 0;
    }

    #[test]
    fn zoom_in_pushes_path_and_moves_selection_to_root() {
        let (mut app, _dir) = fresh_app();
        seed_nested(&mut app);
        app.selected = 3; // block `b`
        app.zoom_in_on_selected();
        assert_eq!(app.zoom_stack, vec![vec![1]], "b is the second top-level");
        assert_eq!(app.selected, 3, "selection lands on the zoom root");
        assert_eq!(app.zoom_root_window(), (3, 5), "b + its one child b1");
    }

    #[test]
    fn zoom_in_on_leaf_is_allowed() {
        let (mut app, _dir) = fresh_app();
        seed_nested(&mut app);
        app.selected = 1; // leaf a1
        app.zoom_in_on_selected();
        assert_eq!(app.zoom_stack, vec![vec![0, 0]]);
        assert_eq!(app.zoom_root_window(), (1, 2), "just the leaf itself");
    }

    #[test]
    fn zoom_out_pops_back_to_whole_page() {
        let (mut app, _dir) = fresh_app();
        seed_nested(&mut app);
        app.selected = 0; // block a
        app.zoom_in_on_selected();
        assert!(!app.zoom_stack.is_empty());
        app.zoom_out();
        assert!(app.zoom_stack.is_empty(), "back to whole page");
        assert_eq!(app.zoom_root_window(), (0, 5), "window spans the page");
    }

    #[test]
    fn zoom_out_at_page_root_is_noop() {
        let (mut app, _dir) = fresh_app();
        seed_nested(&mut app);
        app.zoom_out();
        assert!(app.zoom_stack.is_empty());
    }

    #[test]
    fn navigation_is_confined_to_the_zoomed_subtree() {
        let (mut app, _dir) = fresh_app();
        seed_nested(&mut app);
        app.selected = 0; // zoom into `a` (children a1, a2)
        app.zoom_in_on_selected();
        // Forward from the root walks a → a1 → a2 and then stops: `b`
        // (idx 3) is outside the window.
        app.move_selection(1);
        assert_eq!(app.selected, 1, "a1");
        app.move_selection(1);
        assert_eq!(app.selected, 2, "a2");
        app.move_selection(1);
        assert_eq!(app.selected, 2, "clamped at the subtree bottom");
        // Backward stops at the root, never climbing above `a`.
        app.move_selection(-5);
        assert_eq!(app.selected, 0, "clamped at the zoom root");
    }

    #[test]
    fn zoom_root_node_returns_focused_subtree() {
        let (mut app, _dir) = fresh_app();
        seed_nested(&mut app);
        app.selected = 0;
        app.zoom_in_on_selected();
        let (node, index) = app.zoom_root_node().expect("zoomed");
        assert_eq!(node.text, "a");
        assert_eq!(index, 0);
    }

    #[test]
    fn breadcrumb_walks_ancestors_down_to_root() {
        let (mut app, _dir) = fresh_app();
        seed_nested(&mut app);
        app.selected = 2; // a2, nested under a
        app.zoom_in_on_selected();
        assert_eq!(
            app.zoom_breadcrumb(),
            vec!["a".to_string(), "a2".to_string()]
        );
    }

    #[test]
    fn zoom_resets_on_view_change() {
        let (mut app, _dir) = fresh_app();
        seed_nested(&mut app);
        app.selected = 0;
        app.zoom_in_on_selected();
        assert!(!app.zoom_stack.is_empty());
        // Any view switch reloads the page and must clear the zoom.
        app.go_today().unwrap();
        assert!(
            app.zoom_stack.is_empty(),
            "view change resets zoom to whole page"
        );
    }

    #[test]
    fn zoom_in_twice_on_same_block_does_not_double_push() {
        let (mut app, _dir) = fresh_app();
        seed_nested(&mut app);
        app.selected = 0;
        app.zoom_in_on_selected();
        app.zoom_in_on_selected();
        assert_eq!(app.zoom_stack.len(), 1, "no duplicate push on same root");
    }

    #[test]
    fn stale_zoom_path_falls_back_to_whole_page_window() {
        let (mut app, _dir) = fresh_app();
        seed_nested(&mut app);
        // Push a path that doesn't resolve (page has no 9th top-level).
        app.zoom_stack.push(vec![9]);
        assert_eq!(
            app.zoom_root_window(),
            (0, app.flat_len),
            "unresolvable root behaves as whole page"
        );
        assert!(app.zoom_root_node().is_none());
    }
}
