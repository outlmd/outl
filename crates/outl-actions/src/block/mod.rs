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
//!   the anchor is stale.
//! - `slot` — what to do when the fractional index has **no** slot
//!   between two siblings: free a neighbour's key, and which of the two
//!   empty gaps can be cleared at all. Shared by `siblings`, `moves`
//!   and `crate::desync` (hence `pub(crate)`) so the five places that
//!   place a node next to an existing one cannot disagree about it.
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
pub(crate) mod slot;
mod split;

pub(crate) use create::create_with_explicit_id;
pub use create::{append_block, create_under};
pub use edit::{edit_text, mark_done, toggle_quote, toggle_todo};
pub use forest::{append_forest, append_tree, BlockTreeOutcome, BlockTreeSpec};
pub(crate) use moves::move_to;
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
///
/// The predicate is [`crate::tree::is_live`]'s, not a local pair of
/// conditions. This function used to spell the walk out itself — its
/// own `in_trash`, bounded, beside a `tree::is_under` that was not —
/// and *"two copies of one walk where only one is safe is how the
/// unbounded one ends up on the hottest path"*. The bound moved to the
/// survivor then; the same argument is why the **question** has one
/// owner now, because this function was private and both GUI and TUI
/// clients hand-rolled their own `bool`-shaped copy of it in front of a
/// cross-page backlink write.
fn ensure_live(workspace: &Workspace, node: NodeId) -> Result<(), ActionError> {
    if crate::tree::is_live(workspace, node) {
        Ok(())
    } else {
        Err(ActionError::NotInTree(node.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::{append_block, delete, ensure_live};
    use crate::page::{open_or_create as open_or_create_page, PageKind};
    use crate::tree::is_live;
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::{ActorId, NodeId};
    use outl_core::workspace::Workspace;

    /// **The `bool` and the `Result` cannot disagree about "live".**
    ///
    /// `ensure_live` is private, so both clients' cross-page write gates
    /// re-derived the predicate as a `bool`; now all three ask
    /// [`is_live`]. The four states are asserted together rather than
    /// each against its own expectation, because the defect this
    /// prevents is not a wrong answer — it is two answers.
    ///
    /// Drop the trash half from either side and the two trashed rows
    /// fail.
    #[test]
    fn the_bool_and_the_result_cannot_disagree_about_live() {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).expect("in-memory workspace");
        let page = open_or_create_page(&mut ws, &hlc, "notes", "Notes", PageKind::Page)
            .expect("a page to hang the blocks off");
        let live = append_block(&mut ws, &hlc, Some(page), Some("live")).expect("live block");
        let trashed = append_block(&mut ws, &hlc, Some(page), Some("doomed")).expect("doomed");
        let under_trashed =
            append_block(&mut ws, &hlc, Some(trashed), Some("child")).expect("its child");
        delete(&mut ws, &hlc, trashed).expect("a peer deletes the subtree root");

        for (what, node) in [
            ("a live block", live),
            ("a trashed block", trashed),
            // The whole reason `is_trashed` is an ancestor walk: one
            // `Op::Move` on the subtree root leaves every block under it
            // still pointing at the trashed parent.
            ("a block under a trashed one", under_trashed),
            ("an id the tree never held", NodeId::new()),
        ] {
            assert_eq!(
                is_live(&ws, node),
                ensure_live(&ws, node).is_ok(),
                "{what}: the predicate and the guard answer differently",
            );
        }
        // And the guard is not vacuously agreeing with itself: the four
        // rows above have to cover both verdicts.
        assert!(is_live(&ws, live));
        assert!(!is_live(&ws, under_trashed));
    }
}
