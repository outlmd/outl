//! Tree → markdown projection (page and single-block forms).

use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_md::parse::{OutlineNode, ParsedPage};
use outl_md::render::render;

use crate::tree::{children_of, is_page_model_key, renderable_prop_value};

pub(super) fn build_outline(workspace: &Workspace, parent: NodeId) -> Vec<OutlineNode> {
    children_of(workspace, parent)
        .into_iter()
        .map(|(id, _)| OutlineNode {
            text: workspace.block_text(id).unwrap_or_default(),
            properties: block_properties(workspace, id),
            children: build_outline(workspace, id),
        })
        .collect()
}

/// Block-level properties as `(key, value)` pairs, alpha-sorted so the
/// rendered `.md` is stable across runs.
///
/// Reconcile parses `key:: value` continuation lines into
/// `Op::SetProp` on the block node — this is the projection back.
/// Rendering them used to be skipped entirely (`Vec::new()`), so any
/// op → `.md` re-render silently deleted the property lines from disk
/// and the next external-edit reconcile emitted prop-removal ops:
/// convergent data loss surfaced by the importer's resolve pass.
/// **No page-model filtering here, deliberately.** `page-slug` /
/// `page-kind` are book-keeping *on a page root*; on an ordinary block
/// they are whatever the user typed. The dialect has no allow-list of
/// property keys, so `parse_property_line` accepts them and `diff_to_ops`
/// emits a `SetProp` like any other. Dropping them on render would put
/// the value in the tree and nowhere on disk, and the next external-edit
/// reconcile would emit the removal, which is the same convergent loss
/// the doc above describes.
fn block_properties(workspace: &Workspace, id: NodeId) -> Vec<(String, String)> {
    let mut props: Vec<(String, String)> = workspace
        .tree()
        .properties_of(id)
        .filter_map(|(k, v)| renderable_prop_value(v).map(|s| (k.to_string(), s)))
        .collect();
    props.sort_by(|a, b| a.0.cmp(&b.0));
    props
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
/// Sort order is alphabetical on the key — `HashMap::iter` is
/// unordered, and we don't want the rendered `.md` to flap between
/// runs. The renderer doesn't care about order; users do.
pub fn render_page_md(workspace: &Workspace, page_root: NodeId) -> String {
    let mut properties: Vec<(String, String)> = workspace
        .tree()
        .properties_of(page_root)
        .filter(|(k, _)| !is_page_model_key(k))
        .filter_map(|(k, v)| renderable_prop_value(v).map(|s| (k.to_string(), s)))
        .collect();
    properties.sort_by(|a, b| a.0.cmp(&b.0));

    let page = ParsedPage {
        properties,
        blocks: build_outline(workspace, page_root),
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
    let block = OutlineNode {
        text: workspace.block_text(node).unwrap_or_default(),
        properties: block_properties(workspace, node),
        children: build_outline(workspace, node),
    };
    let page = ParsedPage {
        properties: Vec::new(),
        blocks: vec![block],
        warnings: Vec::new(),
    };
    render(&page)
}
