//! The sentence a user reads when the block they acted on through a
//! backlink row no longer names a live block.
//!
//! The cross-page write refuses a stale id instead of re-targeting it
//! (the text would land on a page the user is not looking at), so the
//! sentence has one job: say what happened and what it cost. It lives
//! here, beside the other refusal wording, so every client prints the
//! same words and none invents its own.
//!
//! **Two causes**, because they are not the same event. A block a peer
//! deleted keeps its record under the trash. An id that was never a
//! block (a transient one minted for a page whose sidecar is absent,
//! short or misaligned) has no record at all, and so does a block this
//! device has not received yet. The second case cannot be told apart
//! from those, so it claims only what is known: the block was not
//! found.
//!
//! **Two costs**, because the consequence is the other half of the
//! sentence and the clients do not all pay the same one. A text edit
//! loses the user's keystrokes, and it is unrecoverable — that is
//! [`stale_backlink_block_text`]. A chord (a task flip, a move, a
//! delete, an edit that never started) typed nothing, so nothing is
//! lost and the only news is that it did not land — that is
//! [`stale_backlink_block_unchanged_text`]. Telling a user their text
//! was not saved when they pressed `Ctrl+T` names a loss that did not
//! happen; telling them "nothing changed" after they typed a paragraph
//! hides one that did.
//!
//! Both pairs are here rather than one per client, because the *cause*
//! half is the same question and a client that owns half a sentence
//! owns a sentence.

use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use crate::tree::is_trashed;

/// The user-facing sentence for a backlink edit whose block is gone.
///
/// `was_deleted` is true only when the tree still holds the block
/// under the trash.
pub fn stale_backlink_block_text(was_deleted: bool) -> &'static str {
    if was_deleted {
        "That block was deleted, so your text could not be saved."
    } else {
        "That block could not be found, so your text could not be saved."
    }
}

/// The sentence for a backlink **chord** whose block is gone: a task
/// flip, a move, a delete, or an edit that never got to start.
///
/// Same two causes as [`stale_backlink_block_text`], different cost.
/// Nothing was typed, so nothing is lost and the sentence says only
/// that the gesture did not land — claiming "your text could not be
/// saved" on `Ctrl+T` names a loss that did not happen.
pub fn stale_backlink_block_unchanged_text(was_deleted: bool) -> &'static str {
    if was_deleted {
        "That block was deleted, so nothing changed."
    } else {
        "That block could not be found, so nothing changed."
    }
}

/// Pick the sentence for `node` from what the tree says about it.
pub fn stale_backlink_block_refusal(workspace: &Workspace, node: NodeId) -> &'static str {
    stale_backlink_block_text(was_deleted(workspace, node))
}

/// [`stale_backlink_block_refusal`] for a chord — see
/// [`stale_backlink_block_unchanged_text`].
pub fn stale_backlink_block_unchanged_refusal(workspace: &Workspace, node: NodeId) -> &'static str {
    stale_backlink_block_unchanged_text(was_deleted(workspace, node))
}

/// The cause half, asked once so the two refusals cannot disagree
/// about what happened to the same block.
fn was_deleted(workspace: &Workspace, node: NodeId) -> bool {
    workspace.tree().contains(node) && is_trashed(workspace, node)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every sentence in the module, both costs. Covered here rather
    /// than only through the clients that print them: a client that
    /// stopped calling one would leave the catalog's own guarantees
    /// untested.
    fn all_sentences() -> impl Iterator<Item = &'static str> {
        [true, false].into_iter().flat_map(|deleted| {
            [
                stale_backlink_block_text(deleted),
                stale_backlink_block_unchanged_text(deleted),
            ]
        })
    }

    #[test]
    fn no_sentence_leaks_an_implementation_noun() {
        for s in all_sentences() {
            for noun in ["tree", "NodeId", "ULID", "01"] {
                assert!(!s.contains(noun), "{s}");
            }
        }
    }

    #[test]
    fn each_cost_states_its_own_consequence() {
        for deleted in [true, false] {
            let text = stale_backlink_block_text(deleted);
            assert!(text.ends_with("your text could not be saved."), "{text}");
            let unchanged = stale_backlink_block_unchanged_text(deleted);
            assert!(unchanged.ends_with("nothing changed."), "{unchanged}");
            // The whole reason there are two pairs: a chord typed
            // nothing, so it must not claim a loss.
            assert!(!unchanged.contains("text"), "{unchanged}");
        }
    }

    #[test]
    fn only_the_deleted_sentences_claim_a_deletion() {
        for cost in [
            stale_backlink_block_text as fn(bool) -> &'static str,
            stale_backlink_block_unchanged_text,
        ] {
            assert!(cost(true).contains("deleted"), "{}", cost(true));
            assert!(!cost(false).contains("deleted"), "{}", cost(false));
        }
    }

    /// The two costs share the *cause* half, which is what makes one
    /// module rather than two: a change to how a deletion is described
    /// has to reach both, and a second owner is how they drift.
    #[test]
    fn both_costs_describe_the_same_cause_the_same_way() {
        for deleted in [true, false] {
            let cause = if deleted {
                "was deleted"
            } else {
                "could not be found"
            };
            assert!(stale_backlink_block_text(deleted).contains(cause));
            assert!(stale_backlink_block_unchanged_text(deleted).contains(cause));
        }
    }
}
