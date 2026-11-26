//! [`BacklinkRowOp`]: which mutation a backlink row asks for, the shape
//! of write each one is, and the write itself.
//!
//! A child of `backlink_edit` rather than more of it, for the size
//! ratchet: the command bodies and their refusal ordering stay there, the
//! vocabulary and its exhaustive `match`es live here.

use outl_actions::{
    delete, indent, mark_done, move_down, move_up, outdent, set_block_collapsed, toggle_todo,
    ActionError, CrossPageWrite,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use serde::{Deserialize, Serialize};

/// Which cross-page mutation a backlink row is asking for.
///
/// An exhaustive `match` in [`backlink_row_op`](super::backlink_row_op) is behind it, so a tenth
/// gesture on a backlink row cannot compile without saying what it does
/// — and therefore cannot ship printing a raw [`ActionError`] the way
/// these eight did.
///
/// A create is **not** here. It replies with a new block's id
/// ([`create_backlink_sibling`](super::create_backlink_sibling)), and an `Option<String>` that is
/// meaningless for nine of eleven variants is a claim the type keeps
/// making.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BacklinkRowOp {
    /// Cycle the citing block's TODO / DOING / DONE marker (`Ctrl+T`).
    ToggleTodo,
    /// Set it DONE outright (`g D`).
    MarkDone,
    /// Move it to the trash root (`dd`).
    Delete,
    /// Make it the last child of its previous sibling (`Tab`).
    Indent,
    /// Move it up one level (`Shift+Tab`).
    Outdent,
    /// Swap it with its previous sibling (`Alt+↑`).
    MoveUp,
    /// Swap it with its next sibling (`Alt+↓`).
    MoveDown,
    /// Fold it on its source page: `Op::SetCollapsed { value: true }`.
    ///
    /// Two explicit variants rather than a toggle, because a toggle
    /// aimed at a stale view flips the wrong way, and two devices
    /// toggling the same block cancel each other out.
    Collapse,
    /// Unfold it on its source page: `Op::SetCollapsed { value: false }`.
    Expand,
}

impl BacklinkRowOp {
    /// Which [`CrossPageWrite`] shape this op is — the mapping, and
    /// nothing more.
    ///
    /// The **answer** (whether a shape's op is computed from the logged
    /// text, and so is the one the ahead-of-log gate refuses) belongs to
    /// `outl-actions`, because the TUI gates on it too. It used to be a
    /// second `computed_from_the_logged_text` here, under the same name,
    /// over these seven ops — and a drift between the two copies meant
    /// one client refusing an operation the other performed, which costs
    /// user text.
    ///
    /// Why each arm, since the mapping is the half that can still be
    /// wrong:
    ///
    /// - `ToggleTodo` / `MarkDone` are **not** structural. `toggle_todo`
    ///   reads `workspace.block_text`, runs `cycle_todo` on it and
    ///   writes the result back as an `Op::Edit`; `mark_done` does the
    ///   same through `set_todo` + `edit_text`. Both therefore compute
    ///   an op from the **logged** text while the row the user tapped
    ///   rendered the text on **disk**. On a frozen page the commit's
    ///   projection is withheld (invariant 8), the frontend refetches
    ///   `pageBacklinks` from disk, and the row comes back unchanged:
    ///   the tap looks like it did nothing while the task state walks
    ///   `TODO → DOING → DONE` in the log, and
    ///   `outl reconcile --ahead-of-log` later drops all of it.
    /// - The other five are `Op::Move`s naming a node. They read no
    ///   text, so a stale local `.md` cannot change what they do, and
    ///   they converge through the op log whatever that file says — the
    ///   frozen `.md` is the pre-existing condition rather than
    ///   something the gesture caused (root `CLAUDE.md` invariant 11).
    ///   What they still lack is a *notice*, which is the open item
    ///   recorded in
    ///   [`docs/client-backlinks.md`](https://github.com/outlmd/outl/blob/main/docs/client-backlinks.md),
    ///   not a reason to withhold their op.
    /// - `Collapse` / `Expand` are `Op::SetCollapsed`s naming a node. They
    ///   read no text either, and a fold is not even part of the `.md`,
    ///   so a frozen page cannot make one land differently.
    ///
    /// `CrossPageWrite::TypedText` has no arm here on purpose: a text
    /// edit is [`edit_backlink_block`](super::edit_backlink_block), not a row op, and these clients
    /// seed its draft from the `.md` rather than from the log — so it is
    /// not that shape and asks unconditionally. That variant's own doc
    /// owns the asymmetry.
    ///
    /// Exhaustive on purpose: a new variant cannot compile without
    /// declaring its shape.
    pub(super) fn shape(self) -> CrossPageWrite {
        match self {
            Self::ToggleTodo | Self::MarkDone => CrossPageWrite::MarkerFromLoggedText,
            Self::Delete
            | Self::Indent
            | Self::Outdent
            | Self::MoveUp
            | Self::MoveDown
            | Self::Collapse
            | Self::Expand => CrossPageWrite::Structural,
        }
    }

