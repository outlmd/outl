//! Build a whole subtree — or a forest of them — in one call.
//!
//! [`BlockTreeSpec`] is the shape agents and import pipelines naturally
//! produce ("a page with these bullets and these sub-bullets"), and
//! [`BlockTreeOutcome`] mirrors it so the caller can walk the spec and
//! the freshly minted ids in lockstep.
//!
//! The batching is the reason this is not a loop at the call site: one
//! user-visible action is one [`Workspace::begin_batch`], so N nodes
//! cost one flush per storage destination instead of an fsync per node.

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use serde::{Deserialize, Serialize};

use crate::error::ActionError;

use super::create::append_block;

/// Recursive spec for building a block + its descendants in one
/// shot. The shape is what agents naturally produce ("write me a
/// page with these bullets and these sub-bullets") and what import
/// pipelines reduce trees to.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BlockTreeSpec {
    /// Raw block text (TODO/DONE prefix is left to the caller, same
    /// rule as [`super::edit_text`]).
    pub text: String,
    /// Children specs, applied left-to-right as last children of
    /// the parent. Empty by default.
    #[serde(default)]
    pub children: Vec<BlockTreeSpec>,
}

/// Outcome of `append_tree` / `create_under_tree`. Mirrors the
/// shape of the input so callers can walk the original spec and the
/// freshly minted ids in lockstep.
#[derive(Debug, Clone, Serialize)]
pub struct BlockTreeOutcome {
    /// Id of the node created for the root of this subtree.
    pub id: NodeId,
    /// Children outcomes, in the same order as the input
    /// `children`. Empty when the spec had no children.
    pub children: Vec<BlockTreeOutcome>,
}

/// Append a whole subtree under `parent` in a single call.
///
/// `spec.text` becomes a new last child of `parent`; each entry in
/// `spec.children` is then attached recursively as the last child of
/// that new node. The returned `BlockTreeOutcome` mirrors the input
/// shape so the caller can pair every spec node with its freshly
/// minted [`NodeId`].
///
/// The whole subtree persists in **one** batch: this is one user-visible
/// action, so it flushes once per storage destination
/// ([`Workspace::begin_batch`]) instead of fsyncing per node.
///
/// Failure mode: if any nested op fails, the previously-applied ops
/// stay in the op log (we intentionally do not roll them back — the
/// CRDT log is append-only and the partial subtree is observable
/// behavior). On the error path the batch guard drops and flushes the
/// ops applied so far best-effort, so the on-disk state matches the
/// per-op path's. Callers that need all-or-nothing semantics should run
/// the spec through validation first.
pub fn append_tree(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    parent: NodeId,
    spec: &BlockTreeSpec,
) -> Result<BlockTreeOutcome, ActionError> {
    let mut batch = workspace.begin_batch();
    let outcome = append_tree_inner(&mut batch, hlc, parent, spec)?;
    batch.commit()?;
    Ok(outcome)
}

/// Non-batched recursive core shared by [`append_tree`] and
/// [`append_forest`].
///
/// The public entry points open a single [`Workspace::begin_batch`] and
/// drive this helper. Keeping the recursion off `begin_batch` means the
/// batch depth counter is pushed once at the entry, not once per node —
/// the whole forest still coalesces into one flush per destination.
fn append_tree_inner(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    parent: NodeId,
    spec: &BlockTreeSpec,
) -> Result<BlockTreeOutcome, ActionError> {
    let id = append_block(workspace, hlc, Some(parent), Some(&spec.text))?;
    let children = spec
        .children
        .iter()
        .map(|child| append_tree_inner(workspace, hlc, id, child))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BlockTreeOutcome { id, children })
}

