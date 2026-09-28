//! Minting a block: the `Op::Create` primitive, and the two entry
//! points that need no anchor to place one.
//!
//! Everything here appends as a **last child**, which is the one
//! position that needs no knowledge of the siblings around it. Placing
//! a block relative to an existing one is [`super::siblings`], building
//! a whole subtree in one batch is [`super::forest`], and re-parenting
//! or editing an existing node is [`super::moves`] / [`super::edit`].

use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::op::Op;
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::tree::position_for_new_last_child;

use super::edit::edit_text;
use super::wrap;

/// Append a brand-new block as the last child of `parent` and return
/// its id. `parent` defaults to [`NodeId::root`] when not supplied.
pub fn append_block(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    parent: Option<NodeId>,
    text: Option<&str>,
) -> Result<NodeId, ActionError> {
    let parent = parent.unwrap_or_else(NodeId::root);
    let position = position_for_new_last_child(workspace, parent);
    create_with_position(workspace, hlc, parent, position, text)
}

/// Append a new block as the last child of `parent`. Synonym for
/// [`append_block`] when the parent is explicit.
pub fn create_under(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    parent: NodeId,
    text: Option<&str>,
) -> Result<NodeId, ActionError> {
    let position = position_for_new_last_child(workspace, parent);
    create_with_position(workspace, hlc, parent, position, text)
}

pub(super) fn create_with_position(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    parent: NodeId,
    position: Fractional,
    text: Option<&str>,
) -> Result<NodeId, ActionError> {
    create_with_explicit_id(workspace, hlc, NodeId::new(), parent, position, text)
}

/// Create variant that uses a caller-supplied `node` id instead of a
/// fresh ULID.
///
/// Used by [`crate::page::open_or_create`] so that two peers
/// independently materialising the same slug end up with the same
/// `NodeId`. With a random id, each device would create a separate
/// page node and the CRDT would have no way to merge them after the
/// fact (different ids = different subtrees).
///
/// Re-creating an already-existing node is a no-op at the CRDT layer
/// (the second `Op::Create` is dropped because the node is already in
/// the tree), which makes this safe to call even when the page
/// already exists locally.
pub(crate) fn create_with_explicit_id(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
    parent: NodeId,
    position: Fractional,
    text: Option<&str>,
) -> Result<NodeId, ActionError> {
    workspace.apply(wrap(
        hlc,
        Op::Create {
            node,
            parent,
            position,
        },
    ))?;

    if let Some(body) = text {
        let trimmed = body.trim();
        if !trimmed.is_empty() {
            edit_text(workspace, hlc, node, trimmed)?;
        }
    }
    Ok(node)
}
