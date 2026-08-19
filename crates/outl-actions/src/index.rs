//! Derive the workspace index from the **op log**, not from `.md`.
//!
//! [`outl_md::WorkspaceIndex::build`] answers "what is in this
//! workspace" by walking `pages/` + `journals/`, parsing every `.md`
//! with comrak and reading every `.outl` sidecar to recover the ids.
//! Everything it produces already exists in the tree after the op-log
//! replay every caller has already paid for, so that walk is a second,
//! slower derivation of state we hold in memory — and it derives it
//! from the *projection* rather than the source of truth (root
//! `CLAUDE.md` invariant 1).
//!
//! [`derive()`] is the replacement: one pass over the tree, no
//! filesystem access, no markdown parse, no JSON.
//!
//! ## Why this lives in `outl-actions` and not in `outl-md`
//!
//! The issue that proposed this ([#81]) put it on `WorkspaceIndex`
//! itself. It cannot go there: `outl-md` does not depend on
//! `outl-actions` (the arrow points the other way), so a `derive` in
//! `outl-md` would have to re-implement `page::page_meta`,
//! `tree::children_of` and the tree → AST projection that
//! `outl-actions` already owns — three parallel implementations of
//! logic whose whole value is having one owner
//! ([Reuse-first](../../../docs/contributing.md)).
//!
//! So the split is: the **type** stays in `outl-md` (every client
//! already consumes `WorkspaceIndex`), the **tree-side constructor**
//! lives here. That is the same shape
//! [`crate::backlinks_index::build_backlink_index`] (workspace) and
//! `build_backlink_index_from_disk` (disk) already have.
//!
//! ## What is NOT equivalent to the disk build
//!
//! Two differences, both deliberate:
//!
//! 1. **A `.md` holding content that exists in no op is invisible
//!    here.** That is the state root `CLAUDE.md` invariant 8 exists to
//!    describe, and the honest answer for an index that claims to
//!    reflect the op log. `outl reconcile --ahead-of-log` is the route
//!    that turns such content into ops; until it runs, this index does
//!    not see it and must not pretend to.
//! 2. **A block whose sidecar disagrees with the `.md` is indexed
//!    here, and dropped by the disk path.** `BlockIndex::walk_blocks`
//!    pairs the AST with `sidecar_blocks[cursor]` positionally and
//!    skips on `content_hash` mismatch, so one block typed before
//!    reconcile ran desynchronises the cursor and silently removes the
//!    rest of the page from the index. Nothing here can skip: the id
//!    arrives on the node.
//!
//! ## Who may call this, and who may not
//!
//! **Not a client's index-rebuild path.** [`derive()`] reads
//! `Workspace::block_text` for every node, which forces a lazy-boot
//! vault ([#179]) to materialize in full and holds the workspace lock
//! for the whole walk. That pair is the named "opening the journal /
//! pressing Esc freezes" regression, and it is exactly why
//! [`crate::backlinks_index::build_backlink_index_from_disk`] exists
//! alongside the from-workspace builder.
//!
//! So this is for **short-lived, one-shot readers** that already
//! replayed the log, hold no UI, and exit right after. Today that is
//! exactly three callers: `outl search`, `outl backlinks`, and the MCP
//! session's index cache.
//!
//! **Every GUI path uses the disk build instead**, and that is a
//! decision, not an oversight. The TUI's auto-run loop passes the index
//! it already rebuilds on a background thread; the desktop / mobile
//! auto-run sweep and `crate::exec::run_code_block` call
//! `WorkspaceIndex::build`. All three run while their client holds the
//! workspace mutex, so the correctness below is not worth a stall on
//! the boot path.
//!
//! ## What this does NOT buy you
//!
//! Measured on a real 2,835-page / 67k-node / 216k-op workspace, warm
//! cache: `outl search` went 650 ms → 655 ms. **The derivation is not
//! faster.** It removes 65 MB of file reads and all markdown parsing,
//! and spends the savings materializing block text out of the CRDT.
//!
//! What it does buy is the correctness in the section above, an end to
//! deriving the index from the projection rather than the source of
//! truth, and no filesystem dependency for a caller that has none.
//! Anyone reopening [#81] on performance grounds should re-measure
//! first — the original issue's premise did not survive contact with
//! this code base.
//!
//! [#81]: https://github.com/outlmd/outl/issues/81
//! [#179]: https://github.com/outlmd/outl/issues/179

