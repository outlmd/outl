//! Tree → markdown projection (page and single-block forms).

use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_md::parse::{OutlineNode, ParsedPage};
use outl_md::render::render;

use crate::outline::ChildrenIndex;
use crate::page::SLUG_KEY;
use crate::tree::{
    group_properties_of, is_page_model_key, renderable_prop_value, PropertiesByNode,
};

/// The subtree as walked, before properties are attached.
///
/// [`OutlineNode`] carries no id, and the property scan needs the whole
/// id set before it can run — so the walk produces this, the scan runs
/// once, and [`build_outline`] turns it into the outline.
struct Skeleton {
    id: NodeId,
    text: String,
    children: Vec<Skeleton>,
}

/// Walk `parent`'s subtree in render order, materialising block text but
/// no properties.
///
/// Children come from `children` rather than `crate::tree::children_of`,
/// which rescans every node in the workspace per call — one scan per
/// block rendered. Same trade as `subtree_properties` below, in the
/// other dimension: one pre-built map instead of a per-node filter.
fn walk_subtree(workspace: &Workspace, parent: NodeId, children: &ChildrenIndex) -> Vec<Skeleton> {
    children
        .get(&parent)
        .map(|kids| {
            kids.iter()
                .map(|&id| Skeleton {
                    id,
                    text: workspace.block_text(id).unwrap_or_default(),
                    children: walk_subtree(workspace, id, children),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn collect_ids(skeletons: &[Skeleton], out: &mut Vec<NodeId>) {
    for node in skeletons {
        out.push(node.id);
        collect_ids(&node.children, out);
    }
}

/// Turn the walked skeleton into outline nodes, attaching each block's
/// properties.
///
/// **No page-model filtering here, deliberately.** `page-slug` /
/// `page-kind` are book-keeping *on a page root*; on an ordinary block
/// they are whatever the user typed. The dialect has no allow-list of
/// property keys, so `parse_property_line` accepts them and `diff_to_ops`
/// emits a `SetProp` like any other. Dropping them on render would put
/// the value in the tree and nowhere on disk, and the next external-edit
/// reconcile would emit the removal — convergent data loss.
///
/// Rendering block properties used to be skipped entirely
/// (`properties: Vec::new()`), so any op → `.md` re-render silently
/// deleted the property lines from disk and the next reconcile emitted
/// prop-removal ops: the same loss, surfaced by the importer's resolve
/// pass.
fn build_outline(skeletons: Vec<Skeleton>, properties: &PropertiesByNode) -> Vec<OutlineNode> {
    skeletons
        .into_iter()
        .map(|node| OutlineNode {
            text: node.text,
            properties: properties.get(&node.id).cloned().unwrap_or_default(),
            children: build_outline(node.children, properties),
        })
        .collect()
}

/// Project `parent`'s subtree, returning `parent`'s own properties
/// alongside its rendered children — both off a single property scan
/// and one `parent -> children` map.
fn project_subtree(
    workspace: &Workspace,
    parent: NodeId,
    children: &ChildrenIndex,
) -> (Vec<(String, String)>, Vec<OutlineNode>) {
    let skeleton = walk_subtree(workspace, parent, children);
    let mut ids = Vec::with_capacity(16);
    ids.push(parent);
    collect_ids(&skeleton, &mut ids);
    ids.sort_unstable();

    // One pass over the property map for the whole page, never one per
    // block — `group_properties_of` owns why, and what the per-node
    // version costs.
    let properties = group_properties_of(workspace, &ids, renderable_prop_value);
    let own = properties.get(&parent).cloned().unwrap_or_default();
    (own, build_outline(skeleton, &properties))
}

/// Render every block under `page_root` to a clean `.md` string,
/// **including** the page-level properties stored on the page node
/// (`title::`, `icon::`, `pinned::`, `type::`, `role::`, anything
/// custom). The page's title (`workspace.block_text(page_root)`) is
/// **not** included in the body — clients can prepend it themselves
/// if they want.
///
/// The page model's book-keeping ([`crate::tree::is_page_model_key`]) is
/// skipped: `outl_actions::page` owns those keys through its own ops, and
/// surfacing them here would re-write the slug on every reconcile (a
/// no-op via the CRDT, but noise on disk).
///
/// One of them, `page-frontmatter`, does belong in the file — as a `---`
/// fence, not a `key:: value` line — so it is **lifted into
/// `ParsedPage::frontmatter` before that filter runs**. See
/// `render_page_md_with`.
///
/// Sort order is alphabetical on the key — see `subtree_properties`.
pub fn render_page_md(workspace: &Workspace, page_root: NodeId) -> String {
    render_page_md_with(
        workspace,
        page_root,
        &crate::tree::children_index(workspace, page_root),
    )
}

/// [`render_page_md`] against a caller-supplied `parent -> children`
/// map, for a pass that renders **many** pages.
///
/// The map may be scoped to this page or cover the whole workspace —
/// the walk only ever reads entries inside `page_root`'s subtree, so
/// both answer identically. Which one to hand it is the amortization
/// question, and it has a measured answer: a whole-workspace map costs
/// ~5 ms at 64k nodes against ~0.5 ms for a scoped one, so it pays only
/// when one map serves every page. [`render_page_md`] scopes; a sweep
/// over `crate::page::list_all` should build
/// `crate::backlinks_index::build_children_index` once and pass it here.
pub(crate) fn render_page_md_with(
    workspace: &Workspace,
    page_root: NodeId,
    children: &ChildrenIndex,
) -> String {
    let (mut properties, blocks) = project_subtree(workspace, page_root, children);

    // A page whose `.md` opened with a YAML frontmatter fence carries it in
    // the op log under `PAGE_FRONTMATTER_KEY` (`outl_md::parse` refuses to
    // read it as outline, issue #281). Lift it out of the property list and
    // into the field the renderer writes the `---` delimiters from.
    //
    // **Before the `is_page_model_key` filter, which hides this key too.**
    // Filtering first deletes the fence from every projection, which is the
    // exact write the issue reported.
    let frontmatter = properties
        .iter()
        .position(|(k, _)| k == outl_md::PAGE_FRONTMATTER_KEY)
        .map(|at| properties.remove(at).1);
    properties.retain(|(k, _)| !is_page_model_key(k));

    let page = ParsedPage {
        frontmatter,
        properties,
        blocks,
        warnings: Vec::new(),
    };
    render(&page)
}

/// Render the block `node` and its subtree to clean outl markdown as
/// a single top-level bullet (with its descendants nested under it).
///
/// This is the "copy block" projection: the desktop's `Cmd+C` in view
/// mode hands the result to the clipboard, and the matching paste
/// re-ingests it through the same `paste_markdown` pipeline external
/// clipboard text uses — so a copy duplicates the subtree with fresh
/// ids. Reuses the exact projection [`render_page_md`] writes to disk,
/// so a copied block reads identically to how it lives in the `.md`.
pub fn render_block_md(workspace: &Workspace, node: NodeId) -> String {
    render_fragment(project_blocks(workspace, &[node]))
}

/// Render already-projected blocks as a fragment: top-level bullets, no
/// page properties and no frontmatter fence.
pub(crate) fn render_fragment(blocks: Vec<OutlineNode>) -> String {
    // A block projection is a fragment, not a file: frontmatter is a
    // page-level fact and there is no page here to own one.
    render(&ParsedPage {
        blocks,
        ..Default::default()
    })
}

/// Project each of `roots` **including the root itself** — text,
/// properties and its whole subtree — as one top-level outline node,
/// in the caller's order.
///
/// The projection [`render_block_md`] and `crate::copy_markdown` share,
/// so `yy` and `Cmd+C` on the same block write the same bytes to the
/// clipboard. It is [`project_subtree`] for several roots: one children
/// map built for all of them together, one [`group_properties_of`] pass
/// over every id the walk reached. A Visual range of N blocks is
/// therefore one property scan, not N — and not one per block, which is
/// what the clipboard used to pay through a per-node reader (root
/// `CLAUDE.md` invariant 15).
///
/// The roots must not nest (`copy_markdown` drops a root whose ancestor
/// is selected before calling this); a nested one would be emitted twice.
///
/// A root that is a **page** loses the page model's book-keeping keys
/// from its own properties, exactly as [`render_page_md`] drops them
/// from a page's header. Below a root nothing is filtered, for the
/// reason [`build_outline`] gives.
pub(crate) fn project_blocks(workspace: &Workspace, roots: &[NodeId]) -> Vec<OutlineNode> {
    let children = crate::tree::children_index_of(workspace, roots.to_vec());
    let skeleton: Vec<Skeleton> = roots
        .iter()
        .map(|&id| Skeleton {
            id,
            text: workspace.block_text(id).unwrap_or_default(),
            children: walk_subtree(workspace, id, &children),
        })
        .collect();
    let mut ids = Vec::with_capacity(16);
    collect_ids(&skeleton, &mut ids);
    ids.sort_unstable();

    let mut properties = group_properties_of(workspace, &ids, renderable_prop_value);
    for &root in roots {
        if workspace.tree().property(root, SLUG_KEY).is_some() {
            if let Some(own) = properties.get_mut(&root) {
                own.retain(|(k, _)| !is_page_model_key(k));
            }
        }
    }
    build_outline(skeleton, &properties)
}
