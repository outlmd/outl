//! Workspace mutations expressed as high-level user actions.
//!
//! Every function in this module tree:
//!
//! 1. Reads the current tree to figure out the right `Op` parameters
//!    (parent id, position, undo fields, ...).
//! 2. Generates a fresh [`LogOp`] via the caller-supplied
//!    [`HlcGenerator`].
//! 3. Routes it through [`Workspace::apply`] so the op log stays the
//!    single source of truth.
//!
//! The functions never reach for storage directly and never touch
//! filesystem state — the caller decides whether (and when) to
//! re-render the markdown projection by calling
//! [`crate::journal::apply_page_md`] or
//! [`crate::journal::apply_all_pages_md`].
//!
//! The surface is split by responsibility so no single file owns
//! unrelated concerns:
//!
//! - `create` — mint a block with no anchor: append it as a last
//!   child, and the `Op::Create` primitive every other file here uses.
//! - `siblings` — place a new block relative to an existing one
//!   (`create_before` / `create_after`), including what happens when
//!   the fractional index has no slot and when the anchor is stale.
//! - `forest` — build a whole subtree, or a forest of them, in one
//!   batched call.
//! - `edit` — rewrite a block's text and toggle its TODO / quote
//!   markers.
//! - `moves` — re-parent (incl. the arbitrary cross-page
//!   `move_under` the plugin host applies), reorder, and delete
//!   existing nodes (delete is a move to the trash root, per
//!   invariant #6).
//!
//! `wrap` and `ensure_in_tree` are the shared helpers every
//! submodule reaches for via `super::`.

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::op::{LogOp, Op};
use outl_core::workspace::Workspace;

use crate::error::ActionError;

mod create;
mod edit;
mod forest;
mod moves;
mod siblings;
mod split;

pub(crate) use create::create_with_explicit_id;
pub use create::{append_block, create_under};
pub use edit::{edit_text, toggle_quote, toggle_todo};
pub use forest::{append_forest, append_tree, BlockTreeOutcome, BlockTreeSpec};
pub use moves::{delete, indent, move_after, move_down, move_under, move_up, outdent};
pub use siblings::{create_after, create_after_or_append, create_before, create_before_or_append};
pub use split::split_block;

/// Build a [`LogOp`] wrapping `op` with a fresh HLC.
fn wrap(hlc: &HlcGenerator, op: Op) -> LogOp {
    let ts = hlc.next();
    LogOp {
        ts,
        actor: ts.actor,
        op,
    }
}

/// Reject an action whose target node has no record in the tree at all.
///
/// **This does not mean "alive".** Invariant 6 makes delete a
/// `Move(node, TRASH_ROOT)`, so a trashed node keeps its record and
/// answers `contains` — which is exactly what lets `trash::restore`
/// move one back out. Paths that must not touch deleted content want
/// [`ensure_live`] instead.
fn ensure_in_tree(workspace: &Workspace, node: NodeId) -> Result<(), ActionError> {
    if workspace.tree().contains(node) {
        Ok(())
    } else {
        Err(ActionError::NotInTree(node.to_string()))
    }
}

/// Reject an action whose target is gone *or* sitting in the trash.
///
/// A peer's delete arrives as a move under `TRASH_ROOT`, and the node
/// keeps its record, so `contains` still says yes. Editing then wrote
/// into deleted content, and `create_after` on a trashed anchor built
/// the new block **under the trash**, where the user never saw the
/// keystroke land.
///
/// Returns `NotInTree`, deliberately the same error: the stale-anchor
/// fallback in [`create_after_or_append`](siblings::create_after_or_append)
/// keys on it, so a trashed anchor now appends to the page instead of
/// disappearing.
fn ensure_live(workspace: &Workspace, node: NodeId) -> Result<(), ActionError> {
    ensure_in_tree(workspace, node)?;
    if in_trash(workspace, node) {
        return Err(ActionError::NotInTree(node.to_string()));
    }
    Ok(())
}

/// Whether `node` sits anywhere under `TRASH_ROOT`.
///
/// Walks parents rather than checking one level: deleting a parent
/// trashes its whole subtree, and every descendant keeps pointing at
/// the parent. Bounded because a corrupt log must not hang a keystroke.
fn in_trash(workspace: &Workspace, node: NodeId) -> bool {
    let tree = workspace.tree();
    let trash = NodeId::trash();
    let mut at = node;
    for _ in 0..1024 {
        match tree.parent(at) {
            Some(p) if p == trash => return true,
            Some(p) if p == NodeId::root() => return false,
            Some(p) => at = p,
            None => return false,
        }
    }
    false
}
