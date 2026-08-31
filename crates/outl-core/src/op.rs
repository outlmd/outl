//! Op log primitives: the `Op` enum and the `LogOp` envelope.
//!
//! Every mutation to the materialized tree is expressed as an `Op` wrapped
//! in a `LogOp` (HLC + actor + op). The op log is the source of truth;
//! the tree is a projection.
//!
//! Adding a new `Op` variant is non-trivial — see `/new-op` slash command.

use crate::fractional::Fractional;
use crate::hlc::Hlc;
use crate::id::{ActorId, NodeId};
use crate::property::PropValue;
use serde::{Deserialize, Serialize};

/// A single mutation to the outline.
///
/// `Move` is the operation whose concurrent semantics are the heart of the
/// algorithm. `Edit` carries a Yrs binary update for block content.
/// `SetProp` and `Create` round out the surface.
///
/// Note: there is no `Delete` variant. Deletion is `Move(node, TRASH_ROOT)`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    /// Move a node to a new parent and/or position.
    ///
    /// `old_parent` and `old_position` are populated by `do_op` so that
    /// `undo_op` can revert. They are not meaningful before the op is
    /// applied for the first time.
    Move {
        /// The node being moved.
        node: NodeId,
        /// New parent of the node.
        new_parent: NodeId,
        /// Position of the node among siblings of `new_parent`.
        position: Fractional,
        /// Filled by `do_op`. Required for `undo_op`.
        ///
        /// **Only `undo_op` may read this.** In the paper it is not part
        /// of the transmitted operation at all — `Move` carries four
        /// fields and `oldp` lives on the local `LogMove` record (Fig. 4,
        /// §3.2). Here the two types are one, so the field rides the
        /// JSONL, but it stays what the paper says it is: the originating
        /// replica's derivation against *its* tree at *first*
        /// application. A reorder recomputes it in the resident log and
        /// cannot rewrite the persisted line (`redo_op`, §3.4), and every
        /// device that ingests the line overwrites it in `do_op` before
        /// reading it.
        ///
        /// So a reader of the log **as data** — a page history, `doctor`,
        /// a human — must never trust it, and this is more tempting now
        /// than it used to be: before `Workspace::apply` was fixed to
        /// persist what `do_op` derived, the field was uniformly wrong
        /// (`root` in 65,141 of 65,703 stored Moves) and obviously junk.
        /// It is plausible now, and still not authoritative. Fold the
        /// parent trail from `Create.parent` / `Move.new_parent`, which
        /// describe the op's own effect.
        old_parent: NodeId,
        /// Filled by `do_op`. Required for `undo_op`.
        old_position: Fractional,
    },

    /// Apply a Yrs binary update to a block's content.
    Edit {
        /// The block whose content is edited.
        node: NodeId,
        /// Yrs `encode_update_v1` bytes.
        text_op: Vec<u8>,
    },

    /// Set or clear a property on a node.
    ///
    /// `old_value` is populated by `do_op` for undo.
    SetProp {
        /// The node owning the property.
        node: NodeId,
        /// Property key.
        key: String,
        /// `None` removes the property.
        value: Option<PropValue>,
        /// Filled by `do_op` for `undo_op`. Same rule as
        /// [`Op::Move::old_parent`]: local derivation, undo-only, never
        /// authoritative to a reader of the log as data.
        old_value: Option<PropValue>,
    },

    /// Create a new node under a given parent and position.
    ///
    /// Idempotent: re-applying for an already-existing node is a no-op.
    Create {
        /// The new node's id.
        node: NodeId,
        /// Initial parent.
        parent: NodeId,
        /// Initial position among siblings.
        position: Fractional,
    },

    /// Set the **collapsed** (folded) flag of a node.
    ///
    /// Controls whether the block's children are hidden in the outline
    /// view. UI presentation, but globally meaningful — folding a
    /// block on one device shows up folded on every other device.
    ///
    /// **Going through `Op` is the canonical path for any per-block
    /// state that must converge between devices.** Writing such state
    /// straight to a sidecar would lose under iCloud / Syncthing's
    /// last-write-wins-per-file semantics; the op log gives each
    /// device its own `ops-<actor>.jsonl` and lets the CRDT merge
    /// concurrent flips by HLC ordering. Idempotent re-apply of the
    /// same `LogOp` is a no-op (the HLC dedup at the top of
    /// [`crate::tree::Tree::apply_op`] guarantees this).
    ///
    /// `old_value` is populated by `do_op` for `undo_op`.
    SetCollapsed {
        /// The node being folded / unfolded.
        node: NodeId,
        /// Desired flag.
        value: bool,
        /// Filled by `do_op` for `undo_op`.
        old_value: bool,
    },

    /// Silence a block's `remind::` rule until a wall-clock instant.
    ///
    /// Snooze is per-block user intent that **must** converge: snoozing
    /// a nagging TODO on the phone has to silence the same block on the
    /// desktop, so it goes through the op log like every other shared
    /// state (root `CLAUDE.md` invariant #7). The device-local half —
    /// "did I already fire this one here" — deliberately does not; it's
    /// a local cache each device rebuilds.
    ///
    /// `until_ms` is Unix epoch **milliseconds**, not an [`Hlc`]. The
    /// envelope's `ts` already carries the ordering; this field is a
    /// point on the user's calendar, and conflating the two would make
    /// a clock-skewed device's snooze resolve to the wrong wall time.
    /// `None` clears the snooze (the "un-snooze" / reschedule path).
    ///
    /// `old_until_ms` is populated by `do_op` for `undo_op`.
    SnoozeRemind {
        /// The block whose reminder is being silenced.
        node: NodeId,
        /// Resume firing at-or-after this Unix-epoch millisecond.
        /// `None` clears any existing snooze.
        until_ms: Option<u64>,
        /// Filled by `do_op` for `undo_op`.
        old_until_ms: Option<u64>,
    },
}

