//! Per-op machinery: `do_op` applies one [`LogOp`] forward,
//! `undo_op` reverts it.
//!
//! Each branch of the `match` here mirrors the corresponding rule in
//! Kleppmann et al. 2022 §3 (Move) and the outl-specific extensions
//! for `SetProp`, `Create`, `SetCollapsed`. The `old_*` fields on
//! `Move` and `SetProp` are filled in by `do_op` so `undo_op` can
//! reverse the transition exactly — see the algorithm sketch in
//! `crates/outl-core/CLAUDE.md`.
//!
//! `Create` has no such field and cannot grow one without changing the
//! shape of every `Op::Create` literal in the workspace; its undo record
//! lives in `Tree::created_by` instead, which is where the paper keeps it
//! (on the log record, not on the transmitted op). The pairing rule is the
//! same either way: **`undo_op` must be the exact inverse of `do_op` for
//! every op, including the ones `do_op` ignored** (`do_undo_op_inv`,
//! Kleppmann et al. 2022 / `Move.thy`). `apply_op`'s reorder loop assumes
//! nothing less.

use super::Tree;
use crate::op::{LogOp, Op};

impl Tree {
    /// Apply one op to the materialized tree, mutating it in place.
    ///
    /// For [`Op::Move`] and [`Op::SetProp`] this also fills in the
    /// `old_*` fields of the passed-in `LogOp` so that [`Self::undo_op`]
    /// can later revert exactly.
    ///
    /// Move with a cycle is a no-op on the materialized tree — but the
    /// caller still appends the `LogOp` to the log unchanged. Reordering
    /// may turn the same op into a non-cycle move later.
    pub fn do_op(&mut self, log_op: &mut LogOp) {
        // Read before the `&mut` borrow of `log_op.op` below. `Op::Create`
        // needs it: the undo record it cannot carry is keyed by this ts.
        let ts = log_op.ts;
        match &mut log_op.op {
            Op::Move {
                node,
                new_parent,
                position,
                old_parent,
                old_position,
            } => {
                // Capture pre-state for undo. If the node doesn't yet
                // exist locally (Move arrived before its Create), the
                // op is a complete no-op on the tree — but it still
                // ends up in the log, and `undo_op` will skip it via
                // the "parent matches new_parent" check.
                match self.nodes.get(node) {
                    Some((p, pos)) => {
                        *old_parent = *p;
                        *old_position = pos.clone();
                        if !self.creates_cycle(*node, *new_parent) {
                            self.nodes.insert(*node, (*new_parent, position.clone()));
                        }
                        // If a cycle would be created, no mutation of
                        // the tree. The op still ends up in the log.
                    }
                    None => {
                        // Sentinel `old_*` values so undo's parent
                        // check (`parent == new_parent`) cannot match,
                        // making undo a guaranteed no-op.
                        *old_parent = *new_parent;
                        *old_position = position.clone();
                    }
                }
            }
            Op::Edit { .. } => {
                // Block text content lives outside `Tree` (in a Yrs `Doc`
                // managed by `Workspace`). Tree-level do_op is a no-op for
                // `Edit`; the caller dispatches the update separately.
            }
            Op::SetProp {
                node,
                key,
                value,
                old_value,
            } => {
                let key_owned = key.clone();
                *old_value = self.properties.get(&(*node, key_owned.clone())).cloned();
                match value {
                    Some(v) => {
                        self.properties.insert((*node, key_owned), v.clone());
                    }
                    None => {
                        self.properties.remove(&(*node, key_owned));
                    }
                }
            }
            Op::Create {
                node,
                parent,
                position,
            } => {
                // Idempotent: if the node already exists, keep its current
                // parent/position. The Create only seeds initial placement.
                //
                // Cycle guard, exactly like Move: a reordered Create whose
                // `parent` is already a descendant of `node` (a prior Move
                // re-parented something under `node`, putting `parent` below
                // it) would close a loop. That is a no-op on the materialized
                // tree — but the LogOp still goes into the log unchanged, so a
                // later reorder can turn it into a valid Create.
                //
                // Three outcomes, and `undo_op` must tell them apart:
                //
                // | outcome                       | inverse            |
                // |-------------------------------|--------------------|
                // | inserted (absent, no cycle)   | remove the node    |
                // | skipped — node already there  | nothing            |
                // | skipped — would cycle         | nothing            |
                //
                // Only the first has `remove` as its inverse. Recording
                // which one happened is what `Op::Move` gets from its
                // `old_parent` field and what the paper gets from
                // `LogMove.oldp` (Fig. 4 l.28); `Op::Create` carries
                // neither, so the answer goes in `self.created_by`, keyed by
                // node and holding the ts of the op that owns that node's
                // existence right now.
                //
                // The paper records `oldp` unconditionally, *outside* the
                // cycle branch, because for `Move` the skip cases still need
                // it: their correct inverse is "restore what was there",
                // which is a value. Here both skip cases have the **identity**
                // as their inverse, and the identity needs no value — so
                // absence of an entry for this ts is itself the record, and
                // it cannot go stale: an entry is written only by the insert
                // below and removed only by that insert's own undo, so
                // `created_by[node]` always names the op the node currently
                // owes its existence to.
                if !self.nodes.contains_key(node) && !self.creates_cycle(*node, *parent) {
                    self.nodes.insert(*node, (*parent, position.clone()));
                    self.created_by.insert(*node, ts);
                }
            }
            Op::SetCollapsed {
                node,
                value,
                old_value,
            } => {
                // Capture previous state for `undo_op`. Membership in
                // `self.collapsed` is the source of truth (presence =
                // collapsed, absence = expanded).
                *old_value = self.collapsed.contains(node);
                if *value {
                    self.collapsed.insert(*node);
                } else {
                    self.collapsed.remove(node);
                }
            }
            Op::SnoozeRemind {
                node,
                until_ms,
                old_until_ms,
            } => {
                // Same shape as `SetCollapsed`: capture the previous
                // value, then set-or-clear. Absence from `self.snoozed`
                // is "not snoozed", so `None` is a `remove`.
                *old_until_ms = self.snoozed.get(node).copied();
                match until_ms {
                    Some(ms) => {
                        self.snoozed.insert(*node, *ms);
                    }
                    None => {
                        self.snoozed.remove(node);
                    }
                }
            }
        }
    }

