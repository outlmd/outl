//! `creates_cycle` — the cycle-detection guard for `Op::Move`.
//!
//! Algorithm: walk up from `new_parent` following `parent(_)` until
//! we hit a sentinel (ROOT / TRASH_ROOT) or `node` itself. If we
//! reach `node`, then `node` is an ancestor of `new_parent` and the
//! move would close a loop. Bounded by `node_count + 2` steps so a
//! malformed tree (no sentinel reachable) cannot spin forever.

use super::Tree;
use crate::id::NodeId;

impl Tree {
    /// Whether moving `node` under `new_parent` would create a cycle.
    ///
    /// `node == new_parent` is always a cycle. Otherwise we walk up from
    /// `new_parent` toward the root. If we encounter `node` along the way,
    /// `node` is an ancestor of `new_parent` and the move would close a
    /// loop. The ROOT / TRASH_ROOT sentinels terminate the walk.
    pub fn creates_cycle(&self, node: NodeId, new_parent: NodeId) -> bool {
        if node == new_parent {
            return true;
        }
        let mut cursor = new_parent;
        // Bound the walk by the number of edges in the tree as a safety
        // net against malformed state. A well-formed tree terminates in
        // at most `node_count` steps.
        let mut steps = 0usize;
        let max_steps = self.nodes.len() + 2;
        loop {
            if cursor == NodeId::root() || cursor == NodeId::trash() {
                return false;
            }
            match self.parent(cursor) {
                None => return false,
                Some(p) => {
                    if p == node {
                        return true;
                    }
                    cursor = p;
                }
            }
            steps += 1;
            // Defensive only, and **unreachable while the tree is acyclic** —
            // which `theorem apply_ops_acyclic` (Kleppmann et al. 2022 §4.1)
            // guarantees for any tree built by `apply_op`. The bound is not
            // part of the algorithm: the paper's `ancestor` is an inductive
            // least fixed point over a tree already proved acyclic, and needs
            // no bailout. Do not "optimize" `max_steps` into something that
            // varies for another reason — under a malformed tree the answer
            // already depends on `nodes.len()`, and two replicas that
            // disagree about malformation would then diverge with no op
            // behind it. `true` (refuse the move) is the conservative
            // direction to fail in.
            //
            // The `return true` below is therefore **dead in any debug
            // build**: the assert one line up fires first, so `cargo test`
            // (and `cargo llvm-cov`, which builds in debug) can never reach
            // it. That is the one line keeping `creates_cycle` off the 100 %
            // target in `crates/outl-core/CLAUDE.md`, and it is a property of
            // this pair of statements, not a missing test.
            debug_assert!(
                steps <= max_steps,
                "creates_cycle: malformed tree (loop without sentinel)"
            );
            if steps > max_steps {
                return true;
            }
        }
    }
}
