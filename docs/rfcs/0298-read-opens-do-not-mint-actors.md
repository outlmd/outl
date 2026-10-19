# RFC 0298 — A read does not mint a write actor

| | |
|---|---|
| **Status** | Accepted |
| **Issue** | [#298](https://github.com/outlmd/outl/issues/298) |
| **PR** | [#394](https://github.com/outlmd/outl/pull/394) |
| **Date** | 2026-10-19 |
| **Reference doc** | [doctor.md](../doctor.md) (the report), [storage.md](../storage.md) (why a lock is not deleted), [`crates/outl-core/CLAUDE.md`](../../crates/outl-core/CLAUDE.md) (invariant 9's four questions for the write actor) |
| **Invariant** | root `CLAUDE.md` invariants 9 and 11 |
| **Guarded by** | `a_read_only_open_does_not_mint_an_actor_while_the_device_lock_is_held`, `a_read_only_open_cannot_append_to_the_device_actors_log` (`crates/outl-ws/tests/read_only_open.rs`); `actor_logs_are_counted_when_this_device_holds_no_write_lock`, `a_pile_of_local_write_locks_is_a_warning`, `fewer_write_locks_than_the_threshold_stay_a_note`, `actor_write_locks_are_counted_and_never_collected` (`crates/outl-cli/src/cmd/doctor/tests/index_sidecars.rs`) |

## Why

When the device actor's write lock is held, `resolve_write_actor` mints a new `ActorId` and locks that.
That is the right answer for a process that is about to append: two processes must not write one `ops-<actor>.jsonl`.
It is the wrong answer for a process that only reads.

`outl workspace info` is the documented poll.
Beside a running `outl serve` it lost the lock on every tick and left a permanent `ops-<ulid>.jsonl` behind.
A machine with about nine automated writers accumulated actors forever.
Each actor is an entry in the sync handshake (`HashMap<ActorId, ActorClock>`) and in every snapshot cutoff (`BTreeMap<ActorId, Hlc>`).
Compaction drops inert ops.
It does not drop identities, so the handshake cost stays after the bytes are gone.

The same workspace had 20 actors, 16 of them under 500 ops.
Invariant 11 had already recorded 16 files of 1 to 221 ops and said the cost deserved its own issue.
This is that issue.
Invariant 9's fourth question — what cleans this up — had no answer.

`outl doctor` printed an info line about lock files, and only when at least one lock existed.
A peer's log, which has no `.lock-<actor>` on this device, was invisible.
The measured pile is locks ≈ logs, because the lock files are never deleted, so "more locks than logs" does not fire on it either.

## What we chose

A read does not mint.

`outl_ws::open_read` is the owner.
It takes the shared workspace lock, resolves the device actor, and does not call `resolve_write_actor`.
`OpenOptions::writes` defaults to `true`, hand-written so a derived `Default` cannot turn `open()` into a read.
After per-page shards are registered, the open calls `Workspace::disallow_local_writes`, which reaches every shard.
`JsonlStorage::append_ops_inner` then refuses a non-empty batch before it opens the `.jsonl`.
`WsCtx::commit` refuses earlier, before `commit_page` mutates.
Slug repair is skipped on a read, because it can emit ops.
The next writing open still runs it.

`open()` is unchanged.
A writer that loses the device lock still mints, and the HLC for the actor it keeps is still seeded from that actor's log by `Workspace::seed_clock`.

The CLI's read subcommands (`workspace info`, `search`, `query`, `page get` / `list` / `render`, `block get` / `tree`, `backlinks`, `history`, `export`, `tag`, `trash list`, `template list` / `resolve`, `prop get` / `list`, `daily range`) open through `open_read`.
The GUI still refuses a held lock instead of forking, for the reason in [clients.md](../clients.md): its `HlcGenerator` is fixed at startup.

`outl doctor` reports the `ops-*.jsonl` count even when this device holds no write lock, and warns when the local `.lock-<actor>` count reaches `EPHEMERAL_ACTOR_WARN_AT`.
The constant owns the number.
Below it the line stays info.
Nothing offers a lock or an op log for deletion.

## Why not the alternatives

**A bounded pool of secondary actor ids, persisted per device and reused across runs.**
That is a second store of actor identity, outside the workspace and unsynced, which is a new home for state invariant 9 asks four questions about.
The issue's definition of done does not ask for it.
Reuse would also have to resume each id's HLC from its existing log, which is real machinery on top of a design the issue did not choose.
The writers that still mint are the follow-up, not this change.

**Wait briefly for the device lock instead of minting.**
The issue names this as the writer-side step with real upside, and it is not in the definition of done.
The holder in the measured pile is a long-lived process — `outl serve`, a GUI — that does not release the lock for the session.
A short wait still loses, and the command then either mints or fails.
It does not bound that pile.

**Merge a small actor back into the device actor.**
The issue says this is not safe, and it is right.
Rewriting the actor field changes the HLC tiebreak, so two ops that tied become ordered and convergence changes.
Re-read [RFC 0211](0211-state-that-leaves-a-boundary.md) before touching it.

**Retire an actor from the handshake once every peer's snapshot cutoff covers it.**
That is the issue's third step, and it is about identities already on disk.
Compaction ([#110](https://github.com/outlmd/outl/issues/110)) and per-page shards are about op volume, not identity count.
This change does not touch the handshake or the cutoff.

## The opposite direction

Writers that lose the lock still mint.
A machine whose overlapping writers are all long-lived keeps growing actors.
This change stops the reads, which were the unbounded automated case, and it does not bound a session that is all writers.

A read skips slug repair, so a split-brain slug stays split until the next writing open.
The read shows both roots.
That is the cost of a read that must not emit ops.

`Workspace::apply` on a read-only workspace still mutates the in-memory tree and then fails at storage.
That is the existing storage-failure shape, documented on `apply`.
`commit` refuses before the mutation, which is the path the CLI uses.
A caller that goes through `apply` directly has a dirty memory tree and an unchanged log.

Doctor warns and does not delete.
A workspace that already has 20 actors keeps 20 actors.
The warning names the count.
Nothing rewrites an id, so a peer that already exchanged those ops still converges on the same tiebreaks.

Opening storage can still write an index sidecar.
That is a boot cache, not an actor identity, and it is out of scope here.
The actor-name assertions in the tests ignore those files on purpose, so a sidecar write cannot hide a minted actor, and it also cannot be mistaken for one.

## How it cannot regress

1. **The invariant.**
   `crates/outl-core/CLAUDE.md` → "The write actor, and invariant 9's four questions" answers who writes, who reads, how a test gets its own copy, and what cleans it up.
   The cleanup answer is "nothing", with the reason that stays acceptable and the bound this change actually adds.
2. **The tests.**
   `a_read_only_open_does_not_mint_an_actor_while_the_device_lock_is_held` fails if a read calls `resolve_write_actor` again: the directory's lock and `ops-*.jsonl` names grow.
   `a_read_only_open_cannot_append_to_the_device_actors_log` fails if the refused append creates a file, or if `commit` stops refusing.
   `actor_logs_are_counted_when_this_device_holds_no_write_lock` fails if the report goes back to "locks only, and nothing when there are none".
   `a_pile_of_local_write_locks_is_a_warning` fails if the pile is info again.
   `fewer_write_locks_than_the_threshold_stay_a_note` fails if the warning moves down onto an ordinary machine.
   `actor_write_locks_are_counted_and_never_collected` fails if a lock becomes repairable.

## Scope

Existing `ops-*.jsonl` and `.lock-<actor>` files stay.
Nothing in this change rewrites an actor id or compacts a log.
A workspace that already piled actors up keeps them; `outl doctor` is how they become visible, and new reads stop adding.

Not in this change:

- Bounding the writers that still mint when the device lock is held.
- Retiring an actor from the handshake once every peer's snapshot covers it.
- Compaction of identities ([#110](https://github.com/outlmd/outl/issues/110)).
- `outl sync` and `outl plugin`, which still open for writing.
- The MCP session, which serves writes and still uses `open`.
- Index sidecar writes during a read open.
