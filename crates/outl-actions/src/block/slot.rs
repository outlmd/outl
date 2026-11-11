//! Making room where the fractional index has none.
//!
//! `a..=z` is dense enough for a midpoint and not dense enough for
//! every pair: a tie between two siblings has no interior (#282), and
//! neither does a neighbour sitting one floor byte above (`"b"` /
//! `"ba"`, #476). `crate::tree::interior_between` is the single owner
//! of noticing that — [`crate::tree::position_before`],
//! [`crate::tree::position_after`] and [`slot_between`] all ask it
//! rather than each comparing against the bound themselves.
//!
//! What a caller does with that `None` is one decision, taken here
//! rather than five times. [`vacate`] is the primitive — free the key a
//! node holds by shifting it up — [`slot_between`] is the decision, and
//! [`slot_after`] is the "insert after this one" shape of it.
//!
//! **None of this is a migration.** The obvious misreading is "the
//! checks exist for op logs written before the mint rule", which is the
//! reading that deletes them once every device has upgraded. The forced
//! floor tail keeps `between` minting `(k, k + "a")` pairs on an
//! all-new workspace (18,278 of 12.8M sampled gaps), and a peer on an
//! older binary keeps minting them tomorrow rather than only yesterday.
//! [`crate::tree::position_after`] carries the measurement.
//!
//! **Two rules, and the second is why this is not just a retry.** A new
//! block may land one slot off its intended place; an existing sibling
//! may never move to a key it cannot be cleared into. Relocating content
//! the user did not touch is the worse of the two failures, and it is
//! the one that looks like data loss.

use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::tree::{interior_between, next_sibling};

use super::moves::move_to;

/// Free the key `node` holds, and return it.
///
/// Shifts `node` up into the gap toward its own next sibling, so the key
/// it was sitting on is available for a block that has to sort below it.
/// Mirrors what `move_up` does rather than minting a sub-floor key,
/// which the `[a-z]` index has no room for.
///
/// **The gap above `node` can be empty too**, and then there is nothing
/// to shift into: [`Fractional::between`] drops a bound it cannot
/// satisfy, so the shifted key comes back sorting *past* the next
/// sibling and the move would send `node` below a block the user never
/// touched. Two shapes reach that — `node` tied with its successor
/// (three offline creates under one parent), and a successor that is
/// `node`'s key plus a single floor byte. There the key is returned
/// **without** moving anything, so the caller's block shares it with
/// `node` and `sort_siblings`' `NodeId` tiebreak decides which of the
/// two comes first.
///
/// That is not a shrug: a tie is not separable without choosing a
/// `NodeId`, and no caller here gets to choose one. One block landing a
/// slot off is the price of never relocating a sibling.
pub(super) fn vacate(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<Fractional, ActionError> {
    let parent = workspace
        .tree()
        .parent(node)
        .ok_or_else(|| ActionError::NotInTree(node.to_string()))?;
    let held = workspace
        .tree()
        .position(node)
        .cloned()
        .ok_or_else(|| ActionError::MissingPosition(node.to_string()))?;
    let above = next_sibling(workspace, node).and_then(|n| workspace.tree().position(n).cloned());
    let shifted = Fractional::between(Some(&held), above.as_ref());
    if above.as_ref().is_none_or(|next| &shifted < next) {
        move_to(workspace, hlc, node, parent, shifted)?;
    }
    Ok(held)
}

/// A fractional slot strictly between two sibling positions, opening
/// one when the gap has none.
///
/// **The one place that decides what an empty gap costs**, and the one
/// shape every caller can reach: `right` carries the neighbour's
/// `NodeId` because freeing its key is the only way to make room, so a
/// caller that has only positions cannot open a slot and a caller that
/// passes `None` is saying there is no upper bound to honour.
///
/// Three answers, and the middle one is the subtle one:
///
/// - the gap holds a key → that key, nothing moves;
/// - `left` and `right` are **tied** → a key above both, nothing moves.
///   Freeing `right`'s key would hand the new block `left`'s own
///   position, where `sort_siblings`' `NodeId` tiebreak could sort it
///   *above* the block the caller asked to insert after. One slot
///   further than asked is a smaller wrong than the wrong side;
/// - `right` is `left` plus a floor byte → [`vacate`] it and take the
///   freed key. This is issue #476's shape, and the only one where
///   repositioning the neighbour actually buys the slot.
pub(crate) fn slot_between(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    left: Option<&Fractional>,
    right: Option<(NodeId, &Fractional)>,
) -> Result<Fractional, ActionError> {
    if let Some(slot) = interior_between(left, right.map(|(_, pos)| pos)) {
        return Ok(slot);
    }
    // No upper bound is always honoured, so `interior_between` cannot
    // decline one — belt to its braces, and a key above `left` is what
    // an absent bound asks for anyway.
    let Some((right_node, right_pos)) = right else {
        return Ok(Fractional::between(left, None));
    };
    if left == Some(right_pos) {
        return Ok(Fractional::between(left, None));
    }
    vacate(workspace, hlc, right_node)
}

/// A fractional slot for a block that must sort immediately after
/// `node`, opening one when the gap has none.
///
/// [`slot_between`] in the shape the four block actions want: `outdent`
/// (promote a child to sit after its old parent), `move_after` (the
/// cut-and-paste primitive) and `create_after` (Enter on a block). Each
/// one used to take `between`'s key unchecked, so when the gap above
/// `node` was empty the block landed **behind** the next sibling —
/// issue #476's `P c4 c3 c5 …`.
///
/// Appending at the end of the parent is deliberately not the fallback:
/// that is the same misplacement, moved further.
pub(super) fn slot_after(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<Fractional, ActionError> {
    let node_pos = workspace
        .tree()
        .position(node)
        .cloned()
        .ok_or_else(|| ActionError::MissingPosition(node.to_string()))?;
    let next = next_sibling(workspace, node)
        .and_then(|n| workspace.tree().position(n).cloned().map(|pos| (n, pos)));
    slot_between(
        workspace,
        hlc,
        Some(&node_pos),
        next.as_ref().map(|(n, pos)| (*n, pos)),
    )
}