    /// Whether this op changes what the page's `.md` + sidecar say.
    ///
    /// A fold does not: `Op::SetCollapsed` lives only in the op log, and
    /// the outline's own `set_block_collapsed` skips the commit pipeline
    /// for exactly that reason (reprojecting would re-upload an
    /// unchanged file over a file transport on every fold). The backlink
    /// index is built from the `.md`, so a fold leaves nothing in it to
    /// refresh either: `page_backlinks` overlays the flag from the tree
    /// at lookup time.
    pub(super) fn reprojects(self) -> bool {
        match self {
            Self::Collapse | Self::Expand => false,
            Self::ToggleTodo
            | Self::MarkDone
            | Self::Delete
            | Self::Indent
            | Self::Outdent
            | Self::MoveUp
            | Self::MoveDown => true,
        }
    }

    /// The write itself. The only place a row op names its action.
    ///
    /// The no-op arms `block.rs` swallows are swallowed here too
    /// (`NoPreviousSibling`, `AlreadyAtRoot`).
    pub(super) fn write(
        self,
        ws: &mut Workspace,
        hlc: &HlcGenerator,
        node: NodeId,
    ) -> Result<(), ActionError> {
        match self {
            Self::ToggleTodo => toggle_todo(ws, hlc, node),
            Self::MarkDone => mark_done(ws, hlc, node),
            Self::Delete => delete(ws, hlc, node),
            Self::Indent => match indent(ws, hlc, node) {
                Err(ActionError::NoPreviousSibling(_)) => Ok(()),
                other => other,
            },
            Self::Outdent => match outdent(ws, hlc, node) {
                Err(ActionError::AlreadyAtRoot(_)) => Ok(()),
                other => other,
            },
            Self::MoveUp => move_up(ws, hlc, node),
            Self::MoveDown => move_down(ws, hlc, node),
            Self::Collapse => set_block_collapsed(ws, hlc, node, true).map(drop),
            Self::Expand => set_block_collapsed(ws, hlc, node, false).map(drop),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BacklinkRowOp, CrossPageWrite};

    /// **This client's half of the shared criterion: the mapping.**
    ///
    /// The shapes and the verdict are `outl-actions`' (and
    /// `backlinks_write`'s
    /// `every_shape_verdict_matches_what_the_action_behind_it_reads` is
    /// what keeps the verdict honest against the actions both clients
    /// call). What can still go wrong here is an op mapped onto the
    /// wrong shape, which is the only way these clients and the TUI can
    /// now disagree about an operation — so the table is the mapping,
    /// with the verdict it produces beside it.
    ///
    /// `tests/backlink_row_ops.rs` asserts the same split on the op log
    /// and the `.md` for every op. This is the cheap, direct net at the
    /// mapping site; that one is the behavioural one.
    #[test]
    fn each_row_op_declares_the_shape_of_the_write_it_performs() {
        use CrossPageWrite::{MarkerFromLoggedText, Structural};
        // (op, shape, ahead-of-log gated, goes through the commit pipeline)
        for (op, shape, gated, reprojects) in [
            (BacklinkRowOp::ToggleTodo, MarkerFromLoggedText, true, true),
            (BacklinkRowOp::MarkDone, MarkerFromLoggedText, true, true),
            (BacklinkRowOp::Delete, Structural, false, true),
            (BacklinkRowOp::Indent, Structural, false, true),
            (BacklinkRowOp::Outdent, Structural, false, true),
            (BacklinkRowOp::MoveUp, Structural, false, true),
            (BacklinkRowOp::MoveDown, Structural, false, true),
            // A fold names a node and is not part of the `.md`: the
            // outline's `set_block_collapsed` shape, no reprojection.
            (BacklinkRowOp::Collapse, Structural, false, false),
            (BacklinkRowOp::Expand, Structural, false, false),
        ] {
            assert_eq!(op.shape(), shape, "{op:?} is mapped wrong");
            assert_eq!(
                op.shape().computed_from_the_logged_text(),
                gated,
                "{op:?}: the gate would change for this op",
            );
            assert_eq!(
                op.reprojects(),
                reprojects,
                "{op:?}: the commit path would change for this op",
            );
        }
    }
}
