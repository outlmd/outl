//! The blocks under a citing block, as the panel's cursor sees them.
//!
//! A `Backlink` ships shallow on purpose (the TUI clones the list every
//! frame), so a citing block's children are **not** in it. They live in
//! the index's side table and are read here as `Arc<[OutlineNode]>`:
//! a refcount bump per call, never a deep copy. `Focus::Backlink`
//! carries a [`NodeId`] into that subtree, not a path, because a path
//! goes stale on the first refetch.

use std::sync::Arc;

use outl_actions::{Backlink, OutlineNode};
use outl_core::id::NodeId;

use crate::actions::node_id;
use crate::state::{App, Focus};

/// Ids of every node under `nodes`, depth first, parents before their
/// children. Nodes whose id is not a ULID are skipped.
fn descendant_ids(nodes: &[OutlineNode], out: &mut Vec<NodeId>) {
    for node in nodes {
        out.extend(node_id(&node.id));
        descendant_ids(&node.children, out);
    }
}

fn find_node(nodes: &[OutlineNode], id: NodeId) -> Option<&OutlineNode> {
    nodes.iter().find_map(|n| {
        if node_id(&n.id) == Some(id) {
            Some(n)
        } else {
            find_node(&n.children, id)
        }
    })
}

impl App {
    /// The capped subtree under a citing block: one lookup and a
    /// refcount bump. `None` for a leaf, an unknown id, or an index
    /// that is not built yet.
    pub(crate) fn backlink_subtree(&self, block_id: &str) -> Option<Arc<[OutlineNode]>> {
        self.backlink_index.borrow().as_ref()?.subtree(block_id)
    }

    /// The cursor positions of one row in walk order: the citing block
    /// (`None`), then its descendants depth first.
    pub(crate) fn backlink_row_stops(&self, bl: &Backlink) -> Vec<Option<NodeId>> {
        let mut ids = Vec::new();
        if let Some(sub) = self.backlink_subtree(&bl.block_id) {
            descendant_ids(&sub, &mut ids);
        }
        std::iter::once(None)
            .chain(ids.into_iter().map(Some))
            .collect()
    }

    /// The block `node` names under `bl`: the citing block for `None`,
    /// the descendant for `Some`.
    ///
    /// `None` when `node` is a descendant the subtree no longer holds.
    /// Falling back to the citing block here would aim a delete or an
    /// indent at the parent and its whole subtree after a peer removed
    /// the child the user was on; the fall-back belongs to
    /// `clamp_backlink_focus`, which moves the *cursor*, never to a
    /// resolver a mutation reads.
    pub(crate) fn backlink_block_of(&self, bl: &Backlink, node: Option<NodeId>) -> Option<NodeId> {
        match node {
            None => node_id(&bl.block_id),
            Some(n) => self.backlink_row_stops(bl).contains(&Some(n)).then_some(n),
        }
    }

    /// The text of the block the cursor names inside `bl`.
    pub(crate) fn backlink_text_of(&self, bl: &Backlink, node: Option<NodeId>) -> String {
        let Some(id) = node else {
            return bl.source_block.text.clone();
        };
        self.backlink_subtree(&bl.block_id)
            .and_then(|sub| find_node(&sub, id).map(|n| n.text.clone()))
            .unwrap_or_else(|| bl.source_block.text.clone())
    }

    /// Where the panel's cursor goes to name `block`: the row whose
    /// citing block it is, else the first row whose subtree holds it.
    /// `None` when the list no longer shows it anywhere.
    ///
    /// Roots win over descendants: a descendant that cites the page is
    /// drawn twice (as its own row and inside its ancestor's subtree)
    /// and the row is the stable one.
    pub(crate) fn backlink_focus_of(&self, block: NodeId) -> Option<Focus> {
        let rows = self.backlinks_for_current();
        if let Some(idx) = rows
            .iter()
            .position(|bl| node_id(&bl.block_id) == Some(block))
        {
            return Some(Focus::Backlink { idx, node: None });
        }
        rows.iter().enumerate().find_map(|(idx, bl)| {
            self.backlink_row_stops(bl)
                .contains(&Some(block))
                .then_some(Focus::Backlink {
                    idx,
                    node: Some(block),
                })
        })
    }

    /// One `j` inside the panel: next stop of this row, else the next
    /// row. `false` at the end of the list.
    pub(super) fn backlink_step_forward(&mut self, idx: usize, node: Option<NodeId>) -> bool {
        let rows = self.backlinks_for_current();
        let Some(bl) = rows.get(idx) else {
            return false;
        };
        let stops = self.backlink_row_stops(bl);
        let at = stops.iter().position(|s| *s == node).unwrap_or(0);
        self.focus = match stops.get(at + 1) {
            Some(next) => Focus::Backlink { idx, node: *next },
            None if idx + 1 < rows.len() => Focus::Backlink {
                idx: idx + 1,
                node: None,
            },
            None => return false,
        };
        self.cursor_col = 0;
        true
    }

    /// One `k` inside the panel: previous stop of this row, else the
    /// last stop of the previous row, else back out to the outline.
    /// `false` for a stale `idx`.
    pub(super) fn backlink_step_backward(&mut self, idx: usize, node: Option<NodeId>) -> bool {
        let rows = self.backlinks_for_current();
        let Some(bl) = rows.get(idx) else {
            return false;
        };
        let stops = self.backlink_row_stops(bl);
        let at = stops.iter().position(|s| *s == node).unwrap_or(0);
        if at > 0 {
            self.focus = Focus::Backlink {
                idx,
                node: stops[at - 1],
            };
        } else if idx > 0 {
            let before = self.backlink_row_stops(&rows[idx - 1]);
            self.focus = Focus::Backlink {
                idx: idx - 1,
                node: before.last().copied().flatten(),
            };
        } else {
            self.focus = Focus::Outline;
            self.selected = self.flat_len.saturating_sub(1);
        }
        self.cursor_col = 0;
        true
    }
}
