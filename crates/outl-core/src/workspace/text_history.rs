//! Reconstructing a block's *past* text from the op log.
//!
//! [`Workspace::block_text`](super::Workspace::block_text) answers "what
//! does this block say now". This module answers "what did it say before",
//! and it exists because the two questions have very different failure
//! costs: an `Op::Edit` that replaced a block's text with a shorter string
//! looks, from the materialized tree, exactly like the block never held
//! more.
//!
//! It did. `Op::Edit` carries a Yrs `update_v1` delta, not a snapshot, and
//! the log is append-only — so the pre-edit text is still reconstructible
//! by replaying the block's `Edit`s up to (but not past) the shrink. That
//! is the only recovery route for content whose `.md` was overwritten
//! before anyone noticed, which `outl reconcile --ahead-of-log` (a
//! `.md → tree` path) cannot reach.
//!
//! See `outl_actions::recover` for the caller, and
//! [RFC 0210](../../../../docs/rfcs/0210-md-content-outside-op-log.md).

use super::{Workspace, WorkspaceError};
use crate::hlc::Hlc;
use crate::id::{ActorId, NodeId};
use crate::op::{LogOp, Op};

/// One past state of a block's text, with the identity of the edit that
/// produced it.
///
/// [`Workspace::block_text_history`] answers *what* the block said and is
/// all a recovery caller needs. A timeline needs *when* and *by whom* as
/// well, and both were already on the `LogOp` — this type stops dropping
/// them on the floor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRevision {
    /// HLC of the `Op::Edit` that produced this state. Total order, so
    /// two revisions are comparable across devices.
    pub ts: Hlc,
    /// The device that made the edit.
    pub actor: ActorId,
    /// What the block said once this edit had been applied.
    pub text: String,
}

impl Workspace {
    /// Every intermediate state of `node`'s text, oldest first — one entry
    /// per `Op::Edit` in the block's history, the last entry being the
    /// block's current text. Empty when the block was never edited.
    ///
    /// Reads the node's ops from **storage**, never from the resident log
    /// or the text cache. Both are boot-mode dependent (a snapshot boot
    /// leaves the resident log holding only the post-cutoff delta), so a
    /// history read from either could silently *shorten* — and a shortened
    /// history is indistinguishable from "nothing was lost here", which is
    /// the exact wrong answer for a recovery caller to get. The op log is
    /// the source of truth; this reads it.
    ///
    /// Cost is one index-driven read set per node — O(edits-of-node), not
    /// O(log).
    pub fn block_text_history(&self, node: NodeId) -> Result<Vec<String>, WorkspaceError> {
        Ok(self
            .block_revisions(node)?
            .into_iter()
            .map(|rev| rev.text)
            .collect())
    }