/// Extract the `NodeId` an op targets, if any. Every `Op` variant
/// carries one — there is no op that touches zero nodes. Returns
/// `Option` so callers can `filter_map` cleanly. Used by the migrate
/// CLI to route ops to per-page shards (RFC #137 Phase B).
pub fn op_node(op: &Op) -> Option<NodeId> {
    match op {
        Op::Create { node, .. }
        | Op::Move { node, .. }
        | Op::Edit { node, .. }
        | Op::SetProp { node, .. }
        | Op::SetCollapsed { node, .. }
        | Op::SnoozeRemind { node, .. } => Some(*node),
    }
}

/// An op wrapped with its HLC and actor.
///
/// `LogOp`s are what is stored, sorted, and exchanged between peers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogOp {
    /// HLC timestamp; defines total order.
    pub ts: Hlc,
    /// Originating actor (also embedded inside `ts` for tiebreak).
    pub actor: ActorId,
    /// The mutation itself.
    pub op: Op,
}

/// What kind of payload an [`Op`] variant carries.
///
/// Exists for one assertion: **asset bytes never enter the op log**
/// ([RFC 0202](../../../docs/rfcs/0202-file-assets.md)). That was true by
/// construction — `Op` simply had no variant for it — and "true by
/// construction" is the kind of truth that stops being true in a commit
/// nobody flags, because nothing fails when it does
/// ([issue #213](https://github.com/outlmd/outl/issues/213), item 2).
///
/// RFC 0202 frames the whole asset design as a deliberate, *expiring*
/// exception to invariant 7. An exception that valuable deserves a guard
/// that fires when the thing it excepted comes back.
#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
enum PayloadShape {
    /// Ids, positions, flags, timestamps, a property key and value.
    /// Bounded, structural, and describes an edit rather than carrying a
    /// file.
    Structural,
    /// An opaque byte blob whose size the user controls.
    ///
    /// The string is the justification, and writing one is the point: a
    /// second variant landing here should require an author to state,
    /// in the diff, why the op log is the right home for those bytes.
    OpaqueBytes(&'static str),
}

#[cfg(test)]
mod payload_policy {
    use super::*;

