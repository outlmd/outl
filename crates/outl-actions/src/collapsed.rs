//! Block fold (collapsed) state — Op-driven sync.
//!
//! Folding a block is a user-meaningful state mutation that must
//! converge between devices. Every flip goes through the op log
//! (`Op::SetCollapsed` in `outl-core`), exactly like Move / Edit /
//! SetProp. Each device writes to its own `ops-<actor>.jsonl`; iCloud
//! / Syncthing / shared FS sync those per-actor files; the CRDT
//! merges concurrent flips by HLC ordering.
//!
//! **Do not bring back the "write a flag to the sidecar" path.** That
//! was the previous design and it lost flips under iCloud's
//! last-write-wins-per-file semantics. The sidecar is for structural
//! `.md` ↔ tree matching only; any cross-device state belongs in the
//! op log. The root `CLAUDE.md` invariant codifies this.

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::op::{LogOp, Op};
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::tree::{ancestors_below_page, is_live};

/// Set `node`'s collapsed flag to `value`. Generates `Op::SetCollapsed`
/// with a fresh HLC and applies it through `Workspace::apply`.
///
/// Returns `true` when the materialised state actually changed (the
/// flag flipped), `false` when the op was a no-op against the current
/// state. Either way the op enters the log — the CRDT needs every
/// flip to converge across devices, and idempotent re-apply is cheap.
pub fn set_block_collapsed(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
    value: bool,
) -> Result<bool, ActionError> {
    let was = workspace.tree().is_collapsed(node);
    let ts = hlc.next();
    let op = LogOp {
        ts,
        actor: ts.actor,
        op: Op::SetCollapsed {
            node,
            value,
            // Filled by `do_op`. The value we pass here is irrelevant
            // beyond the type contract.
            old_value: false,
        },
    };
    workspace.apply(op)?;
    Ok(was != value)
}

