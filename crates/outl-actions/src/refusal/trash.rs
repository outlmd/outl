//! What a user can *do* about a trash refusal, in the words of the
//! surface that refused — the second half of what [`super`] owns.
//!
//! [`super::refusal_support`] answers "does this surface tell the user";
//! for the trash the answer is yes everywhere, because all five print
//! `trash::refusal_for`'s sentence. The fact that was missing is the one
//! below: **is there a way out from this client at all.**
//!
//! ## Why the per-surface fact hangs off the way out, not off the refusal
//!
//! The obvious shape is a `(Refusal, Surface)` table like
//! [`super::refusal_support`]'s. It is the wrong axis here. Four trash
//! refusals share **three** ways out — "check whether this op log is
//! complete" answers two of them — so a per-refusal table would declare
//! that answer twice, and two copies of one sentence is the drift
//! invariant 12 exists to stop, reintroduced by the mechanism meant to
//! prevent it.
//!
//! So [`Remedy`] is the closed set of ways out, and
//! [`remedy_action`] is one exhaustive `match` over
//! ([`Remedy`], [`super::Surface`]). A new refusal that reuses an
//! existing remedy needs **no** new declaration and cannot describe it
//! differently; a new *remedy* does not compile until all five surfaces
//! say what they do about it.
//!
//! ## What `ActionError`'s `Display` keeps
//!
//! The diagnosis, surface-blind: *what* is wrong, with no command and no
//! instruction. That is true on every surface, which is exactly what the
//! strings it replaced were not —
//! [`crate::ActionError::TrashParentMissing`] told a desktop user to run
//! `outl doctor` and [`crate::ActionError::TrashPageRestoreUnsupported`]
//! told them to run `outl block tree <ULID>`, in a panel that renders
//! `e.to_string()` verbatim and shows no block ids (invariant 2).
//!
//! ## Who words a refusal, and who does not
//!
//! [`crate::trash::list`] takes a [`super::Surface`] and words every
//! row's `refusal` with it, so a caller cannot get a sentence without
//! saying who it is for. A **mutation's** error is deliberately left as
//! the bare diagnosis for the surfaces that re-list after a failed
//! restore — the desktop panel and the TUI overlay both refetch on
//! purpose, so that the listing (the single owner of the verdict) gets
//! the last word, and the action arrives in the row the user is looking
//! at rather than twice. The one-shot surfaces have no listing to wait
//! for, so `outl trash restore` and `outl_trash_restore` word their own
//! error through [`trash_refusal_text`].

use crate::error::ActionError;

use super::Surface;

/// A way out of a trash refusal — the thing the user would have to do.
///
/// Borrowed from the [`ActionError`] rather than owned: a remedy is
/// read, phrased and dropped inside one call, and copying two ULIDs per
/// listed row to achieve that would be a cost with nothing buying it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remedy<'a> {
    /// Move a block that currently sits **inside the trash** back out,
    /// so the refused row has somewhere to land.
    ///
    /// The one remedy no outline can perform: the block is inside the
    /// subtree being restored, so no live view shows it, and a GUI has
    /// no block id to offer instead.
    MoveOutOfTrash {
        /// The block that has to move, as `outl block move` takes it.
        block: &'a str,
    },
    /// Find out whether this workspace's op log is complete — the
    /// question behind every shape of
    /// [`ActionError::TrashOriginUnknown`] and behind
    /// [`ActionError::TrashParentMissing`].
    CheckWorkspaceHealth,
    /// Read a trashed subtree's content straight out of the op log,
    /// for a deletion that cannot be restored in place.
    ReadTrashedSubtree {
        /// The root of the subtree to print.
        block: &'a str,
    },
}

