# RFC 0263 — `Op::Create` records whether it created anything

| | |
|---|---|
| **Status** | Draft |
| **Issue** | none yet — found while auditing `undo_op` for the op-log compaction work. The number is **not** an issue number (see "Numbering" below) |
| **PR** | none yet |
| **Date** | 2026-09-11 |
| **Reference doc** | [`docs/crdt.md`](../crdt.md) → `do_op` / `undo_op` |
| **Invariant** | root `CLAUDE.md` invariants 1 (op log is source of truth) and 3 (CRDT follows Kleppmann 2022 literally); `outl-core/CLAUDE.md` invariants 1 (convergence), 2 (commutativity after reordering), 5 (no silent loss) |
| **Guarded by** | `a_repeat_create_converges_under_reordering`, `a_repeat_create_does_not_swallow_an_interleaved_move`, `every_delivery_order_of_a_repeat_create_agrees`, `a_duplicate_create_does_not_resurrect_a_trashed_node`, `undo_op_is_the_inverse_of_do_op_for_every_op` (`crates/outl-core/tests/create_undo_symmetry.rs`); `undoing_a_post_snapshot_duplicate_create_keeps_the_snapshotted_node` (`crates/outl-core/tests/create_undo_after_snapshot_boot.rs`); the whole of `crates/outl-core/tests/convergence_property.rs`, whose generator (`crates/outl-core/tests/convergence_gen/`) now emits duplicate `Create`s |

## Why

**A page that two devices created independently does not converge.**

Page and journal roots are addressed by `NodeId::from_slug` — a deterministic hash of the slug — *on purpose*, so that two devices which create the same page offline land on one node instead of two competing roots.
`outl_actions::page::open_or_create` says so in a comment.
The cost of that design is that **a duplicate `Op::Create` for one node id is routine**: every journal opened on two devices produces one, each carrying whatever sibling position its own device computed.

`do_op(Op::Create)` is idempotent — it skips a node that already exists.
`undo_op(Op::Create)` removed the node unconditionally.
So undoing a `Create` that created nothing deleted a node somebody *else's* `Create` had made.
`apply_op`'s reorder loop needs `undo_op` to be the exact inverse of `do_op` for the op it is handed, including for ops `do_op` ignored.

**How often the reorder loop actually runs, measured.**
On a real 217,811-op log the undo window is **0 for 217,663 of 217,663 ops** — every local path feeds `apply_op` already sorted, so a full replay never undoes anything.
That number is worth stating because it sets where this defect lives, and it is not "everywhere":

- **A single-device replay never hits it.** Boot from an ordered log, and `undo_op(Create)` is never called at all.
- **Out-of-order delivery does.** A peer's op arriving below the local tail is exactly the case the loop exists for, and it is the same case in which a duplicate `Create` exists — because a duplicate `Create` *comes from* a second device.

So the defect is latent on one device and reachable precisely when a workspace becomes multi-device, which is also when duplicate `Create`s start being produced.
`undoing_a_post_snapshot_duplicate_create_keeps_the_snapshotted_node` drives it through `Workspace::apply` rather than through a hand-built `Tree`, so the reachable path is pinned and not merely argued.

Minimally:

```text
Create(n, P, "a") @1
Move(n, P, "t")   @2
Create(n, P, "u") @3
```

Delivered `[1, 2, 3]` the node ends at `"t"`.
Delivered `[3, 1, 2]` it ends at `"u"`.

