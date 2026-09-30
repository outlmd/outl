//! Which of a node's placements the tree actually applied.
//!
//! Root `CLAUDE.md` invariant 4 keeps a `Move` the tree refused as a
//! cycle in the log, so "the ops that name this node" and "the parents
//! this node had" are different lists. Telling them apart needs the tree
//! **as it was when each op ran**: `Move(A, B)` is refused while `B` sits
//! under `A`, and nothing in the op itself, nor in today's tree, records
//! that. `B` may have moved elsewhere since.
//!
//! `do_op` decides with `creates_cycle` against the tree at that instant,
//! and so does this, one node at a time: the parent `X` had just before
//! `t` is the result of `X`'s own placements before `t`, each accepted or
//! refused by the same test against the ancestors of its target just
//! before *its* `t`. Every question asks about a strictly earlier time,
//! so the recursion ends, and it reads only the ops of the nodes on those
//! ancestor chains rather than replaying the whole log.
//!
//! `Move.old_parent` would have answered this for free if it could be
//! trusted, and it cannot; see [`super::parent_at_deletion`].

use std::collections::HashMap;

use outl_core::hlc::Hlc;
use outl_core::id::NodeId;
use outl_core::op::Op;
use outl_core::workspace::Workspace;

use crate::error::ActionError;

/// An op that tries to put a node somewhere.
#[derive(Clone, Copy)]
enum Placement {
    Create(NodeId),
    Move(NodeId),
}

/// One node's placements and, for the prefix resolved so far, the parent
/// it had after each. `None` means the node did not exist yet.
struct Trail {
    ops: Vec<(Hlc, Placement)>,
    after: Vec<Option<NodeId>>,
}

/// Memoised replay of parent trails, scoped to one question.
pub(super) struct Replay<'a> {
    workspace: &'a Workspace,
    trails: HashMap<NodeId, Trail>,
}

impl<'a> Replay<'a> {
    pub(super) fn new(workspace: &'a Workspace) -> Self {
        Self {
            workspace,
            trails: HashMap::new(),
        }
    }

    /// `node`'s parent after each op that tried to place it, oldest
    /// first. A refused op repeats the parent before it.
    pub(super) fn trail(&mut self, node: NodeId) -> Result<&[Option<NodeId>], ActionError> {
        self.resolve(node, None)?;
        Ok(self.trails[&node].after.as_slice())
    }

    fn load(&mut self, node: NodeId) -> Result<(), ActionError> {
        if self.trails.contains_key(&node) {
            return Ok(());
        }
        let ops = self
            .workspace
            .ops_for_node(node)?
            .into_iter()
            .filter_map(|logged| match logged.op {
                Op::Create {
                    node: n, parent, ..
                } if n == node => Some((logged.ts, Placement::Create(parent))),
                Op::Move {
                    node: n,
                    new_parent,
                    ..
                } if n == node => Some((logged.ts, Placement::Move(new_parent))),
                _ => None,
            })
            .collect();
        self.trails.insert(
            node,
            Trail {
                ops,
                after: Vec::new(),
            },
        );
        Ok(())
    }

    /// Decide every placement of `node` older than `before` (all of them
    /// when `None`), exactly as `do_op` would have.
    fn resolve(&mut self, node: NodeId, before: Option<Hlc>) -> Result<(), ActionError> {
        self.load(node)?;
        loop {
            let trail = &self.trails[&node];
            let i = trail.after.len();
            let Some(&(ts, placement)) = trail.ops.get(i) else {
                return Ok(());
            };
            if before.is_some_and(|before| ts >= before) {
                return Ok(());
            }
            let current = i.checked_sub(1).and_then(|prev| trail.after[prev]);
            let next = match placement {
                // `do_op(Create)` only seeds a node that is absent: a
                // reconcile re-emits `Create` for blocks that already
                // exist, naming a parent the tree discards.
                Placement::Create(parent)
                    if current.is_none() && !self.creates_cycle(node, parent, ts)? =>
                {
                    Some(parent)
                }
                // A `Move` that arrived before its `Create` is a no-op.
                Placement::Move(new_parent)
                    if current.is_some() && !self.creates_cycle(node, new_parent, ts)? =>
                {
                    Some(new_parent)
                }
                _ => current,
            };
            self.trails
                .get_mut(&node)
                .expect("loaded above")
                .after
                .push(next);
        }
    }

    /// The parent `node` had just before `at`.
    fn parent_before(&mut self, node: NodeId, at: Hlc) -> Result<Option<NodeId>, ActionError> {
        // Sentinels have no parent, and loading every op that names one
        // would read the whole log.
        if node == NodeId::root() || node == NodeId::trash() {
            return Ok(None);
        }
        self.resolve(node, Some(at))?;
        let trail = &self.trails[&node];
        let decided = trail.ops.partition_point(|(ts, _)| *ts < at);
        Ok(decided.checked_sub(1).and_then(|last| trail.after[last]))
    }

    /// `Tree::creates_cycle`, asked of the tree as it stood just before `at`.
    fn creates_cycle(
        &mut self,
        node: NodeId,
        target: NodeId,
        at: Hlc,
    ) -> Result<bool, ActionError> {
        let mut current = target;
        loop {
            if current == node {
                return Ok(true);
            }
            match self.parent_before(current, at)? {
                Some(parent) => current = parent,
                None => return Ok(false),
            }
        }
    }
}