/// The way out of `err`, or `None` when its `Display` is the whole
/// story.
///
/// **Non-exhaustive over [`ActionError`] on purpose**, and that is the
/// one weakness of this design worth naming: forty-odd variants reach
/// here and only the trash's have a remedy, so a new trash refusal
/// added without a line here silently gets no advice. The compiler
/// cannot catch that; `every_trash_refusal_offers_a_way_out` can, and
/// each variant's own doc in `error.rs` points back at the `Remedy` it
/// uses.
///
/// Degrading to "no advice" rather than to *wrong* advice is the part
/// that matters: a surface with no remedy prints the honest diagnosis,
/// which is what every one of them printed before this existed, with
/// the false instruction removed.
pub fn remedy_for(err: &ActionError) -> Option<Remedy<'_>> {
    match err {
        ActionError::TrashOriginInsideSubtree { parent, .. } => {
            Some(Remedy::MoveOutOfTrash { block: parent })
        }
        ActionError::TrashOriginUnknown { .. } | ActionError::TrashParentMissing { .. } => {
            Some(Remedy::CheckWorkspaceHealth)
        }
        ActionError::TrashPageRestoreUnsupported { node, .. } => {
            Some(Remedy::ReadTrashedSubtree { block: node })
        }
        // `TrashParentTrashed` is deliberately absent. Its instruction —
        // restore the ancestor first — is performable on every surface,
        // because that ancestor is its own row in the same listing, so
        // there is no per-surface fact to record. It names the parent as
        // a ULID, which no GUI row can be matched against; that is an
        // invariant-2 question about the *diagnosis*, the same one
        // `TrashParentMissing` and `TrashOriginInsideSubtree` still
        // carry, and not the "is there a way out here" question this
        // module answers.
        _ => None,
    }
}

/// What one surface tells the user to do about a [`Remedy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemedyAction {
    /// The sentence. Ends in a colon when [`Self::command`] follows it,
    /// and stands on its own when nothing does.
    pub sentence: &'static str,
    /// The command or tool call [`Self::sentence`] hands off to, or
    /// `None` when this surface has nothing to run.
    pub command: Option<String>,
}

/// What `surface` can do about `remedy`.
///
/// **Exhaustive on purpose**, the same mechanism as
/// [`super::refusal_support`] and `outl_shortcuts::support`: a new
/// [`Remedy`] does not compile until every surface has declared whether
/// it can offer it, and a surface that cannot says so rather than
/// prescribing a step it cannot perform.
pub fn remedy_action(surface: Surface, remedy: Remedy<'_>) -> RemedyAction {
    match remedy {
        Remedy::MoveOutOfTrash { block } => match surface {
            // `move_under` guards with `ensure_in_tree`, not
            // `ensure_live`, precisely so a node inside the trash is a
            // legal source (`block/moves.rs`). So this genuinely works
            // here — and only here.
            Surface::Cli => RemedyAction {
                sentence: "move it back out from under this deletion first, then restore this one:",
                command: Some(format!("outl block move {block} --parent <live-block-id>")),
            },
            Surface::Mcp => RemedyAction {
                sentence: "move it back out from under this deletion first, then restore this one:",
                command: Some(format!(
                    "outl_block_move {{ \"id\": \"{block}\", \"parent\": \"<live-block-id>\" }}"
                )),
            },
            // The honest answer, and the reason this module exists. The
            // block that has to move is *inside* the deletion, so no
            // outline draws it, and invariant 2 keeps block ids off
            // these surfaces — so there is nothing to look up, nothing
            // to drag, and no command to hand over. Naming one anyway is
            // a to-do the user cannot do.
            Surface::Tui | Surface::Desktop | Surface::Mobile => RemedyAction {
                sentence: "There's nothing to do about this one from here: the block it has to go \
                           back under is inside this deletion, so nothing here can reach it. The \
                           row stays listed, so nothing is lost.",
                command: None,
            },
        },
        Remedy::CheckWorkspaceHealth => match surface {
            Surface::Mcp => RemedyAction {
                sentence: "check whether ops for it are missing:",
                command: Some("outl_workspace_doctor".to_string()),
            },
            // The TUI and the desktop are both sitting next to a shell,
            // which is the same reading `ahead_of_log_notice` already
            // takes of these three columns; this command carries no
            // block id, so invariant 2 has nothing to say about it.
            Surface::Cli | Surface::Tui | Surface::Desktop => RemedyAction {
                sentence:
                    "check whether ops for it are missing — run this in the workspace folder:",
                command: Some("outl doctor".to_string()),
            },
            // No `outl` binary on a phone, same as the ahead-of-log
            // notice's one legitimate per-surface difference.
            Surface::Mobile => RemedyAction {
                sentence: "Open this workspace on your computer to check whether ops for it are \
                           missing — there's nothing to run from the phone.",
                command: None,
            },
        },
        // The diagnosis already says the content is still in the op log,
        // which is the part that is true everywhere. Repeating it here
        // read as "still in the op log — still in the op log, and this
        // prints it", caught by
        // `a_listing_carries_the_same_refusal_restore_would_return`. So
        // this half says only how to get at it.
        Remedy::ReadTrashedSubtree { block } => match surface {
            Surface::Cli => RemedyAction {
                sentence: "this prints it:",
                command: Some(format!("outl block tree {block}")),
            },
            Surface::Mcp => RemedyAction {
                sentence: "this prints it:",
                command: Some(format!("outl_block_tree {{ \"id\": \"{block}\" }}")),
            },
            // Reading it needs the block id, which these surfaces do not
            // show (invariant 2). A deleted page cannot be reopened by
            // name either — on the TUI that path is what revived the
            // page and turned one trash row into one per child — so
            // there is no gesture to name.
            Surface::Tui | Surface::Desktop | Surface::Mobile => RemedyAction {
                sentence: "There's no way to read it back from here yet, so the row stays listed \
                           to keep pointing at it.",
                command: None,
            },
        },
    }
}

