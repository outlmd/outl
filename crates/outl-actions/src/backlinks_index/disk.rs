//! The from-disk builder: the one every client runs.
//!
//! Split out of `backlinks_index.rs`, which owns the index itself. This
//! half reads each page's `.md`, walks the parsed outline and feeds
//! `BacklinkIndex::insert`; as a child module it reaches the index's
//! private fields without widening them.

use std::path::Path;
use std::sync::Arc;

use tracing::warn;

use super::BacklinkIndex;
use crate::alias_conflicts::PageClaims;
use crate::backlinks::Backlink;
use crate::backlinks_crumbs::{crumbs, CrumbFrame};
use crate::backlinks_keys::mentions_of;
use crate::journal::page_md_path;
use crate::outline::{read_page_outline, OutlineNode};
use crate::outline_subtree::{clone_capped, SUBTREE_MAX_DEPTH};
use crate::page::PageMeta;

/// Build the inverted index by reading each page's `.md` from disk,
/// **without touching the `Workspace`**.
///
/// This is the client-facing builder. Reading block text through
/// `Workspace::block_text` (what [`super::build_backlink_index`] does) forces
/// the whole workspace to materialize on a lazy-boot vault (#179) and
/// holds the workspace lock across the `O(blocks)` walk — together, the
/// "opening the journal / pressing Esc freezes" bug. The `.md` files are
/// the projection the user already sees on disk and carry every block's
/// text + properties + sidecar id, so the index can be built from them
/// on a background thread with no lock and no Yrs materialization.
///
/// `metas` is the page list (cheap to get via `list_pages`, which reads
/// only page roots, not block text); pass it in so this function needs
/// no `Workspace` at all and stays `Send` for a worker thread.
pub fn build_backlink_index_from_disk(metas: &[PageMeta], root: &Path) -> BacklinkIndex {
    let started = std::time::Instant::now();
    let mut index = BacklinkIndex {
        claims: PageClaims::from_metas(metas),
        ..BacklinkIndex::default()
    };
    for meta in metas {
        let outline = match read_page_outline(root, meta) {
            Ok(o) => o,
            Err(e) => {
                // Skipping a page here means every `[[ref]]` it makes
                // disappears from the backlinks panel. A missing `.md`
                // is silent by design (nothing to index); anything else
                // is an unreadable page and the user deserves to know
                // why their links went quiet.
                warn!("backlink index: skipping page `{}`: {e}", meta.slug);
                continue;
            }
        };
        let source_path = page_md_path(root, meta);
        let mut path: Vec<usize> = Vec::new();
        let mut ancestors: Vec<CrumbFrame<'_>> = Vec::new();
        walk_parsed(
            &outline.nodes,
            meta,
            &source_path,
            &mut path,
            &mut ancestors,
            &mut index,
        );
    }
    tracing::debug!(
        pages = metas.len(),
        refs = index.len(),
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        "build_backlink_index_from_disk (full workspace rescan)"
    );
    index
}

/// DFS the parsed `.md` outline of one page, adding every mentioning
/// block to `index`. The from-disk twin of [`walk_page`]: same shape,
/// but reads the leaf + `from-template::` value off the already-parsed
/// [`OutlineNode`] instead of the workspace tree.
pub(super) fn walk_parsed<'a>(
    nodes: &'a [OutlineNode],
    meta: &PageMeta,
    source_path: &Path,
    path: &mut Vec<usize>,
    ancestors: &mut Vec<CrumbFrame<'a>>,
    index: &mut BacklinkIndex,
) {
    for (idx, node) in nodes.iter().enumerate() {
        path.push(idx);
        let from_template = node
            .properties
            .iter()
            .find(|(k, _)| k == crate::template::FROM_TEMPLATE_KEY)
            .map(|(_, v)| v.as_str());
        let keys = mentions_of(&node.text, from_template);
        if !keys.is_empty() {
            index.insert(
                Backlink {
                    block_id: node.id.clone(),
                    block_text: node.text.clone(),
                    todo: node.todo,
                    source_page: Some(meta.clone()),
                    source_block: shallow_parsed(node),
                    source_block_path: path.clone(),
                    ancestors: crumbs(ancestors),
                    source_path: Some(source_path.to_path_buf()),
                },
                keys,
                capped_subtree(node),
            );
        }
        ancestors.push(CrumbFrame::Parsed(node));
        walk_parsed(&node.children, meta, source_path, path, ancestors, index);
        ancestors.pop();
        path.pop();
    }
}

/// The side-table entry for one citing block: its children copied down to
/// [`crate::SUBTREE_MAX_DEPTH`] levels, `None` for a leaf.
///
/// The parse already holds the whole tree (and tokenized it) by the time
/// the walk gets here, so this costs a clone and the memory the index
/// then keeps — no parse, no tokenize, no lock. That is the difference
/// from what #179 removed, which projected the subtree out of the
/// `Workspace`.
fn capped_subtree(node: &OutlineNode) -> Option<Arc<[OutlineNode]>> {
    if node.children.is_empty() {
        return None;
    }
    Some(Arc::from(clone_capped(&node.children, SUBTREE_MAX_DEPTH)))
}

/// Copy a parsed outline node as a shallow leaf (no `children`), keeping
/// the already-parsed tokens/properties. Mirrors
/// `project_outline_node_shallow` for the from-disk path.
fn shallow_parsed(node: &OutlineNode) -> OutlineNode {
    OutlineNode {
        id: node.id.clone(),
        text: node.text.clone(),
        todo: node.todo,
        collapsed: node.collapsed,
        properties: node.properties.clone(),
        tokens: node.tokens.clone(),
        table: node.table.clone(),
        children: Vec::new(),
    }
}
