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
use crate::todo::{cycle_todo, set_todo};
use crate::TodoState;

use super::{ensure_live, wrap};

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
    ensure_live(workspace, node)?;

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
    ensure_live(workspace, node)?;
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

/// Mark the block's task state DONE outright, whatever it was: no
/// marker, `TODO`, `DOING`, a CommonMark checkbox, or already `DONE`.
///
/// Unlike [`toggle_todo`] this is **idempotent** — running it on an
/// already-`DONE` block is a genuine no-op (`build_text_replace_update`
/// returns empty, so no op is even logged). It also skips straight
/// past the intermediate states instead of walking one step at a time,
/// which is exactly what a "finish this" gesture needs: pressing it
/// should never leave the user three cycles short of done.
///
/// The prefix arithmetic itself lives in [`crate::todo::set_todo`],
/// shared with `outl-tauri-shared`'s reminder-completion command
/// rather than restated anywhere either client reads it from.
pub fn mark_done(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<(), ActionError> {
    let current = workspace.block_text(node).unwrap_or_default();
    let next = set_todo(&current, Some(TodoState::Done));
    edit_text(workspace, hlc, node, &next)
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
    ensure_live(workspace, node)?;
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
    fn mark_done_lands_from_every_starting_state() {
        // Unlike `toggle_cycles_through_states` above, this one must
        // not depend on how far from DONE the block started — the
        // whole point of a "finish this" gesture is that it never
        // leaves the user cycling further.
        for start in ["ship it", "TODO ship it", "DOING ship it"] {
            let (mut ws, hlc) = new_workspace();
            let n = append_block(&mut ws, &hlc, None, Some(start)).unwrap();
            mark_done(&mut ws, &hlc, n).unwrap();
            assert_eq!(
                ws.block_text(n).as_deref(),
                Some("DONE ship it"),
                "starting from {start:?}"
            );
        }
    }

    #[test]
    fn mark_done_on_an_already_done_block_is_a_true_no_op() {
        // Idempotent twice over: the text stays put, AND nothing gets
        // appended to the op log — `build_text_replace_update` comes
        // back empty, so there is no `Op::Edit` to apply in the first
        // place. Mattered for the reminders panel's re-tap case and
        // matters just as much for any double-press of the new chord.
        let (mut ws, hlc) = new_workspace();
        let n = append_block(&mut ws, &hlc, None, Some("ship it")).unwrap();
        mark_done(&mut ws, &hlc, n).unwrap();
        let log_len = ws.log().len();

        mark_done(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("DONE ship it"));
        assert_eq!(ws.log().len(), log_len);
    }

    #[test]
    fn mark_done_composes_with_a_quote_in_canonical_order() {
        // Same ordering convention [`toggle_quote`] enforces with
        // `toggle_todo`: task state before the quote marker, so
        // `split_todo` keeps finding the state on the way to the DTO.
        let (mut ws, hlc) = new_workspace();
        let n = append_block(&mut ws, &hlc, None, Some("> ship it")).unwrap();
        mark_done(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("DONE > ship it"));
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

    /// Invariant 6 keeps a deleted node's record, so `Tree::contains`
    /// answers yes for a block a peer just trashed — and every edit path
    /// used only that check. A keystroke then wrote into deleted content:
    /// the op landed, the block stayed in the trash, and the user saw
    /// nothing happen.
    #[test]
    fn a_trashed_block_refuses_edits() {
        let (mut ws, hlc) = new_workspace();
        let n = append_block(&mut ws, &hlc, None, Some("doomed")).unwrap();
        crate::block::delete(&mut ws, &hlc, n).unwrap();

        for res in [
            edit_text(&mut ws, &hlc, n, "after the delete"),
            toggle_todo(&mut ws, &hlc, n).map(|_| ()),
            toggle_quote(&mut ws, &hlc, n).map(|_| ()),
        ] {
            assert!(
                matches!(res, Err(ActionError::NotInTree(_))),
                "a trashed block must refuse the edit, got {res:?}"
            );
        }
    }

    /// Deleting a parent trashes the subtree, and every descendant keeps
    /// pointing at that parent rather than at the trash itself — so a
    /// one-level check would still call the child live.
    #[test]
    fn a_block_under_a_trashed_parent_refuses_edits_too() {
        let (mut ws, hlc) = new_workspace();
        let parent = append_block(&mut ws, &hlc, None, Some("parent")).unwrap();
        let child = append_block(&mut ws, &hlc, Some(parent), Some("child")).unwrap();
        crate::block::delete(&mut ws, &hlc, parent).unwrap();

        assert!(matches!(
            edit_text(&mut ws, &hlc, child, "still here?"),
            Err(ActionError::NotInTree(_))
        ));
    }
}
