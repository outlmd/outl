# RFC 0258 — Snapshots are a cache with no eviction, and the eviction rule is not "the actor is gone"

| | |
|---|---|
| **Status** | Accepted |
| **Issue** | — |
| **PR** | — |
| **Date** | 2026-09-11 |
| **Reference doc** | [`crates/outl-core/CLAUDE.md`](../../crates/outl-core/CLAUDE.md) → "The snapshot cache has a GC, and its rule is about the reader", [`storage.md`](../storage.md) |
| **Invariant** | root `CLAUDE.md` invariants 9, 10, 11 |
| **Guarded by** | `a_snapshot_this_binary_can_never_decode_is_dropped`, `the_devices_own_undecodable_snapshot_is_dropped_on_boot`, `a_candidate_the_boot_selector_would_never_choose_is_dropped`, `a_candidate_with_nothing_to_offer_is_dropped`, `a_scratch_file_a_killed_writer_abandoned_is_debris`, `the_scratch_name_older_builds_left_behind_is_still_debris`, `the_devices_own_snapshot_is_never_dropped_for_being_behind`, `a_snapshot_from_a_newer_build_is_kept`, `a_snapshot_that_could_not_be_read_is_kept`, `a_file_replaced_since_the_survey_is_not_pruned`, `nothing_outside_the_snapshots_directory_is_touched`, `a_scratch_file_that_may_still_be_in_flight_is_kept`, `the_only_usable_candidate_is_never_dropped`, `an_empty_or_absent_directory_is_not_an_error`, `a_file_that_is_not_a_snapshot_is_left_alone`, `the_boot_selector_reclaims_nothing_on_a_read_only_open` (`crates/outl-core/src/snapshot/gc/tests.rs`), `publishing_a_snapshot_collects_the_siblings_it_made_unreachable`, `a_published_snapshot_is_never_torn_by_an_overlapping_worker` (`crates/outl-core/src/workspace/snapshot_policy.rs`), `concurrent_writers_for_one_actor_never_publish_a_torn_snapshot` (`crates/outl-core/src/snapshot.rs`) |

## Why

`<root>/.outl/snapshots/` had no eviction of any kind.
It gained one `snap-<actor>.bin` per actor that ever wrote one on this device and lost none.

The measurement, from the maintainer's daily-driven workspace `~/outl-p2p`:

```
13457754  snap-01KTA7C1K39RJTVK9BR3AY897M.bin   ok, 67689 nodes
16060305  snap-01KVXXX0R05044XGRR3KS5J7ET.bin   DECODE ERROR
13527641  snap-01KZBM0TFWN8V5B2GX3GGACM9D.bin   ok, 67939 nodes
13457754  snap-01KXT6HPBYHNMTTMYEDJVTES21.bin   ok, 67689 nodes
```