use std::collections::HashMap;
use std::path::Path;

use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_md::block_index::IdentifiedNode;
use outl_md::index::{PageEntry, WorkspaceIndex};

use crate::backlinks_index::build_children_index;
use crate::journal::page_md_path;
use crate::outline::ChildrenIndex;
use crate::page::{canonical_root_for_slug, page_meta, PageKind, PageMeta};
use crate::tree::renderable_prop_value;

/// `node → its renderable properties`, built in one scan.
///
/// The exact counterpart of [`ChildrenIndex`], and load-bearing for the
/// same reason: [`outl_core::tree::Tree::properties_of`] filters the
/// workspace-wide property map per call, so asking it once per node is
/// `O(nodes × properties)`. On a 67k-node / 216k-op workspace that
/// measured **2.4× slower than the disk walk this module replaces** —
/// the derivation was correct and useless. Build the map once.
type PropertiesIndex = HashMap<NodeId, Vec<(String, String)>>;

/// Group every property in the tree by the node carrying it.
///
/// Values are already filtered and sorted the way a block wants them,
/// so the per-node path is a map lookup and a clone.
fn build_properties_index(workspace: &Workspace) -> PropertiesIndex {
    let mut grouped: PropertiesIndex = HashMap::new();
    for (node, key, value) in workspace.tree().iter_properties() {
        // No page-model filtering: this map is only ever read for
        // blocks, and on a block `page-slug` / `page-kind` are ordinary
        // user properties the dialect accepts. Matches
        // `journal::render::block_properties` so a block's properties
        // read the same through the index as through the renderer.
        let Some(text) = renderable_prop_value(value) else {
            continue;
        };
        grouped
            .entry(node)
            .or_default()
            .push((key.to_string(), text));
    }
    for props in grouped.values_mut() {
        props.sort_by(|a, b| a.0.cmp(&b.0));
    }
    grouped
}

/// Build a [`WorkspaceIndex`] from the tree, with no filesystem
/// access.
///
/// `root` is used only to compute each page's `.md` path for
/// [`PageEntry::path`] / [`outl_md::BlockEntry::source_path`] — the
/// file is never opened.
///
/// Runs in two passes over the pages, mirroring the disk build: every
/// block is registered before any reverse `((blk-XXXXXX))` edge is
/// resolved, so a page citing a block on a page visited later still
/// records its edge.
///
/// Cost is `O(nodes)`: one scan to build the `parent → children` map,
/// then one DFS per page. Going through [`crate::tree::children_of`]
/// instead would make it `O(nodes²)` — on a 67k-node workspace that is
/// not a constant-factor difference, so the [`ChildrenIndex`] is
/// load-bearing, not an optimisation.
pub fn derive(workspace: &Workspace, root: &Path) -> WorkspaceIndex {
    let children = build_children_index(workspace);
    let properties = build_properties_index(workspace);
    let mut idx = WorkspaceIndex::default();

    for (page_root, meta) in canonical_pages(workspace, &children) {
        let path = page_md_path(root, &meta);
        let blocks = project_identified(workspace, page_root, &children, &properties);

        idx.insert_page(page_entry(&meta, &path));
        idx.collect_page_blocks_from_tree(&meta.slug, &path, &blocks);
    }

    // Reverse refs last, once every handle in the workspace is known —
    // a page can cite a block on a page walked after it. Reads the
    // blocks already in the index, so the projected forests above are
    // dropped as we go instead of being held for a second pass.
    idx.collect_refs_from_indexed();

    idx
}

