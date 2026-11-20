//! What a cross-page backlink write computes its result **from**, which
//! is the one question the ahead-of-log gate turns on.
//!
//! Its own module beside [`crate::backlinks`] (*what counts* as a
//! backlink) and [`crate::backlinks_sort`] (*how a list is ordered*),
//! because this is a third, distinct question: a gesture aimed at a row
//! is about to write to a page the user is not looking at, and whether
//! that write may run against a `.md` the op log does not hold depends
//! on where the op's result comes from.
//!
//! **It is here because it was answered twice.** The same question,
//! under the same method name, was implemented in two crates — over
//! the TUI's three gestures and over the Tauri backend's seven concrete
//! row ops. A recorded reason does not make two copies safe; it only
//! documents which one will drift. And the drift here is not cosmetic:
//! the two halves gate the invariant-8 refusal, so a disagreement means
//! one client refuses an operation the other performs. Both defects that
//! shape produced were real — a refusal that discarded typed text in one
//! client, and a missing refusal on two read-modify-write ops in the
//! others — and the cost of the second is user text that walks on
//! invisibly in the log until `outl reconcile --ahead-of-log` drops it.
//!
//! **The gesture vocabularies stay per client, deliberately.** The TUI
//! has a typed-text gesture that is a separate command on the GUI side,
//! and the GUI's seven concrete ops have no TUI counterpart. So what is
//! shared is the classification and its answer; the mapping onto it is
//! each client's, and both map with an exhaustive `match` so a new
//! gesture cannot compile without declaring its shape.

/// Where a cross-page backlink write's result comes from.
///
/// Three shapes, because the ahead-of-log question has three different
/// answers and only one of them is "refuse". A client maps its own
/// gesture or op type onto this and asks
/// [`computed_from_the_logged_text`](Self::computed_from_the_logged_text);
/// it never re-derives the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossPageWrite {
    /// An `Op::Move` or an `Op::Create` naming a node: a delete, an
    /// indent, an outdent, a reorder, a new sibling.
    ///
    /// It reads no text, so a stale local `.md` cannot change what it
    /// does, and it converges through the op log whatever that file
    /// says. The frozen `.md` is the pre-existing condition rather than
    /// something the gesture caused (root `CLAUDE.md` invariant 11).
    Structural,
    /// A marker flip that is a text read-modify-write:
    /// [`crate::toggle_todo`] and [`crate::mark_done`] read the block's
    /// logged text, move a `TODO` / `DOING` / `DONE` prefix on it and
    /// write the result back.
    ///
    /// The row the gesture was aimed at was rendered from the `.md`, so
    /// on a page ahead of its log the op is computed from text the user
    /// never saw. The projection is then withheld (invariant 8), the row
    /// comes back unchanged, and the task state walks on invisibly in
    /// the log. **This is the one shape a later notice cannot repair**,
    /// which is why it is the one the gate refuses up front.
    MarkerFromLoggedText,
    /// The user supplied the whole text, having seen the text the op log
    /// holds — so the logged text *is* what they acted on and this
    /// question has nothing to protect.
    ///
    /// The seeding is what makes it this shape, not the fact that typing
    /// happened. The TUI's `enter_insert_backlink` seeds its buffer from
    /// `Workspace::block_text`, which is the op log. The GUI clients
    /// seed theirs from `link.source_block`, built from the `.md`, so
    /// their text edit is **not** this shape: the user typed over bytes
    /// the log does not hold, and
    /// `outl_tauri_shared::commands::backlink_edit::edit_backlink_block`
    /// therefore asks unconditionally rather than mapping onto anything
    /// here.
    TypedText,
}