    /// Classify a variant's payload. **Exhaustive on purpose** — a new
    /// [`Op`] variant does not compile until someone has said which side
    /// of RFC 0202 it falls on.
    fn payload_shape(op: &Op) -> PayloadShape {
        match op {
            Op::Move { .. }
            | Op::Create { .. }
            | Op::SetCollapsed { .. }
            | Op::SnoozeRemind { .. } => PayloadShape::Structural,

            // A property value is user text, and bounded by being text.
            // It is not a file: `SetProp` cannot express "here are the
            // bytes of a PDF" without someone base64-ing one into a
            // value, which is the abuse this policy exists to make
            // visible rather than a shape the variant invites.
            Op::SetProp { .. } => PayloadShape::Structural,

            Op::Edit { .. } => PayloadShape::OpaqueBytes(
                "a Yrs update IS the block's text state — the thing the CRDT \
                 merges. It is not a file the log is carrying on behalf of \
                 something else, which is the distinction RFC 0202 draws.",
            ),
        }
    }

    /// One sample per variant. Kept complete by
    /// `every_variant_has_a_sample` below.
    fn one_of_each() -> Vec<Op> {
        let node = NodeId::new();
        vec![
            Op::Move {
                node,
                new_parent: NodeId::new(),
                position: Fractional::between(None, None),
                old_parent: NodeId::new(),
                old_position: Fractional::between(None, None),
            },
            Op::Edit {
                node,
                text_op: vec![1, 2, 3],
            },
            Op::SetProp {
                node,
                key: "remind".into(),
                value: None,
                old_value: None,
            },
            Op::Create {
                node,
                parent: NodeId::new(),
                position: Fractional::between(None, None),
            },
            Op::SetCollapsed {
                node,
                value: true,
                old_value: false,
            },
            Op::SnoozeRemind {
                node,
                until_ms: Some(1),
                old_until_ms: None,
            },
        ]
    }

    /// RFC 0202's load-bearing claim, as an assertion rather than a
    /// property of the enum's current shape.
    ///
    /// If this fails, someone added a variant that carries opaque bytes.
    /// That may be right — but it is a decision about invariant 7 and
    /// RFC 0202, not a refactor, and it should not be possible to make
    /// it without noticing.
    #[test]
    fn only_yrs_text_may_put_opaque_bytes_in_the_op_log() {
        let carriers: Vec<_> = one_of_each()
            .iter()
            .filter(|op| matches!(payload_shape(op), PayloadShape::OpaqueBytes(_)))
            .map(std::mem::discriminant)
            .collect();

        let edit = Op::Edit {
            node: NodeId::new(),
            text_op: Vec::new(),
        };
        assert_eq!(
            carriers,
            vec![std::mem::discriminant(&edit)],
            "exactly one Op variant may carry opaque bytes, and it must be \
             Op::Edit. A new byte-carrying variant means asset (or other file) \
             bytes can now enter the op log — see docs/rfcs/0202-file-assets.md \
             and root CLAUDE.md invariant 7 before changing this test.",
        );
    }

    /// Guards the guard: a variant missing from `one_of_each` would make
    /// the assertion above pass without ever looking at it.
    ///
    /// `payload_shape`'s `match` already forces a *classification* at
    /// compile time; this forces the *sample*, which is what the runtime
    /// assertion actually reads.
    #[test]
    fn every_variant_has_a_sample() {
        let samples = one_of_each();
        let distinct: std::collections::HashSet<_> =
            samples.iter().map(std::mem::discriminant).collect();
        assert_eq!(
            distinct.len(),
            samples.len(),
            "one_of_each lists a variant twice",
        );
        assert_eq!(
            samples.len(),
            6,
            "Op has a new variant — add it to one_of_each (and classify it in \
             payload_shape, which will not compile until you do)",
        );
    }
}
