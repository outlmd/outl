//! Reading the trash back, and putting something back where it came
//! from.
//!
//! Invariant 6 makes delete a `Move(node, TRASH_ROOT)` — "simplifies the
//! algorithm and preserves history". The preserving half has worked since
//! day one. This module is the reading half: without it, the safety
//! property the invariant buys is "the bytes are still on disk", which is
//! a much weaker promise than "you can get it back" (issue #287).

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::op::Op;
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::tree::ChildrenIndex;

/// One top-level deletion.
///
/// "Top-level" is the node the `Move` named. Deleting a subtree is one
/// op on its root, so every block underneath rides along implicitly and
/// is **not** a separate entry — it comes back with its root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashEntry {
    /// The node the `Move` named — the root of what was deleted.
    pub node: NodeId,
    /// Where it sat immediately before the move to the trash, folded
    /// from the op log. See [`parent_at_deletion`].
    pub parent_at_deletion: Option<NodeId>,
    /// `Some(slug)` when the deleted node is a page root rather than a
    /// block. [`restore`] refuses these, so a listing that could not
    /// tell them apart would offer an action that always fails.
    pub page: Option<String>,
    /// How many blocks come back together — the root plus everything
    /// under it. This is what the user would lose if the trash were
    /// ever emptied.
    pub subtree_len: usize,
    /// One line of the block's text, for recognising it in a list.
    pub preview: String,
    /// Why [`restore`] would refuse this entry, or `None` when it would
    /// succeed. **Produced by the same function `restore` consults**
    /// ([`refusal_for`]) — a listing that decided this for itself would
    /// be a second owner of the rule, and the direction it drifts is
    /// offering the user an action that then fails.
    pub refusal: Option<String>,
}

/// Where [`restore`] would put `node`, or why it refuses.
///
/// **The single owner of that verdict.** [`restore`] asks it before
/// touching anything and [`list`] asks it per entry, so the listing and
/// the mutation cannot disagree — a listing that decided "restorable"
/// for itself would drift towards offering an action that then fails.
///
/// Returning the landing parent rather than a bare verdict is what lets
/// `restore` ask once: it used to call [`parent_at_deletion`] again
/// afterwards, and `list` paid for the fold twice per entry.
fn landing_for(workspace: &Workspace, node: NodeId) -> Result<NodeId, ActionError> {
    if !crate::tree::is_trashed(workspace, node) {
        return Err(ActionError::NotTrashed(node.to_string()));
    }

    // A page root is a `Move` *plus* a re-projected `.md`, and the slug
    // is usually taken by now. See `ActionError::TrashPageRestoreUnsupported`.
    if let Some(meta) = crate::page::page_meta(workspace, node) {
        return Err(ActionError::TrashPageRestoreUnsupported { slug: meta.slug });
    }

    // Past this point `node` is provably in the trash, so no arm may
    // answer `NotTrashed` — that is the contradiction
    // `a_block_the_log_cannot_place_is_not_reported_as_untrashed` pins.
    let unknown = |why: &str| ActionError::TrashOriginUnknown {
        node: node.to_string(),
        why: why.to_string(),
    };

    let parent = match parent_at_deletion(workspace, node) {
        Ok(Some(parent)) => parent,
        Ok(None) => return Err(unknown("no move in the log put it there")),
        Err(_) => return Err(unknown("its ops could not be read — run `outl doctor`")),
    };

    // The fold replays `Move.new_parent`, and invariant 4 keeps a `Move`
    // the tree **refused** as a cycle in the log. Such an op names a
    // target inside `node`'s own subtree, so believing it would produce
    // "restore X first" naming X itself, forever.
    if crate::tree::is_under(workspace, parent, node) {
        return Err(unknown(
            "the only move on record would have put it inside itself",
        ));
    }

    // `is_trashed` is an ancestor walk, not `parent(target) == trash`:
    // deleting a subtree is one `Move` on its root, so a node buried
    // inside a trashed subtree still points at a live-looking parent.
    if crate::tree::is_trashed(workspace, parent) {
        return Err(ActionError::TrashParentTrashed {
            node: node.to_string(),
            parent: parent.to_string(),
        });
    }

    if !workspace.tree().contains(parent) {
        return Err(ActionError::TrashParentMissing {
            node: node.to_string(),
            parent: parent.to_string(),
        });
    }

    Ok(parent)
}

/// Why [`restore`] would refuse `node`, or `None` when it would work.
///
/// The read-only half of `landing_for`, for a caller that wants the
/// verdict without the destination.
pub fn refusal_for(workspace: &Workspace, node: NodeId) -> Option<ActionError> {
    landing_for(workspace, node).err()
}

