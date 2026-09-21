//! The shapes the block index stores and hands back.
//!
//! Split out of [`super`] so the maps and their population paths are
//! not read through a hundred lines of field documentation. Nothing
//! here holds state or knows about the index — these are the data
//! types, and the module doc for what the index *does* stays next
//! door.

use crate::parse::OutlineNode;
use outl_core::id::NodeId;
use std::path::PathBuf;

/// One indexed block. Carries enough context that
/// `WorkspaceIndex::resolve_block_ref` (see `crate::index`) can return
/// it directly — no follow-up disk read needed for the common path.
///
/// `children` is a clone of the block's subtree (same shape
/// `outl_actions::Backlink::source_block` carries for backlinks).
/// The cost is bounded: one clone per indexed block, not one per
/// reference. For an embed surface, the consumer renders `text` +
/// `children` exactly as the source page would.
#[derive(Debug, Clone)]
pub struct BlockEntry {
    /// Block's stable ULID.
    pub id: NodeId,
    /// Short ref handle (`blk-XXXXXX`). May be 7+ characters when a
    /// collision forced lazy expansion at index time.
    pub ref_handle: String,
    /// Slug of the page hosting the block.
    pub source_slug: String,
    /// Filesystem path of the hosting `.md`.
    pub source_path: PathBuf,
    /// DFS path inside the source page's AST.
    pub source_block_path: Vec<usize>,
    /// Block text at index time. Used as the inline-resolved text
    /// when a `((blk-XXXXXX))` is rendered.
    pub text: String,
    /// Lowercased copy of `text`. Cached so
    /// [`BlockIndex::search_text`](super::BlockIndex::search_text) doesn't reallocate per block on
    /// every autocomplete keystroke.
    pub text_fold: String,
    /// Block properties (`key:: value`), in document order.
    ///
    /// Both population paths already carry them — the disk path off
    /// [`OutlineNode::properties`], the tree path off
    /// [`IdentifiedNode::properties`] — so this is a copy, not a
    /// second parse. Keys and values are stored **lowercased**: the
    /// only consumer is the ` ```query ` DSL's `prop:` / `not-prop:`
    /// filters, which match case-insensitively like every other
    /// directive, and folding once at index time keeps the filter
    /// allocation-free per block.
    pub properties: Vec<(String, String)>,
    /// Cloned subtree under this block — used by embed surfaces.
    pub children: Vec<OutlineNode>,
}

/// Lowercase a property list once, at index time.
pub(super) fn fold_properties(props: &[(String, String)]) -> Vec<(String, String)> {
    props
        .iter()
        .map(|(k, v)| (k.to_lowercase(), v.to_lowercase()))
        .collect()
}

/// A block projected straight from the op-log tree: the outline shape
/// a renderer needs, plus the stable id that the disk path has to go
/// to the sidecar for.
///
/// This is the input type of the tree-side population path
/// ([`BlockIndex::collect_page_blocks_from_tree`](super::BlockIndex::collect_page_blocks_from_tree)).
/// It exists because
/// [`OutlineNode`] deliberately carries no id — it is the shape of a
/// *parsed `.md`*, where ids live in the sidecar and nowhere else
/// (root `CLAUDE.md` invariant 2). A projection of the tree has the
/// opposite problem: the id is the one thing it is certain of.
///
/// Producers live in `outl-actions`, which owns the tree walk
/// (`outl_actions::index::project_identified`). This crate only
/// consumes the shape, so nothing here needs a `Workspace` — the
/// dependency arrow keeps pointing the one way it always has.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IdentifiedNode {
    /// Stable id of the block, straight from the tree.
    pub id: NodeId,
    /// Block content, same convention as [`OutlineNode::text`]
    /// (markdown inline, no `- ` prefix, no property lines).
    pub text: String,
    /// Properties attached to this block.
    pub properties: Vec<(String, String)>,
    /// Children, depth-first — same order the `.md` renders them in.
    pub children: Vec<IdentifiedNode>,
}

impl IdentifiedNode {
    /// Drop the ids, yielding the plain AST shape that
    /// [`BlockEntry::children`] and the renderer both take.
    ///
    /// Recursive, and it clones: one clone per indexed block, which is
    /// the same bound the disk path already pays (`b.children.clone()`
    /// in `walk_blocks`).
    pub fn to_outline(&self) -> OutlineNode {
        OutlineNode {
            text: self.text.clone(),
            properties: self.properties.clone(),
            children: self.children.iter().map(Self::to_outline).collect(),
        }
    }
}

/// One reverse edge: somebody cites the block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockReference {
    /// Slug of the citing page.
    pub source_slug: String,
    /// DFS path of the citing block inside its page's AST.
    pub source_block_path: Vec<usize>,
}