/// Append every entry in `specs` as a contiguous block of new last
/// children under `parent`, preserving order. Convenience for
/// `outl_page_create`-with-content where the caller hands us the
/// page's top-level outline as a forest.
///
/// Like [`append_tree`], the entire forest persists in one batch — one
/// flush per destination for the whole call.
pub fn append_forest(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    parent: NodeId,
    specs: &[BlockTreeSpec],
) -> Result<Vec<BlockTreeOutcome>, ActionError> {
    let mut batch = workspace.begin_batch();
    let outcomes = specs
        .iter()
        .map(|spec| append_tree_inner(&mut batch, hlc, parent, spec))
        .collect::<Result<Vec<_>, _>>()?;
    batch.commit()?;
    Ok(outcomes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_core::id::ActorId;

    fn new_workspace() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    #[test]
    fn append_tree_creates_root_and_children_in_order() {
        let (mut ws, hlc) = new_workspace();
        let spec = BlockTreeSpec {
            text: "root".into(),
            children: vec![
                BlockTreeSpec {
                    text: "a".into(),
                    children: vec![BlockTreeSpec {
                        text: "a1".into(),
                        children: vec![],
                    }],
                },
                BlockTreeSpec {
                    text: "b".into(),
                    children: vec![],
                },
            ],
        };
        let outcome = append_tree(&mut ws, &hlc, NodeId::root(), &spec).unwrap();

        assert_eq!(ws.block_text(outcome.id).as_deref(), Some("root"));
        assert_eq!(outcome.children.len(), 2);

        let a = &outcome.children[0];
        let b = &outcome.children[1];
        assert_eq!(ws.block_text(a.id).as_deref(), Some("a"));
        assert_eq!(ws.block_text(b.id).as_deref(), Some("b"));
        assert_eq!(ws.tree().parent(a.id), Some(outcome.id));
        assert_eq!(ws.tree().parent(b.id), Some(outcome.id));

        // Children of `root` come back in insertion order.
        let kids: Vec<NodeId> = crate::tree::children_of(&ws, outcome.id)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(kids, vec![a.id, b.id]);

        // a's nested child landed under a.
        assert_eq!(a.children.len(), 1);
        let a1 = &a.children[0];
        assert_eq!(ws.block_text(a1.id).as_deref(), Some("a1"));
        assert_eq!(ws.tree().parent(a1.id), Some(a.id));
    }

    #[test]
    fn append_forest_preserves_order_and_targets_parent() {
        let (mut ws, hlc) = new_workspace();
        let parent = append_block(&mut ws, &hlc, None, Some("parent")).unwrap();
        let specs = vec![
            BlockTreeSpec {
                text: "one".into(),
                children: vec![],
            },
            BlockTreeSpec {
                text: "two".into(),
                children: vec![],
            },
            BlockTreeSpec {
                text: "three".into(),
                children: vec![],
            },
        ];
        let outcomes = append_forest(&mut ws, &hlc, parent, &specs).unwrap();
        let ids: Vec<NodeId> = outcomes.iter().map(|o| o.id).collect();
        let kids: Vec<NodeId> = crate::tree::children_of(&ws, parent)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(kids, ids);
    }

    #[test]
    fn append_tree_empty_text_creates_node_without_edit() {
        let (mut ws, hlc) = new_workspace();
        let spec = BlockTreeSpec {
            text: "".into(),
            children: vec![],
        };
        let outcome = append_tree(&mut ws, &hlc, NodeId::root(), &spec).unwrap();
        // Empty text path skips the Edit op; block_text returns empty
        // string (Yrs default).
        assert_eq!(
            ws.block_text(outcome.id).as_deref().unwrap_or(""),
            "",
            "empty-text spec must still create the node"
        );
    }

    /// Recursive `(text child…)` fingerprint of a subtree — id/HLC-free,
    /// so two independent runs of the same spec compare equal.
    fn shape(ws: &Workspace, parent: NodeId) -> String {
        let mut s = String::new();
        for (id, _) in crate::tree::children_of(ws, parent) {
            s.push('(');
            s.push_str(&ws.block_text(id).unwrap_or_default());
            s.push_str(&shape(ws, id));
            s.push(')');
        }
        s
    }

    fn nested_specs() -> Vec<BlockTreeSpec> {
        vec![
            BlockTreeSpec {
                text: "root1".into(),
                children: vec![
                    BlockTreeSpec {
                        text: "a".into(),
                        children: vec![BlockTreeSpec {
                            text: "a1".into(),
                            children: vec![],
                        }],
                    },
                    BlockTreeSpec {
                        text: "b".into(),
                        children: vec![],
                    },
                ],
            },
            BlockTreeSpec {
                text: "root2".into(),
                children: vec![],
            },
        ]
    }

    /// Batching `append_forest` must be observationally identical to
    /// applying the same ops one at a time: the persisted op stream has
    /// the same kinds in the same order, the materialized tree is the
    /// same, and reloading the batched workspace from disk reproduces it.
    #[test]
    fn append_forest_batched_matches_sequential_and_persists() {
        use outl_core::op::{LogOp, Op};
        use outl_core::storage::{JsonlStorage, Storage};
        use std::mem::Discriminant;
        use tempfile::TempDir;

        fn kinds(ops: &[LogOp]) -> Vec<Discriminant<Op>> {
            ops.iter().map(|o| std::mem::discriminant(&o.op)).collect()
        }

        let specs = nested_specs();

        // Batched: `append_forest` opens one batch for the whole forest.
        let actor_a = ActorId::new();
        let tmp_a = TempDir::new().unwrap();
        let (shape_a, parent_a) = {
            let g = HlcGenerator::new(actor_a);
            let store = Box::new(JsonlStorage::open(tmp_a.path().to_path_buf(), actor_a).unwrap());
            let mut ws = Workspace::open_with_storage(actor_a, store, None).unwrap();
            let parent = append_block(&mut ws, &g, None, Some("parent")).unwrap();
            append_forest(&mut ws, &g, parent, &specs).unwrap();
            (shape(&ws, parent), parent)
        };

        // Sequential: drive the same non-batched core per node, so every
        // op persists immediately (one `append_op` each).
        let actor_b = ActorId::new();
        let tmp_b = TempDir::new().unwrap();
        let shape_b = {
            let g = HlcGenerator::new(actor_b);
            let store = Box::new(JsonlStorage::open(tmp_b.path().to_path_buf(), actor_b).unwrap());
            let mut ws = Workspace::open_with_storage(actor_b, store, None).unwrap();
            let parent = append_block(&mut ws, &g, None, Some("parent")).unwrap();
            for spec in &specs {
                append_tree_inner(&mut ws, &g, parent, spec).unwrap();
            }
            shape(&ws, parent)
        };

        // Same materialized tree either way.
        assert_eq!(shape_a, shape_b);

        // Same op-kind stream persisted, in the same HLC order.
        let ops_a = JsonlStorage::open(tmp_a.path().to_path_buf(), actor_a)
            .unwrap()
            .all_ops()
            .unwrap();
        let ops_b = JsonlStorage::open(tmp_b.path().to_path_buf(), actor_b)
            .unwrap()
            .all_ops()
            .unwrap();
        assert_eq!(kinds(&ops_a), kinds(&ops_b));

        // Reloading the batched workspace reproduces its in-memory tree.
        let store = Box::new(JsonlStorage::open(tmp_a.path().to_path_buf(), actor_a).unwrap());
        let reloaded = Workspace::open_with_storage(actor_a, store, None).unwrap();
        assert_eq!(shape(&reloaded, parent_a), shape_a);
    }

    /// Storage that fails the batch flush (`append_ops`) while letting the
    /// pre-batch immediate path (`append_op`) through, to exercise the
    /// error path of a batched composite action.
    struct FlushFailStorage {
        inner: outl_core::storage::MemoryStorage,
    }

    impl outl_core::storage::Storage for FlushFailStorage {
        fn append_op(
            &mut self,
            op: &outl_core::op::LogOp,
        ) -> Result<(), outl_core::storage::StorageError> {
            self.inner.append_op(op)
        }
        fn append_ops(
            &mut self,
            _ops: &[outl_core::op::LogOp],
        ) -> Result<(), outl_core::storage::StorageError> {
            Err(outl_core::storage::StorageError::Backend(
                "flush boom".into(),
            ))
        }
        fn ops_since(
            &self,
            ts: outl_core::hlc::Hlc,
        ) -> Result<Vec<outl_core::op::LogOp>, outl_core::storage::StorageError> {
            self.inner.ops_since(ts)
        }
        fn ops_for_node(
            &self,
            id: NodeId,
        ) -> Result<Vec<outl_core::op::LogOp>, outl_core::storage::StorageError> {
            self.inner.ops_for_node(id)
        }
        fn ops_for_actor(
            &self,
            id: outl_core::id::ActorId,
        ) -> Result<Vec<outl_core::op::LogOp>, outl_core::storage::StorageError> {
            self.inner.ops_for_actor(id)
        }
        fn last_ts_per_actor(
            &self,
        ) -> Result<
            std::collections::HashMap<outl_core::id::ActorId, outl_core::hlc::Hlc>,
            outl_core::storage::StorageError,
        > {
            self.inner.last_ts_per_actor()
        }
        fn all_ops(&self) -> Result<Vec<outl_core::op::LogOp>, outl_core::storage::StorageError> {
            self.inner.all_ops()
        }
    }

    /// When the batch flush fails mid-action, the error propagates but the
    /// ops the forest already applied stay in the tree: the append-only
    /// log is the source of truth, and a persistence failure never unwinds
    /// in-memory state — the same final tree the per-op path produced.
    #[test]
    fn append_forest_flush_failure_propagates_but_keeps_applied_ops() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let store = Box::new(FlushFailStorage {
            inner: outl_core::storage::MemoryStorage::new(),
        });
        let mut ws = Workspace::open_with_storage(actor, store, None).unwrap();

        // Parent is created on the immediate path (append_op) — succeeds.
        let parent = append_block(&mut ws, &g, None, Some("parent")).unwrap();

        // The forest applies to the CRDT (buffered), then the flush on
        // commit hits the failing storage → the error surfaces.
        let res = append_forest(&mut ws, &g, parent, &nested_specs());
        assert!(res.is_err(), "flush failure must propagate");

        // No rollback: both roots and the nested child are still present.
        let roots: Vec<String> = crate::tree::children_of(&ws, parent)
            .into_iter()
            .map(|(id, _)| ws.block_text(id).unwrap_or_default())
            .collect();
        assert_eq!(roots, vec!["root1".to_string(), "root2".to_string()]);

        let root1 = crate::tree::children_of(&ws, parent)[0].0;
        assert_eq!(shape(&ws, root1), "(a(a1))(b)");
    }
}
