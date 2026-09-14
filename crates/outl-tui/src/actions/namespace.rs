//! Page namespaces — which pages live under the open one.
//!
//! The counterpart to `view::namespace`, which draws the list this
//! produces. It stays out of `nav` because it is not navigation: the
//! TUI has no cursor in the nested-pages section (recorded as
//! `Capability::NestedPages` → `Partial`), so nothing here moves a
//! selection or opens a page — it answers one read-only question.
//!
//! The hierarchy itself is `outl_actions::namespace`'s, not this
//! file's: a namespace is read off the page **title**, per slugified
//! segment, and three clients splitting titles on `/` themselves is
//! exactly the drift that module exists to prevent.

use crate::state::App;

impl App {
    /// Pages nested under the current page's namespace — `os` holds
    /// `os/linux` and `os/linux/debian` (issue #275).
    ///
    /// **Memoised**, in `App::namespace_children`. It materializes no
    /// block text (page roots carry none), but it does pay two
    /// full-workspace node scans — `find_by_slug` and `list_pages` each
    /// drive `tree::children_of`, which is one `iter_nodes` pass per
    /// call — plus a `page_meta` per page. The render path runs every
    /// `terminal.draw`, so uncached that was a per-frame cost on a
    /// 64k-node workspace, not a per-navigation one. "It doesn't touch
    /// `block_text`" was the wrong defence: the scans are the cost.
    ///
    /// The slug is the key, but the rows also depend on this page's
    /// title and on the whole page list, so navigation alone is not
    /// enough to keep them honest. Every workspace change that touches
    /// either — a local commit (`reindex_backlinks_for_slug`) or a
    /// whole-workspace swap (`spawn_backlink_index_rebuild`: peer
    /// reload, orphan reconcile, plugin sweep, page delete) — calls
    /// [`Self::invalidate_namespace_children`] so the next frame
    /// re-derives.
    pub(crate) fn namespace_children_for_current(&self) -> Vec<outl_actions::NamespaceChild> {
        let slug = self.current_slug();
        if let Some((cached_slug, rows)) = self.namespace_children.borrow().as_ref() {
            if *cached_slug == slug {
                return rows.clone();
            }
        }
        let rows = self.derive_namespace_children(&slug);
        *self.namespace_children.borrow_mut() = Some((slug, rows.clone()));
        rows
    }

    /// Drop the memoised rows so the next render re-derives them.
    /// Called from the two places every workspace change already
    /// funnels through (see the memo's doc), never from the render
    /// path — an invalidation per frame would undo the memo.
    pub(crate) fn invalidate_namespace_children(&self) {
        *self.namespace_children.borrow_mut() = None;
    }

    /// The uncached derivation. Split out so the memo above reads as
    /// one decision, and so a caller that genuinely wants fresh rows
    /// has somewhere to go.
    fn derive_namespace_children(&self, slug: &str) -> Vec<outl_actions::NamespaceChild> {
        let Some(id) = outl_actions::find_by_slug(&self.workspace, slug) else {
            return Vec::new();
        };
        let Some(meta) = outl_actions::page_meta(&self.workspace, id) else {
            return Vec::new();
        };
        let pages = outl_actions::list_pages(&self.workspace);
        outl_actions::namespace_descendants(&pages, &meta.title)
    }
}
