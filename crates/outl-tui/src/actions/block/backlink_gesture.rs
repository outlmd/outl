//! Which gesture is committing a cross-page mutation, and the two
//! things that answer decides.
//!
//! Its own module rather than a pair of `bool`s at
//! [`App::commit_backlink_mutation`]'s signature, because the two
//! questions below are **per gesture** and were both got wrong by being
//! answered once, for all ten of them, from the shape of the client
//! instead:
//!
//! - the ahead-of-log gate was asked for every gesture in this client
//!   and for one in the GUI clients, on the premise that "a structural
//!   op names an identity, where a text edit applies to the logged
//!   text". That premise is true of neither client as stated — see
//!   [`BacklinkGesture::computed_from_the_logged_text`];
//! - the refusal catalog has a "nothing changed" half and a "your text
//!   could not be saved" half, and the second was written and then
//!   reached by nothing, so a refusal that dropped a consumed Insert
//!   buffer reported a *file* condition instead.
//!
//! [`App::commit_backlink_mutation`]: crate::state::App

use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

/// Which gesture is committing — the two things
/// `App::commit_backlink_mutation` cannot read off the closure it is
/// handed, and each one decides something.
///
/// **Is the op's result computed from the block's *logged* text?** That
/// is the whole of what the ahead-of-log gate turns on. It is not "is
/// this structural": a marker flip is as much a text read-modify-write
/// as a text edit, and in this client the text edit is the one gesture
/// whose result the logged text cannot surprise.
///
/// **Were the user's keystrokes already consumed?** That decides which
/// half of `outl_actions::refusal` is the true sentence. A gesture that
/// typed nothing must not claim a loss; one that typed something must
/// not hide one.
///
/// A variant rather than two `bool`s so each of the eight call sites
/// has to say which gesture it is, and a ninth cannot reach the commit
/// without answering both questions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BacklinkGesture {
    /// `d d`, `Tab`, `Shift+Tab`, `Alt+↑`, `Alt+↓`, `o`, `O` — an
    /// `Op::Move` or an `Op::Create` naming a node. It reads no text,
    /// so a stale local `.md` cannot change what it does; the stale
    /// `.md` is the pre-existing condition rather than something the
    /// gesture caused, and the op converges through the log either way.
    Structural,
    /// `Ctrl+T` / `g D` — `outl_actions::toggle_todo` and
    /// `outl_actions::mark_done` read `block_text`, move a marker and
    /// write the result back. The row the chord was aimed at was
    /// rendered from the `.md` (the backlink index is built from disk),
    /// so on a page ahead of its log the op is computed from text the
    /// user never saw. Nothing was typed.
    MarkerFromLoggedText,
    /// `Esc` out of Insert on a backlink row. The buffer is already
    /// gone — `commit_insert` replaces the mode before it calls — and
    /// the text it carries is what `enter_insert_backlink` seeded from
    /// the **op log** and what the row painted while the user typed. So
    /// the logged text is the text they acted on, and the ahead-of-log
    /// question has nothing to protect here; the catalog's text half is
    /// the true one if anything else refuses.
    TypedText,
}

impl BacklinkGesture {
    /// Whether this gesture's op is computed from the block's logged
    /// text, and so must not run against a `.md` the log does not hold.
    pub(super) fn computed_from_the_logged_text(self) -> bool {
        match self {
            Self::MarkerFromLoggedText => true,
            Self::Structural | Self::TypedText => false,
        }
    }

