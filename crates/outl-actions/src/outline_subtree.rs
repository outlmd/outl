//! How deep a borrowed subtree is carried: the cap, and the two ways to
//! apply it. Split out of `outline.rs`, which is the projection itself.

use crate::outline::OutlineNode;

/// How many levels of a borrowed subtree a client ever draws: an embed
/// (`!((blk-XXXXXX))`) and a backlink row's children alike. Mirrored by
/// the frontend's `EmbeddedSubtree` `MAX_DEPTH` and the TUI's
/// `EMBED_MAX_DEPTH`, so shipping a deeper level is pure IPC weight.
/// Keep the three in step; this is the Rust owner.
pub const SUBTREE_MAX_DEPTH: usize = 4;

/// Drop the levels of `nodes` the client will not render. `nodes` is
/// level 1; once `depth_left` hits 1 the current level keeps its text
/// but loses its children.
pub fn truncate_children_depth(nodes: &mut [OutlineNode], depth_left: usize) {
    if depth_left <= 1 {
        for node in nodes {
            node.children.clear();
        }
        return;
    }
    for node in nodes {
        truncate_children_depth(&mut node.children, depth_left - 1);
    }
}

/// Deep-copy `nodes` down to `depth_left` levels, **never copying the
/// levels it would drop**. Equivalent to `clone` + [`truncate_children_depth`]
/// without paying for the clone of the part that is thrown away, which on
/// a hub block is most of it.
pub fn clone_capped(nodes: &[OutlineNode], depth_left: usize) -> Vec<OutlineNode> {
    nodes
        .iter()
        .map(|n| OutlineNode {
            id: n.id.clone(),
            text: n.text.clone(),
            todo: n.todo,
            collapsed: n.collapsed,
            properties: n.properties.clone(),
            tokens: n.tokens.clone(),
            table: n.table.clone(),
            children: if depth_left <= 1 {
                Vec::new()
            } else {
                clone_capped(&n.children, depth_left - 1)
            },
        })
        .collect()
}