And the second number understates it.
Tracing the divergent order: undoing `Create@3` removes `n`; `do_op(Move@2)` then finds no node to move (outl's `Move` is not total over unborn nodes) and becomes a **complete** no-op; the replay of `Create@3` re-materializes `n` at the creation position.
The `Move` is still in the log — invariant 5 holds — but its effect is gone from the tree.
That is silent loss of a user's edit, not a cosmetic ordering difference.

The user-visible worst case is not ordering at all.
`a_duplicate_create_does_not_resurrect_a_trashed_node` covers it: device A creates page `ideas` and deletes it (`Move` → trash); device B, which never saw either op, creates `ideas` locally — the same node id.
Correct merge: HLC order puts B's `Create` last, where it is a no-op, so the page stays deleted everywhere.
Before this change, one delivery order put the page back under `root` and another left it in the trash — a deleted page that reappears on one device only, and whose reappearance survives a restart because the wrong tree is what gets written to the snapshot cache and served to peers by the iroh snapshot responder.

The defect predates all current work.
It was never caught because the convergence property suite refused to generate the shape: `lower()` turned a second `Create` for a node into a `Move`, with a comment calling a duplicate `Create` "NOT a well-formed CRDT input".
That comment is wrong in both halves — see "Why not the alternatives".

## What we chose

Record, per applied `Op::Create`, whether *that* op is the one that brought the node into existence, and make `undo_op` remove the node only then.

The owner is `Tree::created_by: HashMap<NodeId, Hlc>` (`crates/outl-core/src/tree/mod.rs`), written by `do_op` and read **only** by `undo_op`.
`do_op` computes the verdict *before* the branch that may skip, mirroring the position of `get_parent tree c` at Fig. 4 l.28 — a value written inside the `if` is a value that goes stale on the next replay.

This is the paper's `LogMove.oldp`, in outl's vocabulary.
Kleppmann et al. 2022 has one operation, `Move`, and its `do_op` records `oldp : (parent, meta) option` for every application:

- `oldp = None` — "the node did not exist" — and `undo_op` **removes** it (Fig. 4 l.33);
- `oldp = Some (p, m)` — and `undo_op` restores that placement (Fig. 4 l.35).

`Op::Create`'s three outcomes map onto those two equations exactly:

| `do_op(Create)` outcome | paper's `oldp` | inverse |
|---|---|---|
| inserted (node absent, no cycle) | `None` | remove the node |
| skipped — node already existed | `Some(p, pos)` | restore `(p, pos)` — which is still in `nodes`, untouched, so this is the identity |
| skipped — would close a cycle | `None` | remove — a no-op, the node is absent |

So one bit ("was it me?") answers all three, and `created_by` stores it as the ts of the winning `Create`.

**Why the record is not a field on `Op::Create`.**
In the paper it is not part of the transmitted operation at all: `Move t p m c` carries four fields and `oldp` lives on the local `LogMove` **log record** (§3.2).
outl merged those two types, which is why `Op::Move::old_parent` rides the JSONL — and that field's own doc comment is a paragraph of apology for what riding the wire has cost readers of the log as data (65,141 of 65,703 stored `Move` ops carry a wrong `old_parent`).
Adding a second such field would propagate that mistake to the most common op in the log, and would put a field on the wire that no peer may trust and every `do_op` overwrites before reading.
Keeping `Create`'s answer on the side the paper keeps it is the more faithful of the two options, not the lesser one.

It is also the only option available without a workspace-wide edit: `Op::Create` is constructed as a struct literal in 93 places across 6 crates, and Rust has no default for an enum-variant field.
That is a real cost of the alternative, but it is not the argument for this design — see "Why not the alternatives".

**Why an empty `created_by` is safe.**
It is undo bookkeeping, not materialized state, so it is absent from `Tree::snapshot_parts` and left empty by `Tree::from_parts`.
That is sound because of a property of the boot paths rather than of the map: **every op that reaches the resident log passes through `do_op` first** — on the full-replay path, on the snapshot path (the delta is replayed through `apply_op`, and `Workspace` refuses a snapshot whose delta contains an op at or below the body's high-water mark), and in `Workspace::apply`.
`apply_op` can only ever undo ops in the resident log, so an op folded into a snapshot body is unreachable by any undo.
A missing entry also fails in the safe direction: `undo_op` keeps the node rather than deleting it.

## Why not the alternatives

**Add `old_placement: Option<(NodeId, Fractional)>` to `Op::Create`, with `#[serde(default)]`.**
The shape the `paper-verifier` agent recommended, and the one both `CLAUDE.md` anti-pattern lists point at ("❌ Add an `Op` variant without `old_*` fields", "❌ Storing op log fields outside the `Op` variant").
It is uniform with `Move` / `SetProp` / `SetCollapsed` / `SnoozeRemind`, and its migration is genuinely free: an op already on disk deserializes to `None`, and `do_op` overwrites the field before any `undo_op` can read it, so the default is never actually read.
It was not chosen for two reasons, in this order.
First, it puts undo-only local derivation on the sync surface for the most frequent op in the log, which is the specific mistake `Op::Move::old_parent` already made and is documented as regretting — invariant 9's question, "where does the problem live now", answers *on the wire, where a reader of the log as data will misread it*.
Second, the change is not local: 93 struct literals across `outl-core` (74), `outl-actions` (9), `outl-sync-iroh` (6), `outl-ws` (2), `outl-md` (1) and `outl-cli` (1) stop compiling, most of them adding the token `None`.
**Both designs are correct.** If the uniformity is judged to be worth the wire field, converting is mechanical: move the `created_by` lookup into the field and delete the map. The tests in **Guarded by** do not change.

**Derive `Create` as a `Move` of a node that does not exist yet, and delete `Op::Create` (the paper's actual recommendation, §3.6: "no separate operations for node creation and deletion are needed, since a node is implicitly created when it is first moved").**
This is the right end state and it is not one change.
It requires `do_op(Move)` to become total over unborn nodes (Fig. 4 l.30 inserts unconditionally after the ancestor check, where outl's `None` arm is a complete no-op), which means `Op::Move::old_parent` must itself become an `Option` — the same problem on a bigger variant — and it **reinterprets existing logs**: `Create` stops being first-write-wins and becomes last-write-wins, so any workspace with a `Create` whose HLC is above a `Move` on the same node materializes differently after the upgrade than before.
It also makes a `Move` for a node whose `Create` has not arrived materialize a phantom node, which in outl is a visible empty bullet in the `.md` projection rather than an abstract tuple.
And it removes the property `outl compact`'s predicate is currently built on (condition 3 requires a sole `Create`), while that code is being written.
Staged, it is: (1) this RFC; (2) an RFC for `Move` totality with the phantom-node question answered for the projection layer; (3) compaction's predicate reworked off `Create` identity; (4) `Op::Create` demoted to a legacy deserialization-only variant.
Nothing here blocks that; `created_by` is deleted along with the variant in step 4.

**Decide at undo time from current tree state** — remove the node only if its placement equals this `Create`'s `(parent, position)`.
Needs no new state, and it is wrong.
Two devices creating the same slug frequently compute the *same* position (both append to an empty root), and then the guard cannot tell a duplicate from the original and deletes anyway.
Tried and rejected; it narrows the bug rather than fixing it, which is worse than leaving it visible.

**Re-derive at undo time by scanning the remaining log** for an earlier `Create` of the node.
The log at that moment does hold exactly the ops with a smaller ts, so the question is answerable in principle — but "is there an earlier `Create`" is not the right question (an earlier `Create` that was itself a cycle no-op materialized nothing), and the right question is a full re-materialization.
`O(log)` per undo, for a bit `do_op` already knew.

**Keep the convergence suite's exclusion and call duplicate `Create`s malformed.**
This was the status quo, and it is what hid the defect.
Two claims in that comment are false: a duplicate `Create` is well-formed (the lowest-HLC one wins, which is a property of the op *set*, not of delivery order), and it is not exotic — deterministic page ids make it the most common duplicate in production.

## The opposite direction

**What this makes worse.**
`Tree` now carries one `NodeId → Hlc` entry per live node: 16 bytes of key, 32 of value, no per-entry heap allocation.
On the 67k-node reference workspace that is roughly 3 MB of steady-state RSS that was not there before, against a `nodes` map already costing more.
It is bounded by live nodes, not by log length, so it does not reintroduce the O(log size) boot cost RFC 0137 removed — but it is not free, and a workspace an order of magnitude larger pays proportionally.
The `Op::Create` field alternative costs nothing resident and pays on disk and on the wire instead; neither is free, and this RFC picks the one that does not touch the sync surface.

**The mirrored case.**
The fix makes `undo_op(Create)` *refuse* to remove in a case where it used to remove.
The mirror is therefore: can it now fail to remove a node it should have removed — leaving a node in the tree that no op created?
That requires `created_by` to be missing an entry for a `Create` that did insert, which requires an op to be in the resident log without having gone through `do_op`.
Nothing does that today, and the reasoning is written on the field so the next person to add a boot path sees the obligation.
`undoing_a_post_snapshot_duplicate_create_keeps_the_snapshotted_node` pins the one shape where it could plausibly happen.
If it ever does happen, the failure is a node that survives an undo it should not have, which the next full-replay boot corrects — as against the pre-fix failure, which deleted content the op log still held and then wrote the deletion into the snapshot cache.

**Nothing changes for the user.**
No op format changes, no on-disk format changes, no migration, no behaviour change on any workspace that never produced a duplicate `Create`.
A workspace that *has* already diverged does not self-heal from this change alone: the divergent tree was materialized from a correct log, so a full replay on the fixed binary converges it, and a snapshot cache written from the divergent tree is dropped when a peer's differs.
Nobody is told; there is no detector for "this workspace diverged in the past".
That gap is real and is left open deliberately — see "Scope".

## How it cannot regress

1. **The invariant.**
   Root `CLAUDE.md` invariant 3 already requires `do_op` / `undo_op` / `apply_op` / `creates_cycle` to match the paper literally, and `outl-core/CLAUDE.md` already forbids adding an `Op` variant whose undo is impossible.
   Neither said the thing this bug needed: **`undo_op` must be the exact inverse of `do_op` for every op, including the ones `do_op` ignored.**
   That sentence now leads the module doc of `crates/outl-core/src/tree/op.rs`, and belongs in `outl-core/CLAUDE.md` under "The five invariants" (proposed text is in the handover note for this change; that file is within a handful of bytes of a hard 40,000-**byte** limit — the guard counts bytes, not characters — and was not edited here).

2. **The tests.**
   `undo_op_is_the_inverse_of_do_op_for_every_op` is the one that matters: it asserts the property across every `Op` variant and both its effective and no-op paths, so the next variant to get this wrong fails on a named lemma rather than on a shrunk proptest seed a year later.
   It is named after the property and not after the repro on purpose — a per-variant test keeps missing the next variant, the same argument `tests/stored_op_matches_the_log.rs` makes about `old_*`.
   The four scenario tests pin the user-visible shapes, including the resurrected-trashed-page one.
   `crates/outl-core/tests/convergence_property.rs` is the ratchet: its generator (now `convergence_gen/`, its single owner) emits duplicate `Create`s, and its positions vary per step instead of being the constant `"m"` — with a constant position a node could be placed at the wrong op's position and still compare equal, which is precisely the divergence this defect produces.
   Reverting the one-line guard in `undo_op` fails three of that file's five properties, plus all six scenario/inverse tests, with proptest shrinking to a two-op `Create, Create` program.

A note for whoever reads the `.proptest-regressions` file: `convergence_property.rs` was split while this change was in flight (it was heading for the 900-line hard stop). The generator moved to `convergence_gen/` and the two deterministic `Op::Create` cycle regressions moved to `create_tree_invariants.rs`, both under their original names. The five proptest properties stayed put, so the persisted seeds still name tests in the file that owns them.

## What the review agents said

Both were run because root `CLAUDE.md` mandates them for a change of this shape, and both are recorded here rather than left in a transcript.

**`paper-verifier` — BLOCKED, then satisfied.**
It confirmed the defect against Fig. 4 and supplied the equation this change implements: the paper's `do_op` records `oldp : (parent, meta) option` on *every* application, `undo_op` removes the node on `None` (l.33) and restores the placement on `Some` (l.35), and the `None` case **is** the paper's own "this op created the node" marker.
So outl's bug is not a missing feature; it is a variant that skipped a record the paper already specifies.
It also flagged the shape of the mistake to avoid: write the verdict where a replay cannot leave a stale one behind.
Two further notes from it are carried below in **Scope**.

**`crdt-invariant-checker` — PASS.**
All five `outl-core` invariants OK: convergence, commutativity after reordering, idempotency, tree invariant, no silent loss.
It found no path that puts an op in the resident `OpLog` without a `do_op`, and no path that builds a `Tree` with nodes but no `created_by` other than `Tree::from_parts`, which is the case reasoned about above.
No leak: an entry exists exactly while its node does, so growth is bounded by live nodes.
No flakiness in the widened generator.

Two caveats it raised, recorded because a caveat that only lives in a transcript is a caveat nobody reads in six months:

1. **The snapshot-adoption guard is load-bearing for this change, and does not say so.**
   `Workspace::boot_from_snapshot` refuses a snapshot whose delta contains any op at or below the body's high-water mark.
   That guard was written for replay ordering, and `created_by`'s soundness now leans on it too: it is what guarantees the resident log holds no op that could undo past the snapshot body.
   The comment there names only its original reason.
   `workspace.rs` is owned by another agent in this round, so the line was **not** added; the exact text is in the handover note.
   Until it lands, a future edit that relaxes the guard would break this RFC silently — nothing fails.

2. **A side effect on compaction, in the safe direction.**
   `outl compact` may drop an op only when dropping it changes nothing.
   A duplicate `Create` is such an op — but before this change that was true only of the *forward* replay, since undoing it was destructive.
   With the inverse fixed, "dropping it changes nothing" is now true of the undo direction as well.
   This RFC does not act on that, and compaction's predicate (condition 3, a sole `Create`) is unchanged; it is noted so whoever revisits the predicate knows the ground moved under it.

## Scope

Not covered here:

- **Making `Move` total over unborn nodes**, and the deletion of `Op::Create` that follows from it (paper §3.6). Staged above; it needs its own RFC because it reinterprets existing logs.
- **Detecting or repairing a workspace that already diverged** through this bug. A full-replay boot on a fixed binary converges the tree, but nothing reports that it happened, and `outl doctor` has no check for it. Needs an issue.
- **`outl compact`'s predicate**, which requires a sole `Create` per node and is therefore unaffected — a multi-`Create` node is excluded from compaction either way. It must not be weakened to "fix" this; the fix is here.
- **`creates_cycle`'s `max_steps` bailout**, which makes the predicate's answer depend on `nodes.len()` under an already-malformed tree. Unreachable while `apply_ops_acyclic` holds, flagged by `paper-verifier` as defensive rather than a live defect, and left alone. Its `return true` is also the one line keeping `creates_cycle` off the 100 % coverage target — it is dead in any debug build, because the `debug_assert!` above it fires first. That is a property of the pair of statements, not a missing test.
- **Seeding `HlcGenerator` from the op log at boot.** The generator starts from the wall clock and is never seeded from the log, so after a backwards clock jump a newly minted op can sort *before* ops already on disk (11 events on the real workspace, the worst 2.7 days back). Adjacent to this change and **separately owned** — it is assigned to another agent, in `hlc.rs` / `workspace.rs`. It is not a convergence bug: `apply_op` reorders a low-stamped op into place and every replica still lands on the same tree. What it costs is `OpLog::append`'s debug assertion and any reader that treats HLC order as wall-clock order.
- **The `apply_op` reorder-window optimization**, which is **refuted by measurement and must not be pursued**: the window is 0 for 217,663 of 217,663 ops on the real log, and real tree depth is p50=3, max=8. There is nothing there to optimize. This change is aimed at correctness only.

## Numbering

`docs/rfcs/README.md` says the RFC number **is** the issue number it resolves.
This document has no issue yet and takes 0263 by instruction (three above the highest existing number), so that rule is broken here.
Renumber it to its issue when one is filed, or the convention stops being a convention.