    /// Revert one previously-applied op, using what [`Self::do_op`]
    /// recorded on the way through: the `old_*` fields for every variant
    /// that has them, and `Tree::created_by` for [`Op::Create`], which has
    /// none.
    ///
    /// **This must be the exact inverse of `do_op`, including for ops
    /// `do_op` ignored.** [`Self::apply_op`]'s reorder loop assumes nothing
    /// less, and the paper's correctness argument rests on it
    /// (`do_undo_op_inv`, whose only hypothesis is a well-formed tree).
    ///
    /// For [`Op::Move`] we only revert if the current parent matches the
    /// move's `new_parent` — otherwise the original `do_op` was a cycle
    /// no-op and there's nothing to undo.
    ///
    /// For [`Op::Edit`] tree-level undo is a no-op; Yrs handles its own
    /// merge semantics, accepting that we can't bit-for-bit reverse a
    /// text update that interleaved with concurrent edits.
    pub fn undo_op(&mut self, log_op: &LogOp) {
        match &log_op.op {
            Op::Move {
                node,
                new_parent,
                old_parent,
                old_position,
                ..
            } => {
                if self.parent(*node) == Some(*new_parent) {
                    self.nodes
                        .insert(*node, (*old_parent, old_position.clone()));
                }
                // else: move was a cycle no-op or was already reverted;
                // tree state is consistent.
            }
            Op::Edit { .. } => {
                // See module docs.
            }
            Op::SetProp {
                node,
                key,
                old_value,
                ..
            } => {
                let k = (*node, key.clone());
                match old_value {
                    Some(v) => {
                        self.properties.insert(k, v.clone());
                    }
                    None => {
                        self.properties.remove(&k);
                    }
                }
            }
            Op::Create { node, .. } => {
                // Paper Fig. 4 l.33/l.35, split on whether *this* op is the
                // one that brought the node into existence:
                //
                // - it did (`created_by[node] == ts`, the paper's
                //   `oldp = None`) → remove the node;
                // - it did not → `do_op` left the tree alone, so undo must
                //   too. The paper's `Some((oldp, oldm))` branch re-inserts
                //   the placement `do_op` found; here that placement is
                //   still in `self.nodes` untouched, so "re-insert it" and
                //   "do nothing" are the same edit.
                //
                // Removing unconditionally is the bug this guard exists for:
                // `do_op(Create)` is idempotent, so a duplicate `Create` —
                // routine, since page roots are addressed by the
                // deterministic `NodeId::from_slug` — used to delete a node
                // that somebody else's `Create` had made, taking any
                // interleaved `Move` down with it. See
                // `docs/rfcs/0263-create-is-invertible.md`.
                if self.created_by.get(node) == Some(&log_op.ts) {
                    self.nodes.remove(node);
                    self.created_by.remove(node);
                }
            }
            Op::SetCollapsed {
                node, old_value, ..
            } => {
                // Restore the previous membership captured by `do_op`.
                // `undo_op` on a never-applied `LogOp` (one whose
                // `old_value` is still the default `false`) reduces to
                // "make sure the node is not collapsed", which is a
                // no-op when the materialised state matches.
                if *old_value {
                    self.collapsed.insert(*node);
                } else {
                    self.collapsed.remove(node);
                }
            }
            Op::SnoozeRemind {
                node, old_until_ms, ..
            } => {
                // Restore the value captured by `do_op`. On a
                // never-applied `LogOp` (`old_until_ms` still at its
                // `None` default) this reduces to "make sure the node
                // is not snoozed", a no-op when state already matches.
                match old_until_ms {
                    Some(ms) => {
                        self.snoozed.insert(*node, *ms);
                    }
                    None => {
                        self.snoozed.remove(node);
                    }
                }
            }
        }
    }
}
