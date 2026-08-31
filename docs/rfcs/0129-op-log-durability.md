# RFC 0129 — An acknowledged op must survive the crash, the reader, and the rebuild

| | |
|---|---|
| **Status** | Shipped |
| **Issue** | [#129](https://github.com/outlmd/outl/issues/129), [#157](https://github.com/outlmd/outl/issues/157), [#192](https://github.com/outlmd/outl/issues/192) |
| **PR** | — |
| **Date** | 2026-08-06 |
| **Reference doc** | [storage.md § Failure modes](../storage.md#failure-modes) |
| **Invariant** | root `CLAUDE.md` invariant 1; `outl-core/CLAUDE.md` invariant 5 ("No silent loss") |
| **Guarded by** — never drop a durable op | `torn_tail_is_healed_not_glued_on_next_append`, `non_utf8_line_does_not_truncate_the_rest_of_the_file` (`crates/outl-core/tests/op_log_io_robustness.rs`) |
| **Guarded by** — batch fsync | `append_ops_batch_equals_sequential_appends`, `append_ops_heals_torn_tail_once` (`crates/outl-core/src/storage/jsonl/tests.rs`) |
| **Guarded by** — replay fidelity | `reedit_after_snapshot_matches_full_replay`, `three_edits_across_sessions` (`crates/outl-core/tests/text_replace_rebuild.rs`) |
| **Guarded by** — ghost block (#122) | `a_redundant_op_is_recognised_as_a_noop`, `a_move_that_changes_the_tree_is_never_filtered_out`, `edit_is_left_to_the_yrs_delta_to_decide` (`crates/outl-core/src/workspace.rs`); client half: `"does not commit when the text is unchanged"`, `"still leaves edit mode when the text is unchanged"`, `"commits when the user actually changed the text"` (`crates/outl-desktop/src/components/BlockRow.test.tsx`) |

The full per-decision list is in [How it cannot regress](#how-it-cannot-regress); the rows above name the two that fail first.

## Why

The op log is the source of truth.
Three separate defects meant that sentence was not true in production, and each one broke it at a different point in the op's life.

**A crash could destroy an edit that was already fsynced and acknowledged (#157).**
If the process died mid-write (power loss, iOS jetsam), the last line of a `.jsonl` was left incomplete, and the *next* append wrote straight after it with no separating newline.
That glued a partial fragment onto a good, fully-durable edit.
On the next load the reader tripped over the broken fragment and discarded the **entire** line — including the healthy edit.
The same reader had a second gap: one unreadable byte partway through a file stopped the read of that file, so every edit *after* the damage — potentially thousands — silently vanished from what the user saw, behind a single `warn`.
Both scenarios are routine, not exotic: crashes on mobile, partially-transferred files on iCloud and Syncthing.

**Durability was priced so high that callers would want to avoid it (#192).**
`append_op` did `open + write + sync_all()` per individual op.
On macOS `sync_all()` issues `F_FULLFSYNC`, measured at 4.23 ms on an APFS SSD against 0.03 ms for a plain `fsync`.
A page upsert (trash the old blocks, append the new forest, set three props) emits 15–20 ops, so one logical write cost ~20 barriers:

| operation (embedder benchmark, release, tempdir) | before |
|---|---|
| write 1 new page (~7 blocks) | 86.9 ms |
| update 1 existing page (full replace) | 108.3 ms |
| cold open over a 3 MB op log | 14.6 ms |

The math closes exactly: 20 × 4.2 ms ≈ 87 ms.
Everything else — reads, boot, scans — was already fast.
That number is the whole reason this is in an RFC rather than a changelog line: a durability barrier a caller has an incentive to route around stops being a durability barrier.

**Replaying a healthy log produced text the user never typed (#129).**
Two sequential edits by the same actor on the same block, then reopen the workspace, and the block read back as the **concatenation** of both writes instead of the second one.
`"hello"` then `"hello world"` reopened as `"hellohello world"`.
Not an edge case: every user who edits a block twice and then restarts the TUI, runs a CLI command, or launches the mobile app.
A peer that received both `Edit` ops and rebuilt from scratch saw the same wrong string.

The three share one question — *what does the log owe the user?*
An op that is durable is worth nothing if the reader drops it, if the write path is too slow to be used as intended, or if replay reconstructs something else.

## What we chose

### 1. Never drop a durable op: self-heal on write, resync on read

**Write side.**
`JsonlStorage::append_ops_inner` (`crates/outl-core/src/storage/jsonl/append.rs`) checks whether the file already ends in a newline and adds one if it does not, before writing anything.
A torn tail can therefore never be glued onto the next edit.
It is one byte read on the writer that owns the file.

**Read side — the rule.**
**Every sequential pass over a `.jsonl` skips an unreadable record and continues.**
The hard stop became warn-and-continue in all three passes: `read_ops_file_into` (the full replay) and `rebuild_actor_indexes` / `index_stream` (the index build), all in `crates/outl-core/src/storage/jsonl/read.rs`.
`MAX_CONSECUTIVE_IO_ERRORS = 64` bounds the other direction, so a genuinely dead file or device is concluded gone rather than retried forever.

**The index build is the load-bearing case, and it is the one that hides best.**
A short index never *knows* about the ops past the damage, so no downstream check can fire for them and the tree boots short with no error anywhere.
Two consequences follow.
The pass keeps indexing past a bad record, and a rebuild that hit **any** read error refuses to persist its `.idx` / `.nodes.idx` sidecars.
Caching a known-incomplete index turns a recoverable omission into a permanent one.

**Where the omission *is* indexed, it is an error, not a short result.**
All four index-driven reads — `ops_since`, `ops_for_actor`, `ops_since_per_actor`, `ops_for_node` — return `StorageError::MissingOp` when the index lists an op the file will not return.
Snapshot boot degrades to a full sequential replay on that error, which re-reads the file and recovers everything around the damage.
Returning a shorter list would be permanent: the next snapshot's cutoff is derived from the *index*, so the omission gets written down as already-folded-in and no later boot replays it again.

The single rule behind every row of the [failure-mode table](../storage.md#failure-modes): **a read that returns fewer ops than the log holds must be impossible to confuse with a healthy read.**
Which of the two responses applies is decided by one question — does the offset index know about the op?
If it does, dropping it is invisible *and* gets baked into the next cutoff, so it is an error.
If the index does not know about it either, the omission is self-correcting on a later boot, so it is a loud log line and not a failed open.

Two neighbours landed in the same story because they are the same defect shape.
A single sync-conflict copy (`ops-<id> 2.jsonl`, `ops-<id>.sync-conflict-….jsonl`) used to abort the whole open (#154).
That locked a user out of their notes over a stray file sync tools leave behind in completely normal two-device usage; it is now a warning and a skip.
And a line carrying two concatenated JSON objects with no separating newline is recovered into all its ops instead of being dropped, which is what an interleaved non-atomic append from an external writer produces.

### 2. `Storage::append_ops(&[LogOp])` — one fsync for the batch

A trait method with a **default implementation that loops `append_op`**, so `MemoryStorage` and any future backend need no change and cannot silently lose the guarantee; `JsonlStorage` overrides it.
`append_op` is `append_ops` of one, so the torn-tail heal and index mirroring exist once.

The order inside `append_ops_inner` is deliberate: validate every op (foreign-actor guard), serialize every line, **then** open, heal, write, and fsync once.
A rejected or unserializable batch leaves the file untouched, and an empty batch opens nothing.

`Workspace::begin_batch` / `enter_batch` / `end_batch` (`crates/outl-core/src/workspace/batch.rs`) is the caller-side owner.
Composite actions in `outl-actions` (`append_forest` and friends) route through it, so one user-visible action costs one barrier.

The durability semantics, stated precisely: on `Ok` the whole batch is durable; a crash mid-batch leaves a **durable prefix with a possibly-torn last line**.
That is survivable only because decision 1 shipped alongside it.

Same benchmark, after:

| operation | before | lib batching only | + one `enter_batch`/`end_batch` around the whole upsert |
|---|---|---|---|
| write 1 new page | 86.9 ms | 27.8 ms | **10.8 ms** |
| update 1 page | 108.3 ms | 52.9 ms | **10.4 ms** |

The remaining ~10 ms is the caller's own markdown projection and index write, not the op log.

### 3. Every `Op::Edit` must be replayable, in HLC order, into a fresh empty `Doc`

`ContentStore::build_doc` (`crates/outl-core/src/content.rs`) starts from `Doc::new()` and applies each of the node's `Edit` updates in order.
So an update emitted by `replace_text` must have been produced against a `Doc` that **already carries that node's complete prior `Edit` history**.
Against an incomplete history the captured `sv_before` is wrong, so the `remove_range` half of the encoded update references clocks the rebuilt doc never had.
Yrs records that delete as a no-op because its targets do not exist, and the insert survives on top of the old text.
That is the concatenation, and it is why the CRDT semantics were never violated while the materialized string was still wrong.

The single owner of the precondition is `Workspace::ensure_doc_for_edit` (`crates/outl-core/src/workspace.rs:379`).
It hydrates the node's `Doc` from `ops_for_node_combined` before `ContentStore::replace_text` captures the state vector, and returns early when `log_complete` already guarantees the in-memory log holds everything.
The `ops.sort_by_key(|l| l.ts)` in it is load-bearing, not tidiness: `ops_for_node`'s return order is backend-dependent, and the state vector has to be the one the full-replay path will reproduce.

This is what makes `ops_for_node` the sharpest of the four index-driven reads in decision 1.
A short read there does not shorten a list somebody inspects — it hands the user block text they never wrote.

### The supporting policy: do not write an op for something the user did not do

Every op is permanent, because the log is append-only by design.
So the cheapest durable op is the one never written, and two changes enforce that from opposite ends.

On the desktop, an empty page used to show a *"Click to add the first block"* button whose click appended a real `Op::Create`, so every day merely opened-and-clicked left an empty block and a permanent op behind (#122).
It is now a **ghost first block**: a frontend-only draft that looks and edits like a real row, with the caret already parked.
Nothing reaches the log until the user commits non-empty text, and an unmount with a pending draft fires a materialize so typed text is never dropped.

In the core, `Workspace::op_is_noop` (`crates/outl-core/src/workspace.rs:425`) is the same policy for machine-generated ops.
The `.md`-reconcile diff defensively re-emitted `Create` + `Move` + a `SetProp` per property for **every** block on **every** commit, so a one-block edit on an 11-block page persisted 23 ops.
Each was idempotent on the tree and still cost a barrier and a permanent log line.
`Move` whose target differs is never a no-op, so cycle-forming moves are still logged and invariant 4 holds.

## Why not the alternatives

**Truncate the torn tail on read instead of healing it on append (#157).**
Symmetrical-looking, and it makes a reader rewrite the file it is reading — on a path where another device may be mid-sync and where the process might die again.
It also does nothing for a log that is *already* glued, which is every workspace that had crashed once before the fix.

**Keep aborting the file when a line will not parse, so nothing is quietly wrong (#157).**
This was the shipped behaviour, and it is the defect.
Aborting discarded every op after the damage and booted a truncated tree behind a `warn` line nobody reads, which is the loudest possible way to be silent.
"Loud" cannot be allowed to mean "shorter workspace"; skipping the record confines the loss to the damaged bytes.

**Use a plain `fsync` instead of `F_FULLFSYNC` (#192).**
0.03 ms against 4.23 ms would have made per-op durability essentially free.
It also gives up the guarantee: `F_FULLFSYNC` is what forces the APFS drive cache to the platter, and per-actor JSONL exists precisely so that a crash never costs an acknowledged edit.
Amortizing the barrier keeps the guarantee and moves the cost; weakening the barrier keeps the cost model and loses the guarantee.
That is the one trade this project does not get to make.

**Op-log compaction / checkpointing (#192, raised in the same benchmark).**
A full-replace page update costs 7.2 KiB of log forever, so a hot page updated a thousand times leaves 7 MB every boot replays.
Real, and orthogonal — it shrinks the log, not the number of barriers per user action.
Phase 3 of [#128](https://github.com/outlmd/outl/issues/128); it needs an undo horizon and its own UX.

**Make `build_doc` replay incrementally, matching what `merge_update` does at edit time (#129, the issue's own first candidate).**
It works, and it puts the fix in the *rebuild* while leaving the producer free to keep emitting a state-vector-relative update against an incomplete history.
Every future caller of `replace_text` then has to remember the precondition.
Hydrating the producer's `Doc` fixes it at the one place that captures `sv_before`.

**Emit a state-vector-independent update — delete-by-range then insert, expressed so Yrs cannot encode it as delete-by-ID (#129, the issue's second candidate).**
Also works, and it is paid for in convergence.
A delete Yrs cannot attribute to specific clocks stops merging cleanly with a concurrent edit from a peer, and block text is a CRDT on purpose.
Spending mergeability to fix a rebuild that a lookup can fix is the wrong currency.

**On desktop, seed a real empty bullet like the TUI does (#122).**
Three lines, and it gives the cursor a home immediately.
It also appends a permanent `Op::Create` for every day the user merely opened, on the one client whose empty-page affordance is a click rather than a keystroke.

## The opposite direction

**Decision 1 makes a damaged log quieter, and that is a genuine cost.**
A tolerant reader boots a workspace missing exactly the damaged records, behind a `warn` the user never sees.
The mirrored question — *what if the reader is the healthy side and the file is the short one?* — is exactly why an index rebuild that hit a read error refuses to persist its sidecars.
A cached short index makes the omission permanent, because the next snapshot's cutoff is derived from the index and records the missing op as already folded in.
`outl doctor` is the surface that tells the user, and `glued_op_lines_are_reported_as_recovered` and `sync_conflict_copies_are_reported_as_errors` (`crates/outl-cli/src/cmd/doctor/tests/mod.rs`) pin that it keeps saying so.

**Decision 2 widens the crash window from one op to one batch, and the order the two shipped in is not incidental.**
Before, a crash could cost at most the op being written.
Now it can leave a batch's tail torn, so a composite action is explicitly **not** atomic: the prefix is durable and the last line may be half-written.
That is only survivable because decision 1 self-heals the torn tail on the next append and the reader skips the fragment.
**Anyone who reverts decision 1 while keeping `append_ops` converts the fsync amortization into a data-loss surface.**
`append_ops_heals_torn_tail_once` is the test that fails, and its name is the only place that coupling is visible from inside the write path.

The other half of decision 2's mirror is visibility rather than durability.
`append_ops` amortizes the barrier but not the moment a peer can observe the file, so a reader can see a batch prefix.
On the file transports that is indistinguishable from an in-flight transfer, which is already the steady state; on iroh ops ship after the append returns.
No new case, but it means a batch is not a transaction and must never be described as one.

**Decision 3's mirror is the read path, and it is the sharp one.**
#129 fixed *the producer captured a state vector against an incomplete history*.
The mirror is *the consumer replays an incomplete history* — `ops_for_node` returning a short result — which produces the same wrong text from the opposite side and is invisible to any amount of staring at the write path.
It is fixed, by decision 1's `MissingOp`, and pinned from both directions.
Storage side: `ops_for_node_surfaces_missing_ops_instead_of_dropping_them` (`crates/outl-core/src/storage/jsonl/read_robustness.rs`).
Workspace side: `ops_for_node_complete_after_non_edit_op_on_empty_cache_boot` (`crates/outl-core/tests/text_replace_rebuild.rs`).
The second one is worth reading: it is the RFC 0137 Phase A regression where a snapshot boot left the LRU empty, a warm-only answer dropped the node's disk-resident `Edit`, and #129 reopened through a door that had nothing to do with editing twice.

**When an op is refused, is the user told?**
No, and deliberately.
`MissingOp` is not surfaced; snapshot boot falls back to a full sequential replay that recovers everything around the damage, and `Workspace` warns and keeps the block text it already has rather than adopting a shorter rebuild.
The loud version would be a modal on every partially-downloaded iCloud file, and a guard that fires constantly gets disabled.
The genuinely unrecoverable case is treated differently: a whole peer file that will not open is `error!` naming the file, and the actor is then also absent from the offset index, so no snapshot cutoff ever claims to have folded its ops in.

**And what gets worse for the ghost block.**
Nothing reaches the log until the user commits text, which means an in-flight draft lives only in the webview.
A crash before commit loses it, where the old click-to-create behaviour would have had a durable empty block.
That is the correct trade for an empty draft and the wrong one for a typed one, which is why unmount fires a materialize rather than discarding.
It has no automated test — see the gap below.

## How it cannot regress

1. **Invariants.**
   Root `CLAUDE.md` invariant 1 states that the op log is the source of truth and everything else is a projection, which is what forbids "fixing" state by editing a `.md` or an index.
   `outl-core/CLAUDE.md` invariant 5 ("No silent loss") is the enforcing surface, and it was extended to the **read** side in the words a contributor hits while editing `read.rs`.
   A damaged log may cost you the damaged bytes, never the healthy bytes after them, and never quietly.
   It names all three sequential passes explicitly, so a *new* pass can be checked against the list, and it says outright that the index build is the one that hides best.
   `docs/storage.md` → [Failure modes](../storage.md#failure-modes) is the single owner of the per-failure table and of the "does the offset index know about the op?" test that decides between recover-and-continue and fail-loudly.
   `docs/storage.md` → [Concurrency](../storage.md#concurrency) owns the `append_op` / `append_ops` contract and the glued-op recovery rule.

2. **Tests, per decision.**

   *Never drop a durable op:*
   `torn_tail_is_healed_not_glued_on_next_append`, `non_utf8_line_does_not_truncate_the_rest_of_the_file`, `conflict_named_ops_file_does_not_break_open` (`crates/outl-core/tests/op_log_io_robustness.rs`);
   `corrupt_middle_line_does_not_discard_the_rest_of_the_log`, `unparseable_op_line_does_not_discard_the_rest_of_the_log`,
   `op_listed_in_index_but_unreadable_is_an_error_not_a_short_read`, `ops_for_actor_surfaces_missing_ops_instead_of_dropping_them` (`crates/outl-core/tests/op_log_truncation.rs`);
   `index_rebuild_skips_a_read_error_and_keeps_indexing`, `index_rebuild_covers_ops_after_a_corrupt_middle_line`,
   `index_rebuild_gives_up_on_an_endlessly_failing_reader`, `ops_for_node_surfaces_missing_ops_instead_of_dropping_them` (`crates/outl-core/src/storage/jsonl/read_robustness.rs`);
   `recovers_glued_ops_on_one_line` (`crates/outl-core/src/storage/jsonl/tests.rs`).

   *Batch fsync:*
   `append_ops_batch_equals_sequential_appends`, `append_ops_rejects_foreign_actor_without_writing`, `append_ops_empty_batch_is_noop`, `append_ops_heals_torn_tail_once`, `append_ops_indexes_every_op` (`crates/outl-core/src/storage/jsonl/tests.rs`);
   `batch_persists_same_ops_as_sequential`, `batch_calls_append_ops_once_per_destination`, `batch_drop_without_commit_still_persists`,
   `nested_batches_flush_once_at_outermost`, `batch_buffers_cycle_move_and_keeps_it_in_log`, `reload_after_batch_reproduces_tree` (`crates/outl-core/src/workspace/batch.rs`);
   `append_forest_batched_matches_sequential_and_persists` (`crates/outl-actions/src/block/create.rs`).

   *Replay fidelity:*
   `reedit_after_snapshot_matches_full_replay`, `ops_for_node_complete_after_non_edit_op_on_empty_cache_boot`, `three_edits_across_sessions`, `multi_actor_edit_converges_on_replay` (`crates/outl-core/tests/text_replace_rebuild.rs`).

   Three of these read like something else and must not be "cleaned up":
   `index_rebuild_gives_up_on_an_endlessly_failing_reader` looks like a timeout test and is what stops the tolerant reader from spinning forever on a dead device;
   `ops_for_node_complete_after_non_edit_op_on_empty_cache_boot` looks like an LRU cache test and is a #129 regression;
   `append_ops_heals_torn_tail_once` looks like a duplicate of the single-op heal test and is the only place the coupling between decisions 1 and 2 is asserted.

3. **The gap, stated rather than papered over.**
   The desktop ghost first block (#122) has **no automated test** — not for "an untouched draft leaves the log empty", not for "unmount with pending text materializes".
   Both are behaviours a refactor of `OutlineView.tsx` could remove without failing anything.
   `crates/outl-desktop/src/components/BlockRow.test.tsx` is where they belong.

## Scope

**Not covered — boot cost over a healthy log.**
Reparsing 211k ops on every open, the snapshot cutoff, and lazy content are [RFC 0128](0128-boot-and-memory-at-scale.md).
That RFC's lazy read path *depends* on decision 3 here, which is why the two cross-reference rather than merge.

**Not covered — op-log compaction.**
[#110](https://github.com/outlmd/outl/issues/110) and Phase 3 of [#128](https://github.com/outlmd/outl/issues/128).
Nothing here removes a byte from the log.

**Not covered — per-page op-log shards.**
[RFC 0137](0137-storage-scale.md) Phase B.
`append_ops` already routes per destination (`batch_calls_append_ops_once_per_destination`), so the batch API is forward-compatible with it.

**Not covered — the `jsonl.rs` split.**
[#161](https://github.com/outlmd/outl/issues/161) moved the file into `jsonl/{mod,read,append}.rs` with no behaviour change, and it is mentioned only because landing #154 and #157 required relaxing the file-size guard on that file once.
That relaxation was a one-off, and the split is what removed the need for it.
A pure restructure does not earn an RFC of its own.