    /// [`Self::block_text_history`] with the `Hlc` and [`ActorId`] of the
    /// edit that produced each state kept alongside the text.
    ///
    /// The same read, the same order, the same storage-only sourcing —
    /// this is the owner and `block_text_history` is the projection, so
    /// the two can never disagree about what a block's past was.
    pub fn block_revisions(&self, node: NodeId) -> Result<Vec<TextRevision>, WorkspaceError> {
        let ops = self.ops_for_node(node)?;
        let edits: Vec<&LogOp> = ops
            .iter()
            .filter(|logged| matches!(logged.op, Op::Edit { .. }))
            .collect();
        let texts = crate::content::text_revisions(edits.iter().map(|logged| match &logged.op {
            Op::Edit { text_op, .. } => text_op.as_slice(),
            // Filtered above; `text_revisions` takes updates, not ops, so
            // the type can't carry the invariant for us.
            _ => unreachable!("filtered to Op::Edit"),
        }));
        Ok(edits
            .into_iter()
            .zip(texts)
            .map(|(logged, text)| TextRevision {
                ts: logged.ts,
                actor: logged.actor,
                text,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use crate::fractional::Fractional;
    use crate::hlc::HlcGenerator;
    use crate::id::{ActorId, NodeId};
    use crate::op::{LogOp, Op};
    use crate::workspace::Workspace;

    fn logged(hlc: &HlcGenerator, op: Op) -> LogOp {
        let ts = hlc.next();
        LogOp {
            ts,
            actor: ts.actor,
            op,
        }
    }

    /// Build a workspace holding one block that went through `texts` in
    /// order, returning it plus the block id.
    fn block_with_edits(texts: &[&str]) -> (Workspace, NodeId) {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).expect("in-memory workspace");
        let node = NodeId::new();
        ws.apply(logged(
            &hlc,
            Op::Create {
                node,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        ))
        .expect("create");
        for text in texts {
            let update = ws.build_text_replace_update(node, text);
            ws.apply(logged(
                &hlc,
                Op::Edit {
                    node,
                    text_op: update,
                },
            ))
            .expect("edit");
        }
        (ws, node)
    }

    #[test]
    fn a_never_edited_block_has_no_history() {
        let (ws, node) = block_with_edits(&[]);
        assert!(ws.block_text_history(node).expect("history").is_empty());
    }

    #[test]
    fn history_holds_one_entry_per_edit_in_order() {
        let (ws, node) = block_with_edits(&["one", "one two", "one two three"]);
        assert_eq!(
            ws.block_text_history(node).expect("history"),
            vec!["one", "one two", "one two three"]
        );
    }

    #[test]
    fn the_last_entry_is_the_current_text() {
        let (ws, node) = block_with_edits(&["draft", "final"]);
        let history = ws.block_text_history(node).expect("history");
        assert_eq!(
            history.last().map(String::as_str),
            ws.block_text(node).as_deref()
        );
    }

    /// The whole point: an edit that shrank the block did not erase what
    /// it replaced. The long text is gone from the tree and still in the
    /// log.
    #[test]
    fn a_truncating_edit_leaves_the_earlier_text_recoverable() {
        let long = "title\nbody line one\nbody line two";
        let (ws, node) = block_with_edits(&[long, "title"]);
        assert_eq!(ws.block_text(node).as_deref(), Some("title"));
        assert_eq!(
            ws.block_text_history(node).expect("history")[0].as_str(),
            long
        );
    }

    /// The metadata half. `block_revisions` and `block_text_history` are
    /// the same read, so the texts must line up entry for entry — a
    /// divergence here would mean two answers about one block's past.
    #[test]
    fn revisions_carry_the_texts_history_reports() {
        let (ws, node) = block_with_edits(&["one", "one two"]);
        let revisions = ws.block_revisions(node).expect("revisions");
        assert_eq!(
            revisions.iter().map(|r| r.text.clone()).collect::<Vec<_>>(),
            ws.block_text_history(node).expect("history")
        );
    }

    /// Every revision names the edit that produced it, and the HLCs are
    /// strictly increasing — that ordering is what a timeline sorts on.
    #[test]
    fn revisions_carry_a_rising_timestamp_and_an_actor() {
        let (ws, node) = block_with_edits(&["one", "one two", "one two three"]);
        let revisions = ws.block_revisions(node).expect("revisions");
        assert_eq!(revisions.len(), 3);
        assert!(revisions.windows(2).all(|w| w[0].ts < w[1].ts));
        assert!(revisions.iter().all(|r| r.actor == ws.actor));
    }

    /// `ops_for_node` is the general read the revisions are built on, so
    /// it has to see the structural ops too — a timeline needs the
    /// `Create` and the `Move`, not just the `Edit`s.
    #[test]
    fn ops_for_node_returns_structure_as_well_as_edits() {
        let (ws, node) = block_with_edits(&["one"]);
        let ops = ws.ops_for_node(node).expect("ops");
        assert!(ops.iter().any(|o| matches!(o.op, Op::Create { .. })));
        assert!(ops.iter().any(|o| matches!(o.op, Op::Edit { .. })));
        assert!(ops.windows(2).all(|w| w[0].ts <= w[1].ts));
    }

    /// A block never touched by an `Edit` reads back empty rather than
    /// erroring, so a caller can scan every node in a tree unconditionally.
    #[test]
    fn an_unknown_node_has_no_history() {
        let (ws, _) = block_with_edits(&["x"]);
        assert!(ws
            .block_text_history(NodeId::new())
            .expect("history")
            .is_empty());
    }
}