/// Flip `node`'s collapsed flag. Returns the new value.
pub fn toggle_block_collapsed(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<bool, ActionError> {
    let new_value = !workspace.tree().is_collapsed(node);
    set_block_collapsed(workspace, hlc, node, new_value)?;
    Ok(new_value)
}

/// Open every folded block between `node` and its page, nearest first,
/// and return the ones that were folded.
///
/// **The single owner of "reveal this block"**: a client that lands on a
/// block (a backlink's origin, a search hit) calls this with the block
/// and gets the path to it unfolded. It writes `SetCollapsed(false)` only
/// for ancestors that are actually folded, because
/// [`set_block_collapsed`] appends an op even when nothing changes, and a
/// navigation that wrote one op per ancestor would grow every device's
/// log for a gesture that mostly finds nothing folded. Those writes go in
/// one batch, one flush per destination.
///
/// Neither the page root (it has no fold) nor `node`'s own flag is
/// touched: the caller wants the block *visible*, not its children
/// opened. Nothing folded is `Ok(vec![])` with the log untouched, and a
/// node that is not live is [`ActionError::NotInTree`] with no op.
pub fn unfold_ancestors(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<Vec<NodeId>, ActionError> {
    if !is_live(workspace, node) {
        return Err(ActionError::NotInTree(node.to_string()));
    }
    let folded: Vec<NodeId> = ancestors_below_page(workspace, node)
        .into_iter()
        .filter(|id| workspace.tree().is_collapsed(*id))
        .collect();
    if folded.is_empty() {
        return Ok(folded);
    }
    let mut batch = workspace.begin_batch();
    for id in &folded {
        set_block_collapsed(&mut batch, hlc, *id, false)?;
    }
    batch.commit()?;
    Ok(folded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_core::id::ActorId;

    fn make_ws() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
        let hlc = HlcGenerator::new(actor);
        (ws, hlc)
    }

    #[test]
    fn set_then_read_round_trip() {
        let (mut ws, hlc) = make_ws();
        let node = NodeId::new();
        assert!(!ws.tree().is_collapsed(node));
        let changed = set_block_collapsed(&mut ws, &hlc, node, true).unwrap();
        assert!(changed);
        assert!(ws.tree().is_collapsed(node));
    }

    #[test]
    fn set_block_collapsed_reports_no_change_when_already_equal() {
        // Idempotency at the action level: flipping `true` twice
        // reports `false` on the second call even though the op was
        // still appended to the log (the CRDT needs it for cross-
        // device convergence).
        let (mut ws, hlc) = make_ws();
        let node = NodeId::new();
        assert!(set_block_collapsed(&mut ws, &hlc, node, true).unwrap());
        assert!(!set_block_collapsed(&mut ws, &hlc, node, true).unwrap());
        assert!(ws.tree().is_collapsed(node));
    }

    #[test]
    fn toggle_flips_value() {
        let (mut ws, hlc) = make_ws();
        let node = NodeId::new();
        assert!(toggle_block_collapsed(&mut ws, &hlc, node).unwrap());
        assert!(ws.tree().is_collapsed(node));
        assert!(!toggle_block_collapsed(&mut ws, &hlc, node).unwrap());
        assert!(!ws.tree().is_collapsed(node));
    }

    use crate::block::{append_block, delete};
    use crate::page::{open_or_create as open_or_create_page, PageKind};

    /// page > A > B > C > leaf, returned as (page, a, b, c, leaf).
    fn chain(ws: &mut Workspace, hlc: &HlcGenerator) -> (NodeId, NodeId, NodeId, NodeId, NodeId) {
        let page = open_or_create_page(ws, hlc, "p", "p", PageKind::Page).unwrap();
        let a = append_block(ws, hlc, Some(page), Some("a")).unwrap();
        let b = append_block(ws, hlc, Some(a), Some("b")).unwrap();
        let c = append_block(ws, hlc, Some(b), Some("c")).unwrap();
        let leaf = append_block(ws, hlc, Some(c), Some("leaf")).unwrap();
        (page, a, b, c, leaf)
    }

    #[test]
    fn unfold_ancestors_opens_only_the_folded_ones_nearest_first() {
        let (mut ws, hlc) = make_ws();
        let (_, a, b, c, leaf) = chain(&mut ws, &hlc);
        set_block_collapsed(&mut ws, &hlc, a, true).unwrap();
        set_block_collapsed(&mut ws, &hlc, c, true).unwrap();
        let before = ws.log().len();

        let opened = unfold_ancestors(&mut ws, &hlc, leaf).unwrap();

        assert_eq!(opened, vec![c, a]);
        assert!(!ws.tree().is_collapsed(a));
        assert!(!ws.tree().is_collapsed(c));
        assert!(!ws.tree().is_collapsed(b), "B was open and stays untouched");
        assert_eq!(ws.log().len(), before + 2, "one op per folded ancestor");
    }

    #[test]
    fn unfold_ancestors_with_nothing_folded_writes_no_op() {
        let (mut ws, hlc) = make_ws();
        let (_, _, _, _, leaf) = chain(&mut ws, &hlc);
        let before = ws.log().len();
        assert!(unfold_ancestors(&mut ws, &hlc, leaf).unwrap().is_empty());
        assert_eq!(ws.log().len(), before);
    }

    #[test]
    fn unfold_ancestors_leaves_the_nodes_own_fold_alone() {
        let (mut ws, hlc) = make_ws();
        let (_, a, _, c, _) = chain(&mut ws, &hlc);
        set_block_collapsed(&mut ws, &hlc, a, true).unwrap();
        set_block_collapsed(&mut ws, &hlc, c, true).unwrap();

        let opened = unfold_ancestors(&mut ws, &hlc, c).unwrap();

        assert_eq!(opened, vec![a]);
        assert!(
            ws.tree().is_collapsed(c),
            "the target's own fold is not ours"
        );
    }

    #[test]
    fn unfold_ancestors_never_writes_the_page_root() {
        let (mut ws, hlc) = make_ws();
        let (page, a, _, _, _) = chain(&mut ws, &hlc);
        set_block_collapsed(&mut ws, &hlc, page, true).unwrap();
        let before = ws.log().len();

        assert!(unfold_ancestors(&mut ws, &hlc, a).unwrap().is_empty());
        assert!(unfold_ancestors(&mut ws, &hlc, page).unwrap().is_empty());
        assert_eq!(ws.log().len(), before);
        assert!(ws.tree().is_collapsed(page));
    }

    #[test]
    fn unfold_ancestors_refuses_a_trashed_or_unknown_node_without_ops() {
        let (mut ws, hlc) = make_ws();
        let (_, a, _, c, leaf) = chain(&mut ws, &hlc);
        set_block_collapsed(&mut ws, &hlc, a, true).unwrap();
        delete(&mut ws, &hlc, c).unwrap();
        let before = ws.log().len();

        // `leaf` sits inside the trashed subtree: `is_live` is the verdict.
        assert!(matches!(
            unfold_ancestors(&mut ws, &hlc, leaf),
            Err(ActionError::NotInTree(_))
        ));
        assert!(matches!(
            unfold_ancestors(&mut ws, &hlc, NodeId::new()),
            Err(ActionError::NotInTree(_))
        ));
        assert_eq!(ws.log().len(), before);
        assert!(ws.tree().is_collapsed(a));
    }
}