    /// The refusal catalog's half that is true for this gesture — the
    /// words are `outl_actions::refusal`'s, the choice between them is
    /// this type's.
    pub(super) fn stale_block_refusal(self, workspace: &Workspace, block: NodeId) -> &'static str {
        match self {
            Self::TypedText => {
                outl_actions::refusal::stale_backlink_block_refusal(workspace, block)
            }
            Self::Structural | Self::MarkerFromLoggedText => {
                outl_actions::refusal::stale_backlink_block_unchanged_refusal(workspace, block)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BacklinkGesture::{MarkerFromLoggedText, Structural, TypedText};

    /// The criterion, stated as a table rather than left to the three
    /// `match` arms: exactly one gesture reads the logged text, and the
    /// text edit is deliberately **not** it in this client.
    #[test]
    fn only_the_marker_chords_are_computed_from_the_logged_text() {
        assert!(MarkerFromLoggedText.computed_from_the_logged_text());
        assert!(!TypedText.computed_from_the_logged_text());
        assert!(!Structural.computed_from_the_logged_text());
    }

    /// The two costs, and the half that had no caller: only a gesture
    /// that consumed a buffer may say the user's text was not saved.
    #[test]
    fn only_the_typed_text_gesture_claims_a_lost_edit() {
        use outl_core::hlc::HlcGenerator;
        use outl_core::id::ActorId;
        use outl_core::workspace::Workspace;

        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let page = outl_actions::open_or_create_page(
            &mut ws,
            &hlc,
            "notes",
            "Notes",
            outl_actions::PageKind::Page,
        )
        .unwrap();
        let block = outl_actions::append_block(&mut ws, &hlc, Some(page), Some("x")).unwrap();
        outl_actions::delete(&mut ws, &hlc, block).unwrap();

        assert_eq!(
            TypedText.stale_block_refusal(&ws, block),
            outl_actions::refusal::stale_backlink_block_text(true),
        );
        for chord in [Structural, MarkerFromLoggedText] {
            assert_eq!(
                chord.stale_block_refusal(&ws, block),
                outl_actions::refusal::stale_backlink_block_unchanged_text(true),
                "{chord:?} typed nothing, so it must not name a loss",
            );
        }
    }

    /// **Requirement 1 as a net rather than as an argument.**
    ///
    /// `commit_insert` consumes the Insert buffer on its first line, so
    /// every path through `commit_backlink_mutation` that answers
    /// `false` is a path where the user's keystrokes are already gone.
    /// Each one is therefore driven here, with the gesture that carries
    /// a buffer, and the status line has to end in the catalog's cost
    /// clause.
    ///
    /// Both of these are unreachable from a keystroke today — the entry
    /// gate in `enter_insert_backlink` and `pending_reload` are why —
    /// and that argument is exactly what was offered for the one
    /// refusal that *was* reachable. So the cost is asserted instead of
    /// reasoned about.
    #[test]
    fn every_refusal_a_consumed_buffer_can_reach_says_the_text_was_not_saved() {
        use crate::actions::block::backlink_target::BacklinkTarget;
        use crate::actions::block::test_support::workspace_with_a_backlink;
        use outl_core::id::NodeId;

        for (path, gesture, expected_cost) in [
            (
                "a block the tree no longer holds",
                TypedText,
                "your text could not be saved.",
            ),
            (
                "a row whose page is not a page",
                TypedText,
                "your text could not be saved.",
            ),
            // The mirror, so the two halves of the catalog stay honest:
            // a chord typed nothing and must not claim a loss.
            (
                "a block the tree no longer holds",
                Structural,
                "nothing changed.",
            ),
            (
                "a row whose page is not a page",
                Structural,
                "nothing changed.",
            ),
        ] {
            let (mut app, _d, _source) = workspace_with_a_backlink();
            let target = app
                .focused_backlink_target()
                .expect("a healthy backlink row resolves");
            let hlc = app.hlc.clone();
            let target = if path.starts_with("a block") {
                outl_actions::delete(&mut app.workspace, &hlc, target.block)
                    .expect("a peer deletes the citing block");
                target
            } else {
                BacklinkTarget {
                    page: NodeId::new(),
                    block: target.block,
                }
            };

            let landed = app.commit_backlink_mutation(target, gesture, |ws, hlc, node| {
                outl_actions::edit_text(ws, hlc, node, "what the user typed")
            });

            assert!(!landed, "{path} / {gesture:?}: setup must refuse");
            assert!(
                app.status.ends_with(expected_cost),
                "{path} / {gesture:?}: the status line has to end in the \
                 catalog's cost clause, got {:?}",
                app.status
            );
        }
    }
}
