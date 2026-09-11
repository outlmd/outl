//! Tree → markdown projection (page and single-block forms).

use std::collections::HashMap;

use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_md::parse::{OutlineNode, ParsedPage};
use outl_md::render::render;

use crate::outline::ChildrenIndex;
use crate::tree::{is_page_model_key, renderable_prop_value};

/// Renderable properties of the nodes in one projection, grouped by the
/// node carrying them and alpha-sorted per node — exactly the shape a
/// rendered block wants, so the per-node path is a map lookup.
type PropertiesByNode = HashMap<NodeId, Vec<(String, String)>>;

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

/// Every renderable property carried by one of `ids`, grouped by node.
///
/// `ids` must be sorted — the scan binary-searches it per property.
///
/// **One scan for the whole projection, not one per node.**
/// [`outl_core::tree::Tree::properties_of`] filters the workspace-wide
/// property map on *every* call, so asking it per node is
/// `O(nodes × properties)`; its own doc comment says as much. Measured
/// on a synthetic 64k-node / 192k-property workspace, `render_page_md`
/// went from 13.8 ms/page to 4.4 ms/page purely from collapsing those
/// per-node filters into this single `iter_properties` pass.
///
/// Note this scan is **subtree-scoped**, which is the part that is easy
/// to get wrong. Grouping the *whole* workspace instead — the obvious
/// reading of "build the map once" — is a net loss here, because a page
/// render then allocates a `Vec` and two `String`s for all 192k
/// properties to read the ~80 belonging to its own blocks. Measured, it
/// was 2.7× *slower* than the per-node filtering it replaced. The
/// whole-workspace map is right for a whole-workspace pass (see
/// `crate::index::build_properties_index`) and wrong for a single page.
fn subtree_properties(workspace: &Workspace, ids: &[NodeId]) -> PropertiesByNode {
    let mut grouped: PropertiesByNode = HashMap::new();
    for (node, key, value) in workspace.tree().iter_properties() {
        if ids.binary_search(&node).is_err() {
            continue;
        }
        let Some(text) = renderable_prop_value(value) else {
            continue;
        };
        grouped
            .entry(node)
            .or_default()
            .push((key.to_string(), text));
    }
    // Alpha-sorted so the rendered `.md` is stable across runs: the
    // property map is a `HashMap`, whose iteration order is seeded per
    // process. The renderer doesn't care about order; users reading a
    // diff do. Keys are unique per node, so there are no ties and the
    // order is fully determined.
    for props in grouped.values_mut() {
        props.sort_by(|a, b| a.0.cmp(&b.0));
    }
    grouped
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

    let properties = subtree_properties(workspace, &ids);
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
/// Internal book-keeping keys (`page-slug` / `page-kind`) are skipped:
/// the page-model layer (`outl_actions::page`) owns those through its
/// own ops; surfacing them in the rendered `.md` would re-write the
/// slug on every reconcile (a no-op via the CRDT, but noise on disk).
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
    properties.retain(|(k, _)| !is_page_model_key(k));

    let page = ParsedPage {
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
    let index = crate::tree::children_index(workspace, node);
    let (properties, children) = project_subtree(workspace, node, &index);
    let block = OutlineNode {
        text: workspace.block_text(node).unwrap_or_default(),
        properties,
        children,
    };
    let page = ParsedPage {
        properties: Vec::new(),
        blocks: vec![block],
        warnings: Vec::new(),
    };
    render(&page)
}