54 MB in four files.
The first and last are byte-identical in size and node count — the same materialized tree, stored twice.
The second is a schema-3 **bincode** body left behind when the encoder moved to postcard at schema 4 (#207); a foreign format dies in the parser before the schema check runs, so `outl doctor` reports it as `Decode`, not `SchemaMismatch`.

Two things about that directory were worth correcting in the framing, because they change what the fix has to do.

**It is not a multi-device directory.**
`.outl/` is a dotfile tree and never rides the file-sync surface; iroh ships snapshots only by an explicit pull.
So all four files were written by processes on **this one machine**, under four different write actors over time.
`ops/` in the same workspace holds 20 `ops-<actor>.jsonl` files, which is the same story with more rotations.
The growth driver is actor rotation on one device, not peers.

**The undecodable file is not currently costing this device a replay.**
The boot selector reads `snap-<own actor>.bin` first, and this device's own actor (`01KZBM0TFWN8V5B2GX3GGACM9D`) has a snapshot that decodes.
The dead file costs 15 MB and nothing else *right now*.
What it does cost is paid on the next actor rotation.
With no own snapshot, `read_best_from_disk` reads and decodes **all four files** — 54 MB of I/O and three full postcard decodes of ~13 MB bodies — on every boot until this device writes its own.
It silently skips the dead one each time.
And for whichever device's own actor *is* `01KVXXX…`, it is exactly the "full replay on every boot, forever" the original report described.

Either way the defect is the same one and it is a lifecycle defect, not a correctness defect.
The fallback to full replay is correct and documented: an unreadable snapshot is a cache miss, no data is at risk.
What is wrong is that a cache entry which is *provably* unreadable needs `outl doctor --repair` to disappear, and a user has no reason to run a maintenance command over a cache.

## What we chose

`outl_core::snapshot::gc` (`crates/outl-core/src/snapshot/gc.rs`) is the single owner of the verdict.

### The rule is about the reader, not the writer

The obvious rule — "drop the snapshots of actors that are gone" — is wrong twice over.

It is **unanswerable**, which is [RFC 0211](0211-state-that-leaves-a-boundary.md)'s trap arriving unchanged.
An actor whose `ops-<actor>.jsonl` is not on this disk right now may be a peer whose log has not been pulled yet, an iCloud placeholder that has not materialized, or a file transport mid-sync.
"I cannot see it" and "it does not exist" are the same observation.

It is also **irrelevant**, which is the more useful half and the reason this RFC does not need 0211's three-condition hedge.
A snapshot is never read by its actor.
It is read by exactly two things:

1. `snapshot::read_best_from_disk`, this device's boot selector.
   It reads `snap-<own actor>.bin` unconditionally, and otherwise ranks every candidate by its highest cutoff HLC (tie-broken by filename) and returns **precisely one**.
2. `outl-sync-iroh`'s `SnapshotProtocolHandler`, which serves **only** `snap-<own actor>.bin` to a dialing peer.

So the deciding question is not "does its author still exist" but "can the selector ever choose it again", and that one is answerable from the directory alone.

### The verdicts

| Verdict | Meaning | Action |
|---|---|---|
| `Own` | `snap-<own actor>.bin`, and it decodes | kept |
| `Selected` | the candidate a boot with no own snapshot would adopt | kept |
| `Superseded` | decodes, is not ours, and another candidate outranks it under the selector's **own** comparison | dropped |
| `Unusable` | read end to end, and `SnapshotBody::decode` refused it | dropped |
| `Inconclusive` | anything we failed to *read* rather than read and rejected, plus a body claiming a schema newer than ours | kept |

Three details carry the weight.

**`Superseded` uses the selector's comparison, not a better one.**
`gc::selection_key` is literally the key `read_best_from_disk` ranks on.
A cleverer comparison — cutoff domination, say — would eventually drop a file the selector would have picked.
Cutoffs only move forward, so a candidate that is not today's winner is not a future one either.

**`Own` is never compared against anyone.**
The selector reads it first and does not rank it, so "a peer is further ahead" is not evidence against it.
It is also the file the sync transport serves, and the one file this device cannot recover from anywhere else.

**A `SchemaMismatch` naming a version above ours is kept.**
That body was written by a newer build sharing this workspace.
Two builds on one device resolve to the same write actor and therefore the same `snap-<actor>.bin`, so deleting it buys nothing and starts a delete/rewrite ping-pong between them.
A version *below* ours has no future reader on this machine and is dropped.

### What it refuses

- **"I could not read it" is not "I read it and it is garbage."**
  A permission error, an `EIO`, an unmaterialized placeholder — none says anything about the bytes, and there is a `remove_file` on the other side of the verdict.
  This is the sentence `outl doctor` already says about this directory; the GC says it in code.
- **A verdict is re-checked against the bytes it was computed from.**
  `gc::prune` re-stats the file and refuses when its length or mtime moved.
  A peer pull publishes into this directory with a `rename`, and a co-resident process writes its own snapshot here, so a path can come to name a body that is current and wanted between the survey and the delete.
- **Nothing outside the snapshots directory is touched.**
  `parent ==`, not `starts_with`, for the reason `device/gc.rs` spells out.
  `prune_tmp` has no survey to carry a directory, so it requires the parent to be named `snapshots`.
- **A body the survey could not stamp is kept.**
  No `(len, mtime)` means nothing to re-check against, and a delete on unverifiable evidence is what this module exists to refuse.

### Where it runs, and why not on boot

Not on a schedule, and not as a boot-time sweep.
A sweep on every launch means reading and decoding 54 MB to reclaim disk that is not costing anything yet — the cost root `CLAUDE.md` invariant 11 says to attribute before letting it decide.
Instead the GC runs at the two moments the evidence is already in hand:

- **After the background writer publishes a snapshot**, on the worker thread that just serialized and fsynced a multi-MB body.
  That is the moment the directory *gains* a file, it is off the hot path, and it fires once per `[snapshot] threshold` ops.
  `gc::sweep` is that entry point.
- **When the device's own snapshot fails to decode**, `gc::drop_own_if_unusable` drops it before the error propagates.
  This is the "full replay on every boot, forever" case: the selector reads that exact file first on every launch.
  This boot still full-replays, exactly as before; the next one does not have to.

`read_best_from_disk`'s directory scan **is** `gc::survey` — one decode loop, one owner, so the selector and the GC cannot develop separate opinions about which file matters.
Exactly one decoded body is resident at a time, so a survey does not spike a mobile boot by the size of the directory.
It nonetheless **prunes nothing**, and that is invariant 10 doing its work.
Opening a workspace is something `outl doctor` does in its documented **read-only** mode, and `ops_guard.rs` restores `ops/` only.
A read that silently reclaimed 28 MB would break that promise, for a gain the next background write collects anyway.
The own-snapshot drop is the deliberate exception: one file, proven dead.
`doctor` reads this directory (`check_snapshots`) *before* it opens the workspace, so its report is taken first, and `repair.rs`'s `delete_snapshot` already degrades to "already gone".

The synchronous shutdown writer (`Workspace::save_snapshot`) deliberately does not sweep either: it runs while a user waits for the process to exit.

### Abandoned scratch files

`write_to_disk` composes in `snap-<actor>.bin.tmp.<ulid>` and publishes with `rename`, so a killed process leaves one behind and nothing ever removed it.
The name is unique per write because the shared one it replaced let two writers for the same actor share an inode — the loser of the `rename` wrote on through a path that now pointed at the published snapshot, so a boot read a torn body (a slow boot, never a lost note).
That also means an abandoned scratch is no longer overwritten by the next write, which is what this collector now carries.
`gc::stale_tmp` / `gc::prune_tmp` collect them past `STALE_TMP_TTL` (24h), the same shape and the same reasoning as `device/gc.rs`'s `STALE_SCRATCH_TTL`.
They stay out of the snapshot listing on purpose: a scratch file is a write that never became a snapshot, so reporting one as "a snapshot whose reader is gone" invents a cache entry that never existed.

## Why not the alternatives

**Content-addressed snapshots (`snap-<hash>.bin` plus a small per-actor pointer), to deduplicate the two byte-identical bodies.**
Rejected, and invariant 11 is why: **attribute the cost before you let it decide.**
The 54 MB is caused by *never deleting*, not by per-actor naming.
Plain deletion removes it — under this RFC `~/outl-p2p` converges to at most two files — so dedup would be paying for a problem already solved.
And it would not be free.
A content-addressed store needs a blob format, a pointer format, a new schema version, and its own answer to "who deletes the blob when the last pointer goes".
That is this RFC's question again, in a harder shape, with a dangling-pointer failure mode a per-actor file cannot have.
It also buys least where it looks best: two identical bodies only exist because one device rotated actors, and after a rotation the older file is *already* the thing the selector wants, so it is insurance rather than waste.
If a future workspace genuinely holds many distinct devices' snapshots at once, this becomes worth re-opening; today it is a format change to avoid an `unlink`.

**"Drop the snapshots of actors with no `ops-<actor>.jsonl`."**
Rejected above: unanswerable *and* irrelevant.
Worth recording what it would have done to the real workspace: **nothing**.
All four snapshot actors in `~/outl-p2p` still have a live ops file, so the rule that looks like the obvious fix does not fire on the case that motivated it, while still carrying the risk of deleting a snapshot whose peer log simply has not arrived.

**A boot-time sweep, or a periodic one.**
Rejected on cost attribution (above).
The disk a stale snapshot occupies costs nothing until something reads the directory, and the moment something reads the directory is the moment the sweep is free.

**Read only each candidate's header (postcard is sequential, so the `cutoff` prefix decodes without touching the 13 MB body) to make the sweep cheap enough for boot.**
Rejected: it needs a second struct that must stay in lockstep with `SnapshotBody`'s field order, it bypasses the `content_hash` check so it can never prove decodability, and it buys only disk reclamation timing.
Two definitions of one wire format is a worse trade than a slightly later sweep.

**Add a bincode reader, or a schema-3 → schema-4 converter.**
Rejected, and it was already rejected: `snapshot.rs`'s module doc says a format change here needs no converter, because the worst case of any format change is one slower boot.
Nothing in this RFC touches the fallback.

**Leave it to `outl doctor --repair`.**
That is the status quo, and it is the defect.
A pure cache entry that is provably unreadable should not require a maintenance command, and `--repair` is a writing mode a user runs when they suspect damage — not a cache janitor.
`doctor` stays useful as the on-demand sweep; it is no longer the only one.

## The opposite direction

**What this makes worse.**
Every wrong verdict here costs one full op-log replay on one boot for one device.
That is the whole downside surface, and it is real: on a 210k-op workspace a replay is the slow boot the snapshot exists to avoid.
Three specific ways a wrong verdict can happen, all left in deliberately:

- The `prune` re-check is `(len, mtime)`, so a replacement of identical length inside one mtime tick slips through.
- A co-resident process with a *different* write actor (the desktop resolves `device_actor()`, the CLI resolves `actor_for_instance`) can have its own snapshot judged `Superseded` by this process and deleted.
  In practice that process then adopts the winner instead — which by construction has a cutoff at least as high — so it usually pays nothing.
- A second binary on a *future* postcard schema produces `Decode`, not `SchemaMismatch`, and is dropped.
  This costs that binary one replay, after which it writes a fresh snapshot; both binaries share a write actor and therefore already overwrite each other's file today, so the GC adds nothing to a fight that was already happening.

**The mirrored case.**
This RFC deletes cache, so the mirror of "deleted something still wanted" is "kept something dead".
That direction is the status quo and is explicitly preserved wherever the evidence is weak: unreadable files, unstampable files, newer schemas, and every non-winner in a directory the boot never scanned all stay.
The directory therefore still grows on a device whose own snapshot is healthy and which never rotates its actor — bounded by one file per rotation, swept on the next rotation or the next background write.
That is a deliberate floor, not an oversight.

**What this does not make better.**
`ops/` in `~/outl-p2p` holds 23 orphaned `.lock-<actor>` files and 96 `.idx` entries among 142.
Those are the same invariant-9 question in a directory that *does* sync, and they are a different lifecycle with a different owner (`outl-core`'s storage layer) and a different risk profile — a lock file is not a cache.
Nothing here touches them.

## How it cannot regress

1. **The invariant.**
   Root `CLAUDE.md` invariants 9 (what does the new home require), 10 (who was standing on the old behaviour) and 11 (attribute the cost) all apply.
   `crates/outl-core/CLAUDE.md` carries the rule under "The snapshot cache has a GC, and its rule is about the reader" — the file read on every edit to this crate.

2. **The tests**, in `crates/outl-core/src/snapshot/gc/tests.rs`, each with its own `TempDir` (invariant 9's third question — the one that went unanswered in #211 and made three doctor tests flaky).
   The refusals outnumber the removals, on purpose.
   Do not delete or relax them:
   - `the_devices_own_snapshot_is_never_dropped_for_being_behind` — the selector does not rank `Own`, and a GC that did would cost a replay on the one device that cannot get the file back.
   - `a_snapshot_from_a_newer_build_is_kept` — fails the moment someone simplifies `proves_dead` to "any decode failure".
   - `a_snapshot_that_could_not_be_read_is_kept` — an `Io` error is not evidence about bytes.
   - `a_file_replaced_since_the_survey_is_not_pruned` — fails if the two-pass re-check is dropped as redundant.
   - `nothing_outside_the_snapshots_directory_is_touched` — fails if `parent ==` is relaxed to `starts_with`.
   - `the_only_usable_candidate_is_never_dropped`, `an_empty_or_absent_directory_is_not_an_error` and `a_file_that_is_not_a_snapshot_is_left_alone` — the GC must never empty a directory of its last usable cache.
     Nor error on a device that has never snapshotted, nor touch a stranger's file.
   - `a_scratch_file_that_may_still_be_in_flight_is_kept` — the TTL is what separates debris from a live fsync.
   - `publishing_a_snapshot_collects_the_siblings_it_made_unreachable` pins the *host*: the sweep belongs on the writer's worker thread, not on boot.
   - `the_boot_selector_reclaims_nothing_on_a_read_only_open` pins the mirror of that host decision — fails the moment somebody moves the bulk sweep back onto the read path, where `outl doctor` would run it.

## Scope

**~~Not covered — `doctor`'s snapshot reporting.~~ Done, in the same branch.**
`check_snapshots` had its own read-and-decode loop and its own opinion of which snapshots were corrupt, which made it a second owner of a verdict `gc::survey` owns.
It now calls `gc::survey` and only phrases the result, and `--repair` re-asks the survey at write time rather than trusting the plan.
`Superseded` reports as `info` rather than a warning — nothing is wrong, there is disk to reclaim, and a healthy workspace should not read as sick every time the background writer publishes.
One consequence worth naming: this **widened** `--repair`, which now reclaims superseded snapshots and not only undecodable ones (54MB in 4 files down to 13MB in 1 on a real workspace).

**Not covered — `ops/`'s orphaned `.lock-<actor>` files.**
Named in "The opposite direction"; different directory, different owner, different risk.

**Not covered — the growth floor.**
A device with a healthy own snapshot that never rotates its actor still accumulates one file per background write cycle of any *other* actor writing into the same directory, until the next sweep.
Bounded and swept, but not zero.
