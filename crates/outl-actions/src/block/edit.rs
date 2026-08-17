//! Block text edits and the TODO / blockquote marker toggles.
//!
//! These rewrite the text of an **existing** node via `Op::Edit`. The
//! prefix arithmetic for TODO/DONE and the `"> "` quote marker lives in
//! [`crate::todo`] / [`crate::quote`] so every client agrees on the
//! wire format without re-implementing string surgery.

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::op::Op;
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::quote::toggle_quote as toggle_quote_prefix;
use crate::todo::cycle_todo;

use super::{ensure_in_tree, wrap};

/// Replace the block's text with `new_text` verbatim.
///
/// `new_text` is the **raw** wire text, prefix and all. Callers that
/// surface TODO/DONE state separately (e.g. the mobile client's
/// checkbox) must reattach the appropriate `TODO `/`DONE ` prefix
/// before calling — otherwise the state is lost. This intentionally
/// shifts the responsibility to the caller so the user can also
/// **drop** TODO/DONE by erasing the prefix in the editor; an
/// auto-preserve here would silently undo that intent.
pub fn edit_text(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
    new_text: &str,
) -> Result<(), ActionError> {
    ensure_in_tree(workspace, node)?;

    let update = workspace.build_text_replace_update(node, new_text);
    if update.is_empty() {
        return Ok(());
    }
    workspace.apply(wrap(
        hlc,
        Op::Edit {
            node,
            text_op: update,
        },
    ))?;
    Ok(())
}

/// Cycle the block's task state one step: `None → TODO → DOING → DONE → None`.
pub fn toggle_todo(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<(), ActionError> {
    ensure_in_tree(workspace, node)?;
    let current = workspace.block_text(node).unwrap_or_default();
    let next = cycle_todo(&current);
    let update = workspace.build_text_replace_update(node, &next);
    if update.is_empty() {
        return Ok(());
    }
    workspace.apply(wrap(
        hlc,
        Op::Edit {
            node,
            text_op: update,
        },
    ))?;
    Ok(())
}

/// Toggle the block's blockquote marker: adds or removes the
/// CommonMark `"> "` prefix on the block's text. See
/// [`crate::quote`] for the wire-format details. Mirrors
/// [`toggle_todo`] in shape — the actual prefix arithmetic lives in
/// [`crate::quote::toggle_quote`] so every client agrees on the
/// marker shape without re-implementing string surgery in TS.
pub fn toggle_quote(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<(), ActionError> {
    ensure_in_tree(workspace, node)?;
    let current = workspace.block_text(node).unwrap_or_default();
    let next = toggle_quote_prefix(&current);
    let update = workspace.build_text_replace_update(node, &next);
    if update.is_empty() {
        return Ok(());
    }
    workspace.apply(wrap(
        hlc,
        Op::Edit {
            node,
            text_op: update,
        },
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::append_block;
    use outl_core::id::ActorId;

    fn new_workspace() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    #[test]
    fn append_then_edit_changes_text() {
        let (mut ws, hlc) = new_workspace();
        let n = append_block(&mut ws, &hlc, None, Some("hello")).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("hello"));

        edit_text(&mut ws, &hlc, n, "hello world").unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("hello world"));
    }

    #[test]
    fn toggle_cycles_through_states() {
        let (mut ws, hlc) = new_workspace();
        let n = append_block(&mut ws, &hlc, None, Some("ship it")).unwrap();
        toggle_todo(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("TODO ship it"));
        toggle_todo(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("DOING ship it"));
        toggle_todo(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("DONE ship it"));
        toggle_todo(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("ship it"));
    }

    #[test]
    fn toggle_quote_flips_the_prefix_on_and_off() {
        let (mut ws, hlc) = new_workspace();
        let n = append_block(&mut ws, &hlc, None, Some("a quote")).unwrap();
        toggle_quote(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("> a quote"));
        toggle_quote(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("a quote"));
    }

    #[test]
    fn toggle_quote_composes_with_toggle_todo() {
        // Canonical encoding is `"TODO > body"` (TODO before the
        // quote marker) so the backend's `split_todo` still detects
        // the task state when the block is also quoted — without
        // this convention the DTO would land in mobile / desktop
        // with `todo = null` and the checkbox would disappear.
        let (mut ws, hlc) = new_workspace();
        let n = append_block(&mut ws, &hlc, None, Some("ship it")).unwrap();
        toggle_todo(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("TODO ship it"));
        toggle_quote(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("TODO > ship it"));
        // And the reverse: starting from a quoted block, toggle TODO
        // also lands the canonical order.
        toggle_quote(&mut ws, &hlc, n).unwrap(); // back to "TODO ship it"
        toggle_todo(&mut ws, &hlc, n).unwrap(); // → "DOING ship it"
        toggle_quote(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("DOING > ship it"));
    }

    #[test]
    fn cycle_todo_preserves_quote_marker_in_canonical_order() {
        // The quote marker survives a full TODO cycle and stays in
        // the canonical position (after the task state).
        let (mut ws, hlc) = new_workspace();
        let n = append_block(&mut ws, &hlc, None, Some("ship it")).unwrap();
        toggle_quote(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("> ship it"));
        toggle_todo(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("TODO > ship it"));
        toggle_todo(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("DOING > ship it"));
        toggle_todo(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("DONE > ship it"));
        toggle_todo(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("> ship it"));
    }

    #[test]
    fn edit_writes_text_verbatim_including_todo_prefix() {
        let (mut ws, hlc) = new_workspace();
        let n = append_block(&mut ws, &hlc, None, Some("ship it")).unwrap();
        toggle_todo(&mut ws, &hlc, n).unwrap();
        // Caller is responsible for keeping the prefix. The whole
        // point of dropping prefix-preservation is letting the user
        // erase `TODO `/`DONE ` from the editor and have it stick.
        edit_text(&mut ws, &hlc, n, "TODO ship the feature").unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("TODO ship the feature"));

        edit_text(&mut ws, &hlc, n, "ship the feature").unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("ship the feature"));
    }
}