impl CrossPageWrite {
    /// Whether this write's op is computed from the block's **logged**
    /// text, and so must not run against a `.md` the op log does not
    /// hold.
    ///
    /// Not "is it structural". That premise was the recorded reason for
    /// asking only the text edit, and it is false of both clients as
    /// stated: a marker flip is as much a text read-modify-write as a
    /// text edit, and a text edit seeded from the log is the one gesture
    /// the logged text cannot surprise.
    pub fn computed_from_the_logged_text(self) -> bool {
        match self {
            Self::MarkerFromLoggedText => true,
            Self::Structural | Self::TypedText => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CrossPageWrite::{self, MarkerFromLoggedText, Structural, TypedText};
    use crate::error::ActionError;
    use crate::{
        append_block, create_after, create_before, delete, edit_text, indent, mark_done, move_down,
        move_up, open_or_create_page, outdent, toggle_todo, PageKind,
    };
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::{ActorId, NodeId};
    use outl_core::op::Op;
    use outl_core::workspace::Workspace;

    /// The criterion as a table rather than left to the `match` arms.
    #[test]
    fn exactly_one_shape_is_computed_from_the_logged_text() {
        assert!(MarkerFromLoggedText.computed_from_the_logged_text());
        assert!(!Structural.computed_from_the_logged_text());
        assert!(!TypedText.computed_from_the_logged_text());
    }

    type Action = fn(&mut Workspace, &HlcGenerator, NodeId) -> Result<(), ActionError>;

    /// Run `act` on a page whose middle block's logged text is `prior`,
    /// and report `(did it write that block's text, what the text ended
    /// up as)`.
    ///
    /// Scoped to the target node, not "any `Op::Edit` in the delta": a
    /// create writes the **new** block's text, and counting that would
    /// read as a write computed from the anchor's logged text.
    ///
    /// The target sits in the **middle** of three siblings, one level
    /// down: `indent` needs a previous sibling, `outdent` a grandparent,
    /// and the two reorders somewhere to go in each direction.
    /// Otherwise an action with nowhere to land writes nothing and reads
    /// as structural for the wrong reason.
    fn run(prior: &str, act: Action) -> (bool, Option<String>) {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).expect("in-memory workspace");
        let page = open_or_create_page(&mut ws, &hlc, "notes", "Notes", PageKind::Page)
            .expect("a page to hang the blocks off");
        let parent = append_block(&mut ws, &hlc, Some(page), Some("parent")).expect("parent block");
        append_block(&mut ws, &hlc, Some(parent), Some("above")).expect("sibling above");
        let target = append_block(&mut ws, &hlc, Some(parent), Some(prior)).expect("target block");
        append_block(&mut ws, &hlc, Some(parent), Some("below")).expect("sibling below");

        let before = ws.log().len();
        act(&mut ws, &hlc, target).expect("the action lands on a healthy page");
        let wrote_text = ws
            .log()
            .iter()
            .skip(before)
            .any(|logged| matches!(logged.op, Op::Edit { node, .. } if node == target));
        (wrote_text, ws.block_text(target))
    }

    /// **The net that makes one shared answer correct for every client,
    /// measured against the actions behind it rather than asserted.**
    ///
    /// "Computed from the logged text" is a property of the backend
    /// action a gesture runs, and both clients run these same functions
    /// — so this is where a wrong verdict is wrong for both of them at
    /// once. An `Op::Edit` is not enough on its own: a text edit writing
    /// the user's own string emits one too, and its result does not
    /// depend on what the log held. So the measurement is both halves —
    /// *did it write text*, and *did what it wrote depend on the text
    /// that was there* — run twice over two different priors.
    ///
    /// Flip the verdict in `computed_from_the_logged_text`, or teach a
    /// move to rewrite text, and this fails.
    #[test]
    fn every_shape_verdict_matches_what_the_action_behind_it_reads() {
        let cases: &[(CrossPageWrite, &str, Action)] = &[
            (MarkerFromLoggedText, "toggle_todo", toggle_todo),
            (MarkerFromLoggedText, "mark_done", mark_done),
            (Structural, "delete", delete),
            (Structural, "indent", indent),
            (Structural, "outdent", outdent),
            (Structural, "move_up", move_up),
            (Structural, "move_down", move_down),
            // `o` / `O` on a backlink row: an `Op::Create` naming the
            // anchor, plus the new block's own text. Neither reads the
            // anchor's logged text, which is what the scoping in `run`
            // is for.
            (Structural, "create_after", |ws, hlc, node| {
                create_after(ws, hlc, node, Some("new")).map(|_| ())
            }),
            (Structural, "create_before", |ws, hlc, node| {
                create_before(ws, hlc, node, Some("new")).map(|_| ())
            }),
            (TypedText, "edit_text", |ws, hlc, node| {
                edit_text(ws, hlc, node, "what the user typed")
            }),
        ];

        for (shape, name, act) in cases {
            let (wrote_alpha, alpha) = run("alpha", *act);
            let (wrote_beta, beta) = run("beta", *act);
            assert_eq!(
                wrote_alpha, wrote_beta,
                "{name}: the action is not deterministic about whether it writes text"
            );
            let reads_the_logged_text = wrote_alpha && alpha != beta;
            assert_eq!(
                shape.computed_from_the_logged_text(),
                reads_the_logged_text,
                "{name} is mapped to {shape:?}, but it {} the logged text \
                 (wrote text: {wrote_alpha}, {alpha:?} vs {beta:?})",
                if reads_the_logged_text {
                    "does read"
                } else {
                    "does not read"
                },
            );
        }
    }
}