/// Every top-level deletion currently in the trash.
///
/// Reads the **materialized tree**, not the op log's deletion history:
/// a block that was deleted and then restored is not in the trash and
/// does not belong in a listing of what can be restored.
pub fn list(workspace: &Workspace) -> Vec<TrashEntry> {
    // The whole-workspace index: this walks every top-level deletion, so
    // the one scan is the cheap side of `children_index`'s trade-off.
    let children = crate::tree::children_index_unordered(workspace);

    let mut entries: Vec<TrashEntry> = children
        .get(&NodeId::trash())
        .into_iter()
        .flatten()
        .map(|&node| entry_for(workspace, &children, node))
        .collect();

    // By id, which is ULID, which sorts by creation time. Deliberately
    // not by deletion time: that would need a second fold of the log per
    // entry for an ordering nobody asked for.
    entries.sort_by_key(|entry| entry.node);
    entries
}

/// Describe one trashed node.
fn entry_for(workspace: &Workspace, children: &ChildrenIndex, node: NodeId) -> TrashEntry {
    // One ask, both halves. Where it would land and why it would not are
    // the same question, and asking twice was the whole cost of listing.
    let landing = landing_for(workspace, node);
    TrashEntry {
        node,
        parent_at_deletion: landing.as_ref().ok().copied(),
        page: crate::page::page_meta(workspace, node).map(|meta| meta.slug),
        subtree_len: crate::tree::subtree_ids(children, node).len(),
        preview: preview(workspace, node),
        refusal: landing.as_ref().err().map(|err| err.to_string()),
    }
}

/// One-line preview of a block's text, safe to print.
fn preview(workspace: &Workspace, node: NodeId) -> String {
    let text = workspace.block_text(node).unwrap_or_default();
    let single = text.replace(['\n', '\r'], " ");
    let trimmed = single.trim();
    if trimmed.is_empty() {
        return "(empty block)".to_string();
    }
    let mut out: String = trimmed.chars().take(PREVIEW_CHARS).collect();
    if trimmed.chars().count() > PREVIEW_CHARS {
        out.push('…');
    }
    out
}

/// How much of a block's text a listing shows.
const PREVIEW_CHARS: usize = 80;

/// The parent a node sat under immediately before it was moved to the
/// trash, or `None` if the log never moved it there.
///
/// **Folded from `Create.parent` / `Move.new_parent`, never read off
/// `Move.old_parent`.** That field is the originating replica's local
/// derivation for `undo_op`, and the root `CLAUDE.md` says a reader of
/// the log as data must not trust it: on the workspace this was built
/// against, 65,141 of 65,703 stored `Move`s name `root` as the old
/// parent regardless of where the block actually was. An append-only
/// log never rewrites them, so reading the field would work on this
/// week's history and lie about every year before it.
///
/// This is the same fold [`crate::timeline`] needs to attribute a
/// deletion to a page, and it is deliberately the only one — two
/// answers to "where did this block live" is the drift the reuse-first
/// rule exists to prevent.
pub fn parent_at_deletion(
    workspace: &Workspace,
    node: NodeId,
) -> Result<Option<NodeId>, ActionError> {
    let ops = workspace.ops_for_node(node)?;
    let mut parent: Option<NodeId> = None;
    let mut created = false;
    let mut at_deletion: Option<NodeId> = None;

    for logged in &ops {
        match &logged.op {
            // First `Create` only. A reconcile re-emits one for a block
            // that already exists, and the tree discards the stale
            // parent it names (`do_op`'s `if !self.nodes.contains_key`),
            // so trusting it here would fold a parent the tree never had.
            Op::Create { parent: p, .. } if !created => {
                created = true;
                parent = Some(*p);
            }
            Op::Create { .. } => {}
            Op::Move { new_parent, .. } => {
                // A node cannot become its own parent: `creates_cycle`
                // always refuses this one, so the tree never applied it
                // and neither may the fold (invariant 4). The wider case
                // — a target deeper inside `node`'s subtree — needs the
                // tree as it was then, so `landing_for` catches it
                // against the tree as it is now instead.
                if *new_parent == node {
                    continue;
                }
                // Re-emitting a block's current parent moves nothing, and
                // must not overwrite the answer with the trash itself.
                if parent == Some(*new_parent) {
                    continue;
                }
                if *new_parent == NodeId::trash() {
                    at_deletion = parent;
                }
                parent = Some(*new_parent);
            }
            _ => {}
        }
    }

    Ok(at_deletion)
}

/// Put `node` back under the parent it was deleted from.
pub fn restore(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<TrashEntry, ActionError> {
    let target = landing_for(workspace, node)?;

    // Captured before the move: afterwards the node is no longer in the
    // trash, and an entry describing where it *now* lives would be a
    // receipt for something the caller did not ask for.
    let entry = entry_for(
        workspace,
        &crate::tree::children_index(workspace, node),
        node,
    );
    crate::block::move_under(workspace, hlc, node, target)?;
    Ok(entry)
}

#[cfg(test)]
mod tests;