/// One page root per slug, with its metadata.
///
/// `page_meta` is the owner of "is this node a page" — a root child
/// without a `page-slug` (the trash root, a pre-page-model stray) reads
/// back as `None` and never reaches the projection.
///
/// The **grouping** matters and is not defensive coding. Split-brain
/// leaves more than one live root carrying one slug until
/// `merge_duplicate_slug_roots` repairs it, and this index is keyed by
/// slug in three places at once: `PageEntry`, the per-page block list,
/// and `(slug, dfs_path) -> NodeId`. Indexing both roots would let the
/// last one win the page entry while their blocks merged under one slug
/// and their DFS paths overwrote each other, so `block_at_location`
/// could hand back a block from the root the page metadata does not
/// describe. `page::canonical_root_for_slug` owns which root wins, and
/// this asks it rather than keeping a second copy of that rule.
///
/// The losing root's blocks are left out of the index entirely. They
/// are still in the op log and the merge repair re-parents them under
/// the winner, which is when they come back.
fn canonical_pages(workspace: &Workspace, children: &ChildrenIndex) -> Vec<(NodeId, PageMeta)> {
    let mut by_slug: HashMap<String, Vec<(NodeId, PageMeta)>> = HashMap::new();
    for &id in children.get(&NodeId::root()).into_iter().flatten() {
        if let Some(meta) = page_meta(workspace, id) {
            by_slug
                .entry(meta.slug.clone())
                .or_default()
                .push((id, meta));
        }
    }

    let mut pages: Vec<(NodeId, PageMeta)> = by_slug
        .into_iter()
        .filter_map(|(slug, mut roots)| {
            if roots.len() == 1 {
                return roots.pop();
            }
            let winner = canonical_root_for_slug(&slug, roots.iter().map(|(id, _)| *id))?;
            roots.into_iter().find(|(id, _)| *id == winner)
        })
        .collect();
    // `HashMap` iteration is unordered and this index is rebuilt often;
    // sort so two runs over one tree produce the same handle assignment
    // when a `blk-` collision has to pick a loser to expand.
    pages.sort_by(|a, b| a.1.slug.cmp(&b.1.slug));
    pages
}

/// Translate a [`PageMeta`] into the index's [`PageEntry`].
///
/// The two structs describe the same four page-level facts (title,
/// icon, pinned, type) because one was written against the tree and
/// the other against the `.md`. This function is the seam where that
/// duplication is paid off; collapsing them into one type is tracked
/// separately so this change stays reviewable.
fn page_entry(meta: &PageMeta, path: &Path) -> PageEntry {
    PageEntry {
        path: path.to_path_buf(),
        slug: meta.slug.clone(),
        title: meta.title.clone(),
        icon: meta.icon.clone(),
        is_journal: matches!(meta.kind, PageKind::Journal),
        pinned: meta.pinned,
        page_type: meta.page_type.clone(),
    }
}

/// Project the subtree below `parent` into the id-carrying shape the
/// block index consumes.
///
/// Mirrors `crate::journal::render::build_outline` (which produces the
/// same forest for the renderer, minus the ids) but resolves children
/// through `index` so a full-workspace walk stays linear.
///
/// Private: its signature names `PropertiesIndex`, which no caller
/// outside this module can build, so a `pub` here would advertise an
/// API nobody can reach.
fn project_identified(
    workspace: &Workspace,
    parent: NodeId,
    index: &ChildrenIndex,
    properties: &PropertiesIndex,
) -> Vec<IdentifiedNode> {
    index
        .get(&parent)
        .map(|kids| {
            kids.iter()
                .map(|&child| IdentifiedNode {
                    id: child,
                    text: workspace.block_text(child).unwrap_or_default(),
                    properties: properties.get(&child).cloned().unwrap_or_default(),
                    children: project_identified(workspace, child, index, properties),
                })
                .collect()
        })
        .unwrap_or_default()
}
