//! Split one block into two at a character offset.
//!
//! `split_block` is the workspace-grounded "press Enter in the middle of
//! a block" operation every GUI client (desktop, mobile) and the CLI/MCP
//! surface wraps. The TUI has its own in-flight-AST path (it slices the
//! edit buffer before the text is ever parsed back to a `Workspace`, see
//! `outl_md::outline_ops`), the same way `create_after` here has the TUI
//! twin `insert_sibling_after` — two layers, not two implementations of
//! one concept.

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::text::split_at_char;

use super::edit::edit_text;
use super::ensure_in_tree;
use super::siblings::create_after;

/// Split `node` at `char_offset`: the text up to the offset stays in
/// `node`, the text from the offset onward moves into a **new sibling
/// created immediately after** `node`. Returns the new sibling's id.
///
/// `char_offset` is a **character** index (not a byte offset), clamped
/// to the block's length — the caller passes the editor caret position.
///
/// Semantics chosen to match Roam / Logseq:
///
/// - The new block is a *sibling*, so `node`'s existing children stay
///   with `node` (the head), and the tail block starts childless.
/// - `char_offset == 0` leaves `node` empty and moves the whole text
///   into the sibling — i.e. "open an empty block above". The caller
///   parks the caret at the start of the returned sibling.
/// - `char_offset >= len` leaves `node` untouched and creates an empty
///   sibling below — the plain "Enter at end of line" gesture.
///
/// The ops are emitted through `Workspace::apply`, so the log stays the
/// source of truth: an `Op::Edit` truncating `node` to the head (skipped
/// when the head equals the current text, e.g. offset at end), an
/// `Op::Create` for the empty sibling, then an `Op::Edit` writing the
/// tail into it.
///
/// The sibling is created empty and the tail is written with
/// [`edit_text`] rather than passed to [`create_after`] as initial text,
/// because block creation **trims** its initial text — which would drop
/// a leading space when you split mid-sentence (`"foo bar"` at offset 3
/// must yield a tail of `" bar"`, not `"bar"`). `edit_text` writes
/// verbatim.
pub fn split_block(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
    char_offset: usize,
) -> Result<NodeId, ActionError> {
    ensure_in_tree(workspace, node)?;

    let text = workspace.block_text(node).unwrap_or_default();
    let (head, tail) = split_at_char(&text, char_offset);

    // One keystroke (Enter mid-text) emits up to three ops: truncate the
    // head, create the sibling, write the tail. Batch them so the split
    // flushes once per destination instead of fsyncing three times.
    let mut batch = workspace.begin_batch();

    // Truncate the current block to the head. `edit_text` is a no-op
    // when the text is unchanged (offset at end), so this is cheap in
    // the common "Enter at end of line" case.
    edit_text(&mut batch, hlc, node, &head)?;

    // The tail becomes a fresh sibling right after `node`, created empty
    // so `create_after`'s trim can't eat a leading space; `edit_text`
    // then writes the tail verbatim. `create_after` gives the sibling no
    // children, so `node`'s subtree stays with the head.
    let sibling = create_after(&mut batch, hlc, node, None)?;
    edit_text(&mut batch, hlc, sibling, &tail)?;
    batch.commit()?;
    Ok(sibling)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{append_block, create_under};
    use crate::tree::children_of;
    use outl_core::id::ActorId;

    fn new_workspace() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    fn sibling_texts(ws: &Workspace, parent: NodeId) -> Vec<String> {
        children_of(ws, parent)
            .into_iter()
            .map(|(id, _)| ws.block_text(id).unwrap_or_default())
            .collect()
    }

    #[test]
    fn split_in_the_middle_produces_two_siblings() {
        let (mut ws, hlc) = new_workspace();
        let n = append_block(&mut ws, &hlc, None, Some("hello world")).unwrap();

        let tail = split_block(&mut ws, &hlc, n, 5).unwrap();

        assert_eq!(ws.block_text(n).as_deref(), Some("hello"));
        assert_eq!(ws.block_text(tail).as_deref(), Some(" world"));
        // Tail sorts immediately after the head under the same parent.
        assert_eq!(
            sibling_texts(&ws, NodeId::root()),
            vec!["hello".to_string(), " world".to_string()]
        );
    }

    #[test]
    fn split_at_end_creates_an_empty_sibling_below() {
        let (mut ws, hlc) = new_workspace();
        let n = append_block(&mut ws, &hlc, None, Some("keep me")).unwrap();

        let tail = split_block(&mut ws, &hlc, n, 7).unwrap();

        assert_eq!(ws.block_text(n).as_deref(), Some("keep me"));
        // An untouched block carries no text at all (not `Some("")`).
        assert_eq!(ws.block_text(tail).unwrap_or_default(), "");
    }

    #[test]
    fn split_past_end_clamps_and_still_creates_empty_sibling() {
        let (mut ws, hlc) = new_workspace();
        let n = append_block(&mut ws, &hlc, None, Some("abc")).unwrap();

        let tail = split_block(&mut ws, &hlc, n, 999).unwrap();

        assert_eq!(ws.block_text(n).as_deref(), Some("abc"));
        assert_eq!(ws.block_text(tail).unwrap_or_default(), "");
    }

    #[test]
    fn split_at_start_empties_the_head_and_moves_text_below() {
        let (mut ws, hlc) = new_workspace();
        let n = append_block(&mut ws, &hlc, None, Some("moved")).unwrap();

        let tail = split_block(&mut ws, &hlc, n, 0).unwrap();

        assert_eq!(ws.block_text(n).as_deref(), Some(""));
        assert_eq!(ws.block_text(tail).as_deref(), Some("moved"));
    }

    #[test]
    fn children_stay_with_the_head_block() {
        let (mut ws, hlc) = new_workspace();
        let parent = append_block(&mut ws, &hlc, None, Some("parent text")).unwrap();
        let child = create_under(&mut ws, &hlc, parent, Some("child")).unwrap();

        let tail = split_block(&mut ws, &hlc, parent, 6).unwrap();

        assert_eq!(ws.block_text(parent).as_deref(), Some("parent"));
        assert_eq!(ws.block_text(tail).as_deref(), Some(" text"));
        // The child rode along with the head, not the tail.
        let head_children: Vec<NodeId> = children_of(&ws, parent)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(head_children, vec![child]);
        assert!(children_of(&ws, tail).is_empty());
    }

    #[test]
    fn split_respects_utf8_char_boundaries() {
        let (mut ws, hlc) = new_workspace();
        // "café" — the 'é' is multi-byte; offset 3 is a char index, not
        // a byte index, so the head must keep the whole "caf".
        let n = append_block(&mut ws, &hlc, None, Some("café au lait")).unwrap();

        let tail = split_block(&mut ws, &hlc, n, 4).unwrap();

        assert_eq!(ws.block_text(n).as_deref(), Some("café"));
        assert_eq!(ws.block_text(tail).as_deref(), Some(" au lait"));
    }

    #[test]
    fn split_rejects_a_node_not_in_the_tree() {
        let (mut ws, hlc) = new_workspace();
        // A fresh id that was never created — deleting a node only moves
        // it to the trash root (still `contains`-true), so we use an id
        // the tree has genuinely never seen.
        let never = NodeId::new();

        let err = split_block(&mut ws, &hlc, never, 1).unwrap_err();
        assert!(matches!(err, ActionError::NotInTree(_)));
    }
}