/// A trash refusal as one sentence for `surface`: the diagnosis, plus
/// the way out that surface actually has.
///
/// **The join lives here**, for the reason
/// [`super::AheadOfLogNotice::one_line`] gives: a caller that joins the
/// parts itself is a client writing wording, and the punctuation is
/// exactly where the seam shows. A `sentence` that hands off to a
/// command ends in a colon and is a clause; one that stands alone is a
/// sentence of its own and needs the preceding clause closed first.
pub fn trash_refusal_text(surface: Surface, err: &ActionError) -> String {
    let Some(remedy) = remedy_for(err) else {
        return err.to_string();
    };
    let action = remedy_action(surface, remedy);
    match action.command {
        Some(command) => format!("{err} — {} `{command}`", action.sentence),
        None => format!("{err}. {}", action.sentence),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every trash refusal `trash::refusal_for` can hand back, built by
    /// hand because `ActionError` cannot be iterated.
    ///
    /// This list is the forcing function [`remedy_for`]'s `_` arm costs
    /// us: a new trash variant added without a row here *and* without a
    /// line in `remedy_for` passes, so the two have to be added
    /// together. `NotTrashed` is in the list and deliberately has no
    /// remedy — nothing is wrong, the row is simply not in the trash.
    fn every_trash_refusal() -> Vec<(ActionError, bool)> {
        let node = "01JQ0000000000000000000001".to_string();
        let parent = "01JQ0000000000000000000002".to_string();
        vec![
            (ActionError::NotTrashed(node.clone()), false),
            (
                ActionError::TrashParentTrashed {
                    node: node.clone(),
                    parent: parent.clone(),
                },
                false,
            ),
            (
                ActionError::TrashPageRestoreUnsupported {
                    node: node.clone(),
                    slug: "arquivo".to_string(),
                },
                true,
            ),
            (
                ActionError::TrashParentMissing {
                    node: node.clone(),
                    parent: parent.clone(),
                },
                true,
            ),
            (
                ActionError::TrashOriginUnknown {
                    node: node.clone(),
                    why: "its ops could not be read".to_string(),
                },
                true,
            ),
            (ActionError::TrashOriginInsideSubtree { node, parent }, true),
        ]
    }

    /// The refusals that are a *problem* each offer a way out, on every
    /// surface. This is what the `_` arm in [`remedy_for`] cannot be
    /// trusted for.
    #[test]
    fn every_trash_refusal_offers_a_way_out() {
        for (err, wants_remedy) in every_trash_refusal() {
            assert_eq!(
                remedy_for(&err).is_some(),
                wants_remedy,
                "{err:?} disagrees with the recorded verdict about having a remedy"
            );
        }
    }

    /// A surface either hands over something to run, or says there is
    /// nothing to run. Neither state may be silent, which is the whole
    /// of invariant 12 applied to this table.
    #[test]
    fn every_surface_says_something_about_every_remedy() {
        let block = "01JQ0000000000000000000002";
        for remedy in [
            Remedy::MoveOutOfTrash { block },
            Remedy::CheckWorkspaceHealth,
            Remedy::ReadTrashedSubtree { block },
        ] {
            for surface in Surface::ALL {
                let action = remedy_action(surface, remedy);
                assert!(
                    !action.sentence.trim().is_empty(),
                    "{remedy:?} on {surface:?} says nothing"
                );
                // A sentence that hands off to a command has to hand
                // off; one that does not has to stand on its own. The
                // join in `trash_refusal_text` reads as prose only if
                // this holds.
                if action.command.is_some() {
                    assert!(
                        action.sentence.trim_end().ends_with(':'),
                        "{remedy:?} on {surface:?} names a command without handing off to it: {}",
                        action.sentence
                    );
                } else {
                    assert!(
                        action.sentence.trim_end().ends_with('.'),
                        "{remedy:?} on {surface:?} has no command and no full stop: {}",
                        action.sentence
                    );
                }
            }
        }
    }

    /// **The defect this module was built for.** The cycle refusal's
    /// instruction is performable on the CLI and the MCP and on neither
    /// outline, so those two get the command and the other three are
    /// told there is nothing to do here — instead of being handed a
    /// to-do they cannot perform against a block no view shows.
    #[test]
    fn only_a_surface_that_can_move_a_trashed_block_is_told_to() {
        let err = ActionError::TrashOriginInsideSubtree {
            node: "01JQ0000000000000000000001".to_string(),
            parent: "01JQ0000000000000000000002".to_string(),
        };
        for surface in Surface::ALL {
            let text = trash_refusal_text(surface, &err);
            let prescribes = text.contains("move it back out");
            assert_eq!(
                prescribes,
                matches!(surface, Surface::Cli | Surface::Mcp),
                "{surface:?} is handed the wrong kind of answer: {text}"
            );
            if !prescribes {
                assert!(
                    text.contains("nothing to do about this one from here"),
                    "{surface:?} must say it has no way out rather than going quiet: {text}"
                );
            }
        }
    }

    /// No surface is told to run a binary it does not have. The three
    /// strings this replaced each named one.
    #[test]
    fn no_outl_command_reaches_a_phone() {
        for (err, _) in every_trash_refusal() {
            let text = trash_refusal_text(Surface::Mobile, &err);
            assert!(
                !text.contains("outl doctor")
                    && !text.contains("outl block")
                    && !text.contains("outl_"),
                "a phone was told to run something: {text}"
            );
        }
    }

    /// The join reads as prose on every surface, the same property
    /// `the_one_line_form_is_not_a_run_on` pins for the ahead-of-log
    /// notice — and for the same reason: owning the join here is what
    /// keeps a client from inventing its own.
    #[test]
    fn the_joined_sentence_is_not_a_run_on() {
        for (err, _) in every_trash_refusal() {
            for surface in Surface::ALL {
                let text = trash_refusal_text(surface, &err);
                assert!(!text.contains('\n'), "{surface:?} is not one line: {text}");
                assert!(
                    !text.contains(":  ") && !text.contains(" .") && !text.contains("  "),
                    "{surface:?} has a punctuation seam: {text}"
                );
                // The diagnosis is never swallowed by the advice.
                assert!(
                    text.starts_with(&err.to_string()),
                    "{surface:?} lost the diagnosis: {text}"
                );
            }
        }
    }

    /// A refusal with no remedy is the bare `Display`, byte for byte —
    /// not an empty seam, and not an invented reassurance.
    #[test]
    fn a_refusal_with_no_remedy_is_left_alone() {
        let err = ActionError::NotTrashed("01JQ0000000000000000000001".to_string());
        for surface in Surface::ALL {
            assert_eq!(trash_refusal_text(surface, &err), err.to_string());
        }
    }

    /// The diagnosis carries no instruction of its own, or the join
    /// would state the way out twice — once wrongly. This is what the
    /// three strings in `error.rs` and `trash.rs` were.
    #[test]
    fn no_diagnosis_prescribes_a_command() {
        for (err, _) in every_trash_refusal() {
            let diagnosis = err.to_string();
            for forbidden in ["outl doctor", "outl block", "outl_", "outl reconcile"] {
                assert!(
                    !diagnosis.contains(forbidden),
                    "{err:?}'s Display names `{forbidden}`, which is true on some surfaces \
                     and false on others: {diagnosis}"
                );
            }
        }
    }
}
