# RFC 0256 — Drop the `Move` ops that restate their own `Create`

| | |
|---|---|
| **Status** | Draft |
| **Issue** | [#110](https://github.com/outlmd/outl/issues/110) |
| **PR** | — |
| **Date** | 2026-09-11 |
| **Reference doc** | `docs/storage.md` → op-log compaction; `docs/cli.md` → `outl compact` |
| **Invariant** | root `CLAUDE.md` invariant 1 (op log is source of truth), invariant 5 (no silent loss), invariant 11 (attribute the cost) |
| **Guarded by** | `keeps_the_move_that_lifts_a_block_back_out_of_the_trash`, `keeps_a_pair_whose_node_is_created_twice_anywhere_in_the_log`, `keeps_a_page_root_pair`, `keeps_a_pair_a_concurrent_move_of_the_same_node_interleaved_with`, `any_op_at_all_between_the_create_and_the_move_blocks_the_drop` (`crates/outl-core/tests/compaction.rs`); `compaction_preserves_the_materialized_tree`, `the_compacted_log_matches_the_original_under_any_delivery_order`, `every_dropped_op_is_a_move_that_restates_its_own_create` (`crates/outl-core/tests/compaction_property.rs`); `a_damaged_record_refuses_the_whole_pass`, `a_per_page_layout_refuses_rather_than_compacting_half_a_log` (`crates/outl-core/src/storage/compact/tests.rs`) |

## Why

The op log never shrinks.
It replays on every boot and ships whole to every newly paired device, so anything written into it is a cost the user pays forever, on every machine.

Measured on the maintainer's daily-driven workspace (`~/outl-p2p`, 2,574 pages, 20 actors, 36 MB of markdown):

| | |
|---|---|
| `ops/` on disk | **262 MB** (80.8 MB of `.jsonl`, the rest `.idx` sidecars) |
| ops | 217,811 |
| of which `Move` | 65,707 |
| **`Move` ops that change nothing** | **64,563 (98.3%)** |

Almost every `Move` in that log is inert.
The reason is a producer that has already been fixed:
`outl-md`'s `diff.rs` emits `Create` **and** `Move` defensively for a new block, and until recently both reached disk.
`reconcile.rs` now filters the redundant half with `batch.op_is_noop(&op)` against a batch tree updated op-by-op.
The fix is visible in the data — redundancy by HLC month: 2026-06 = 23.4%, 2026-07 = 30.4%, 2026-08 = 5.7%, **2026-09 = 0.0%**.

So this is not a bug still being produced.
It is dead weight already on disk, and nothing in the codebase has ever removed a byte from `ops/`.

## What we chose

`outl compact`, backed by `outl_core::storage::compact` — the **single owner** of the question "is this op inert?".
`plan_compaction` decides and writes nothing; `apply_compaction` is the only code in this repo that rewrites `ops/*.jsonl`.

### The predicate

Let the **merged log** be every op in every `ops/ops-<actor>.jsonl`, sorted by `Hlc` — which orders on `(physical_ms, logical, actor)`, so the actor tiebreak is part of the type and cannot be skipped.
Replaying that sequence through `Tree::do_op` is the canonical materialization.

A `Move{node: n, new_parent: P, position: X}` at `t2`, written by actor `A`, is dropped iff **all six** hold:

1. **Pair.** The record immediately before it *in the merged HLC order* is `Create{node: n, parent: P, position: X}` at `t1`, by the same actor `A`, with `P` and `X` equal field for field.
2. **First appearance.** `t1` is the minimum HLC among every op naming `n` anywhere in the merged log.
3. **Sole `Create`.** No other `Create` anywhere in the merged log names `n`.
4. **Not a page root.** `P != NodeId::root()`.
5. **Replay-verified inert.** In the replay, the tree immediately before `t2` has `nodes[n] == (P, X)`.
6. **Settling horizon.** `t2` is at least 30 days older than `min(newest op in the log, now)`.

The `min` in condition 6 is not decoration.
`physical_ms` is not a number this device chose: `hlc.rs` is hand-rolled with no drift bound, and `HlcGenerator::observe` adopts a peer's higher `physical_ms` unconditionally.
One device with a wrong clock puts an op years in the future into the merged log, and a cutoff anchored on "the newest op" alone then sits years in the past.
Every real op falls below it, condition 6 stops rejecting anything, and a user who deliberately did **not** pass `--no-horizon` gets `--no-horizon` behaviour with nothing said.
That is the worst combination available for the one condition whose whole job is to protect against a residual "the user is not told about, because nothing can detect it".
Clamping to the wall clock is strictly more conservative and changes nothing on a healthy log, where the newest op is already in the past.

### Why it is sound against the log we hold

Conditions 1 and 5 are a decision procedure, not a heuristic.

Applying ops in HLC order makes the final tree a pure function of the sequence.
An op whose application leaves the state unchanged can be deleted from that sequence and every later op still observes the identical state; induction gives an identical final tree.
`Op::Move` is the only variant that can change `nodes[n]` once `n` exists, and `old_parent` / `old_position` are read by nothing but `undo_op`, which never sees an op that is not in the log.

**The trap this predicate exists to avoid.**
`Op::Create` is idempotent: re-applying it for an already-existing node is a no-op.
So for the pair `Create{n, P, X}` → `Move{n, P, X}`:

- if that `Create` is the one that *created* `n`, it placed it at `(P, X)` and the `Move` changes nothing;
- if `n` **already existed elsewhere**, the `Create` did nothing and **the `Move` is the op doing the work**.

The adjacent pair alone cannot distinguish them.
The sharpest instance is deletion:
`Delete` is `Move(n, TRASH_ROOT)`, so a block that was trashed and later restored produces exactly this shape, with the `Create` inert and the `Move` carrying the block back out of the trash.
Dropping it deletes the user's block and leaves no trace.
Conditions 2 and 5 reject it — pinned by `keeps_the_move_that_lifts_a_block_back_out_of_the_trash`.

### Why it is sound against ops we do **not** hold

This is the part that is an argument rather than a proof, and it is stated here rather than buried.

Only an op naming `n` with HLC below `t2` can invalidate the drop.

- **Below `t1`.** A `Move` there is inert (the node does not exist yet in the replay), so only a second `Create` matters.
  For an unguessable 128-bit `NodeId`, naming `n` requires having received ours — so there can be no earlier op naming it.
  Conditions 3 and 4 exclude the id families that *are* guessable:
  `NodeId::from_slug` page and journal roots, `result_node_id` template result blocks, and anything else already contested in this log.
- **Inside `(t1, t2)`.** A peer would have to have received our `Create`, emitted at `t1`.
  Then emitted its own `Move` on `n`, stamped inside a gap in which this device wrote **nothing at all** (condition 1).
  And still not delivered it to us at compaction time — or condition 1 would see it.
  Condition 6 additionally requires that op to have been in flight for longer than 30 days.

**The residual, named.**
Take a device offline for longer than the horizon, holding an undelivered `Move` on a node whose `Create` it received, stamped inside a gap in which the originating device emitted no other op.
It would materialize a different *position or parent* for that one block than the uncompacted log would.
It is not data loss — the block and its text survive on both sides — but it is a divergence, and the compacted side is not the one the original log would have produced.
Nothing in the CRDT detects it.

That is the whole cost of this feature, and it is the reason the predicate is six conditions instead of two.

### Measured result

On `~/outl-p2p` (copied to scratch; the real workspace was never written):

| | |
|---|---|
| ops dropped | **62,209** of 217,811 (28.6%) |
| of the 65,707 `Move` ops | 94.7% |
| bytes reclaimed from `.jsonl` | **17.4 MB of 77.0 MB (22.6%)** |
| `ops/` total, including rebuilt indexes | 262 MB → **195 MB** |
| materialized tree / properties / collapsed set | **byte-identical** |
| compacted log ⊆ original log | yes, verbatim lines |

The headline "99.9% of `Move` ops are redundant" does not survive contact with the predicate, and the honest number is **94.7%**.
The 3,498 `Move` ops it declines are not noise: 2,074 are pairs whose `Create` was *not* the node's first appearance — the trashed-and-restored shape, which is exactly the one that would have deleted content.

### Why it only rewrites this device's file

The draft above reasoned entirely about *which ops* may be dropped and never asked *whose file* they live in.
That gap is load-bearing, because the answer to the second question is what makes `transport = "file"` safe at all.

`docs/storage.md` → Why "one file per actor" states the premise: *"Each device's file is append-only and owned by exactly one writer."*
iCloud Drive, Syncthing, Dropbox and any shared filesystem reconcile **per path**, last-write-wins, and per-actor files are what turn that into a non-issue.
`apply_compaction` breaks the premise directly: it iterates every actor in the plan and rewrites each one's `ops-<actor>.jsonl`, peers included.
On a file transport, device A shortening `ops-B.jsonl` publishes a competing, shorter version of that path; B's longer copy loses the merge, and every op B had not yet shipped is gone with no error anywhere.
The locks do not help — `flock(2)` is advisory and machine-local, which `docs/clients.md` says in as many words.

The condition to refuse on could have been the transport (`[sync] transport = "file"`), and that is the wrong choice.
A workspace configured for iroh and living in a Dropbox folder is exactly the trap, so the config does not answer the question it appears to answer.
The narrow, provable condition is *is this file mine?*, and the device store already owns that answer (`outl_ws::actor::resolve_device_actor`, the same resolution every other command uses).

So `--apply` narrows the plan to this device's actor (`CompactPlan::restricted_to`) and `apply_compaction_as` refuses anything else with `CompactError::ForeignActorFile`.
The dry run is unchanged — the predicate is decided against the merged log either way, and reporting the whole log's dead weight is information, not a write.

The escape hatch is `--force`, documented as naming the risk rather than hiding it, because a guard with no way past it is a wall ([RFC 0211](0211-state-that-leaves-a-boundary.md)'s lesson, invariant 9).
Two users need it.
Someone on an iroh-only workspace who wants the peers' mirrors compacted here as well; and anyone whose `ops/` holds an `ops-<ephemeral>.jsonl` this device wrote in an earlier session.
The second are ours in fact, but an ephemeral actor leaves no binding behind, so they are not ours *provably* — and "provably" is the bar for rewriting an op log.

**What this costs.**
Most of the measured 22.6% lives in other actors' files, so a default run on a 20-actor workspace reclaims much less than the headline.
That is the correct trade: running the command on each device reclaims the same bytes, and RFC-point 3 below already said per-device is the intended usage for an unrelated reason (a peer holding the full history re-delivers the dropped ops anyway).

### What it refuses to touch

- **A damaged log.** A record that does not parse aborts the whole pass (`CompactError::DamagedLog`).
  Rewriting a file we could not fully read would make the loss permanent — invariant 5.
- **The `PerPage` layout.** Compacting only the `Global` half of a mixed workspace would decide inertness against an incomplete log.
- **Anything while the workspace is open.** An exclusive flock on `<root>/.outl/.lock`, which every live `outl` process holds *shared*.
  Plus an `ActorWriteLock` on **every** actor in `ops/`, not just the rewritten ones — a writer holds exactly one, and it need not be one this plan touches.
- **A log that moved since planning.** Each file's byte length is re-checked under the locks, and the *actor set* is re-listed there too — a length check sees a file that changed, never one that was created.
- **Another device's `ops-<actor>.jsonl`**, unless `--force`. See "Why it only rewrites this device's file" above.
- **Reading a live log at all.** `plan_compaction` takes the same exclusive lock before it reads, because an append caught mid-`write(2)` parses as a torn record and would be reported as `DamagedLog` on a healthy log.

Every rewritten file is copied to `.outl/compact-backup/<timestamp>/` and fsynced **before** a byte of `ops/` changes; the rewrite itself is temp + `rename`; kept lines are copied byte for byte, never re-serialized.
The `.ops-<actor>.idx` / `.nodes.idx` sidecars are **deleted**, because every offset in them points into the file just renumbered.

### Undo

Dropping historical inert ops cannot affect the resident undo stack.
That stack works off the in-memory log of the *running* session: a client pushes an entry when it applies an op it just authored, and undo emits a **new, compensating op** rather than editing history.
Compaction refuses to run while any process holds the workspace, so no session's stack can name a dropped op — and the ops it drops are, by the predicate, ones whose application changes nothing.

## Why not the alternatives

**Drop every inert `Move` (all 64,563).**
The extra 2,354 ops are 3.6% more savings for the only failure mode that deletes content.
2,074 of them are pairs whose `Create` was not the node's first appearance — trashed-and-restored blocks, where the `Move` *is* the restore.
Rejected.

**Decide from the adjacent pair alone, without a replay.**
Cheaper and wrong for exactly the reason above:
`Create` idempotence makes the pair ambiguous.
The replay is O(log) once, on a command the user runs occasionally.

**Rebuild the `.idx` sidecars instead of deleting them.**
A rebuilt index is another chance to write a wrong offset, and a wrong offset is a silently dropped op on every index-driven cold read (#129).
A *missing* index always rebuilds from the `.jsonl`.
Deleting has no failure mode that is not "one slower boot".

**Snapshot-and-truncate (replace old history with a materialized snapshot).**
This is the compaction most log systems do, and it is a much bigger win.
It is also not available here.
An op log whose prefix has been replaced by a snapshot can no longer reorder a late op from a lagging peer below the snapshot cutoff — the case invariant 7 and the per-actor cutoff vector (#156 Half 2) exist to serve.
A separate design, not a bigger version of this one.

**Fix it in `outl-md` and wait.**
Already done, and it fixes nothing that is already written.
The 2026-09 redundancy is 0.0% and the 17.4 MB from June and July is still on every device.
Invariant 11 in reverse: the producer fix and this are answers to different problems.

**Put it in `outl doctor --repair`.**
`--repair` is documented as never touching `ops/`, and that promise is load-bearing — it is what makes the command safe to run on a damaged workspace.
A separate command with its own dry run keeps it.

## The opposite direction

**What this makes worse.**

1. **It introduces a way for `ops/` to lose bytes.** Before this, `ops/` was append-only in the strongest sense: nothing in the codebase ever removed a line.
   That property was itself a safety net — any bug anywhere could be reasoned about knowing the log was complete.
   It is now conditional on this predicate being right.
   The mitigations (backup before write, dry run by default, refusal on a damaged log, refusal while open) reduce the blast radius; they do not restore the property.

2. **The mirrored case: an op arriving *after* compaction.** Compaction reasons about the log as it stands.
   A peer op delivered afterwards is applied normally — the log is still a CRDT log and still reorders.
   The one class it cannot serve is the residual named above: an op stamped inside a compacted gap, naming the compacted node.
   It is applied, it takes effect, and the resulting tree differs from what the uncompacted log would have produced.
   The user is **not** told, because nothing can detect it.
   This is why condition 6 exists and why the horizon is not configurable below 30 days except by an explicit `--no-horizon`.

3. **Re-pairing a device does not undo it.** A device that compacted and then re-pairs ships the compacted log.
   A peer that still has the original will re-deliver the dropped ops — they are ops like any other, and the receiving side dedups by HLC — so the log can *grow back*.
   Compaction is not a one-way ratchet across a fleet, and running it on one device while others hold the full history buys less than the measurement suggests.
   Running it on every device is the intended usage.

4. **A restore from `.outl/compact-backup/` is a whole-file restore.** There is no per-op undo.
   If the backup generation is gone (it has no TTL pruning yet, unlike `repair-backup`), there is nothing to restore from but a peer.

**What does not get worse.** The read path is unchanged:
`JsonlStorage` re-reads the file and rebuilds its indexes, and a missing sidecar was already a supported state.
The snapshot cache under `.outl/snapshots/` stays valid.
Compaction removes only ops whose application changes nothing, so the materialized state a snapshot projects is unchanged.
Its per-actor cutoff is compared with `>`, so it never needs the op at the cutoff to still exist.

## How it cannot regress

1. **The invariant.**
   Root `CLAUDE.md` invariant 1 already says the op log is the source of truth and the `.md` is a projection; this RFC is the record of the one exception where a line may be removed, and of what has to be proven first.
   Invariant 5 ("no silent loss") is what forces `CompactError::DamagedLog` to be a refusal rather than a skip.

2. **The tests.**
   These exist to fail if someone re-simplifies the predicate down to "an adjacent `Create` with the same parent and position" — the shape that reads as obviously equivalent and deletes blocks.
   Do not delete or relax them:

   - `keeps_the_move_that_lifts_a_block_back_out_of_the_trash` — the content-deleting case, in full: trash, re-`Create`, restoring `Move`.
   - `keeps_a_pair_whose_node_is_created_twice_anywhere_in_the_log` — condition 3; a derivable id.
   - `keeps_a_page_root_pair` — condition 4; a `from_slug` id another device can mint with no causal contact.
   - `keeps_a_pair_a_concurrent_move_of_the_same_node_interleaved_with` — the divergence made visible.
   - `any_op_at_all_between_the_create_and_the_move_blocks_the_drop` — condition 1 is *empty gap*, not "nothing that could matter".
     Measured: both rules drop the same 62,209 ops, so the cheaper-to-justify one wins.
   - `compaction_preserves_the_materialized_tree` and `the_compacted_log_matches_the_original_under_any_delivery_order` — the correctness bar itself.
     The generator produces trash moves, restores, duplicate `Create`s and multi-actor interleavings.
   - `every_dropped_op_is_a_move_that_restates_its_own_create` — asserted independently of the tree comparison, so a bug that cancels itself out across two ops still fails.
   - `a_damaged_record_refuses_the_whole_pass` — invariant 5 at the compaction boundary.
   - `a_rewrite_refuses_to_touch_another_devices_ops_file` and `a_plan_narrowed_to_this_device_rewrites_only_its_own_file` — the file-ownership rule in both directions.
     The refusal fires, and the narrowed plan still leaves the peer's bytes identical.
   - `a_future_stamped_op_cannot_disarm_the_settling_horizon` — condition 6 cannot be turned off by a peer's clock; `the_horizon_is_unchanged_when_every_op_is_in_the_past` is the mirror that keeps the clamp from becoming a second policy.
   - `an_actor_file_that_appeared_since_the_plan_is_refused` and `an_actor_file_that_vanished_since_the_plan_is_refused_as_stale` — the plan's actor set, re-checked under the lock.
   - `a_rewrite_that_fails_still_names_the_backup_it_took` — the recovery path is printed in the one case that needs it.
   - `planning_refuses_while_the_workspace_is_open` and `planning_succeeds_once_nothing_holds_the_workspace` — the read-side lock, and that it is not a wall.

## Scope

Not covered here:

- **The `.idx` sidecars' own size.** They are 181 MB of the 262 MB in that workspace — more than twice the log they index.
  Compaction shrinks them proportionally as a side effect of shrinking the log, which is not the same as addressing them.
  Separate work.
- **Snapshot-and-truncate compaction**, which is the design that would make boot proportional to the live tree rather than to history.
  Rejected above as a different RFC, not a bigger version of this one; see also `docs/sync.md` → Per-page op log shards.
- **The `PerPage` op-log layout** (RFC 0137 Phase B).
  `plan_compaction` refuses it rather than compacting half a log.
- **Pruning `.outl/compact-backup/`.** `repair-backup` has two guards (age *and* generation count); this has none yet.
- **A defect this work surfaced and does not fix:** a node with two `Create` ops does **not** converge under reordering.
  `undo_op(Op::Create)` removes the node outright, so a `Move` replayed after that undo finds nothing to move and the redone `Create` reinserts the node at *its* position.
  Three ops reproduce it — `Create(n,P,"a")@1`, `Move(n,P,"t")@2`, `Create(n,P,"u")@3` — materializing `"t"` in HLC order and `"u"` delivered `[3,1,2]`.
  It predates this change and compaction never touches such a node (condition 3), but it is a real hole in invariant 1 (convergence) and deserves its own issue.
