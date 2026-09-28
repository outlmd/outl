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

/// Reject an action whose target node is no longer in the materialised
/// tree (e.g. a peer trashed it between the client's read and write).
fn ensure_in_tree(workspace: &Workspace, node: NodeId) -> Result<(), ActionError> {
    if workspace.tree().contains(node) {
        Ok(())
    } else {
        Err(ActionError::NotInTree(node.to_string()))
    }
}
