# RFC 0265 — Index sidecar lifecycle: one owner, and a GC

**Status:** implemented
**Crates:** `outl-core` (`storage/sidecar/`), `outl-cli` (`cmd/doctor/`, `cmd/compact.rs`)
**Related:** [RFC 0137](0137-storage-scale.md) (the indexes), [RFC 0211](0211-state-that-leaves-a-boundary.md) (state that leaves a boundary)
**Related:** [RFC 0256](0256-op-log-compaction.md) (compaction), [RFC 0258](0258-snapshot-cache-lifecycle.md) (the snapshot cache's answer to the same shape)

## What was measured

`ops/` on the maintainer's daily-driver workspace holds **77MB of op log and 243MB of index**.
The cache is three times the size of the thing it caches, and two thirds of it is unreachable.

2,560 pages, 217,811 ops, 20 op logs:

| shape | files | size | read by |
|---|---|---|---|
| `.ops-<actor>.idx`, `.ops-<actor>.nodes.idx` | 40 | 51MB | the current code |
| `ops-<actor>.idx`, `ops-<actor>.nodes.idx` | 40 | 50MB | **nothing** |
| `ops-<actor>.idx.tmp.<ulid>` | 16 | 84MB | **nothing** |
| `.lock-<actor>` | 23 | 0B | `ActorWriteLock` |

134MB of dead cache, every file dated 10–16 July, in a directory that is **deliberately not a dotfile** so that it syncs.

Three causes.
Two are lifecycle gaps; the third is a performance bug that *manufactures* one of the other two.

**The rename.**
The sidecars were moved to dot-prefixed names specifically so they would stop riding the file-sync transport.
`ops/` must sync (iCloud Documents drops dotted paths — see `docs/storage.md` → "Why the directory is named `ops/`, not `.ops/`"), but an index is a purely local boot cache.
A synced one can arrive torn in the middle with an intact tail, pass the freshness check, and feed a wrong middle offset into `read_op_at`.
The rename happened.
Nothing removed what it left behind, so 50MB of the old generation is still being copied between devices on every sync.

**The temps.**
`write_atomic` created `path.with_extension("idx.tmp.<ulid>")` and removed it only when the **rename** failed.
Every other exit — an error return from the body, a panic, a `?` — leaked one.
A process killed between `File::create` and `rename` leaked one permanently, and nothing ever collected it.

**The 70-second write.**
This is the one that reframes the other two, and it was found by a boot profiler, not by looking at the directory.

`save_entries` emitted one `writeln!` per index entry straight into a bare `File`.
A cold boot rebuilds 435,622 entries across 40 files, so that is 435,622 `write(2)` calls:

| scenario | total |
|---|---|
| cold (no `.idx`, no snapshot) | **70.0–72.0s** |
| warm `.idx`, full replay | 412ms |
| warm `.idx` + usable snapshot | 299ms |

Of a 72.00s cold boot, **71.50s was `sidecar::save_entries`** — 0.62s user + 3.07s sys against 72.00s wall, ~95% blocked in the kernel.
Same entries, same 47.5MB, through a 256KiB `BufWriter`: 0.54s.

So the 84MB of abandoned scratch is not an independent leak.
**It is a consequence.**
A write that takes 70 seconds gets interrupted — the user closes the app, hits Ctrl+C, the laptop sleeps — so 16 abandoned temps is not an exceptional outcome, it is the *expected* one.
Interrupting a 1.4-second write is the exceptional case.

And it is why this had to land in the same change as the GC rather than after it.
`compact/rewrite.rs` deletes every sidecar it invalidates — correctly, since compaction renumbers every byte offset — so **every `outl compact --apply` armed a 70-second freeze on the next open.**
Shipping the compaction without the writer fix would have shipped that.

## Why this is two problems, not one

The 134MB is the symptom.
The cause is that **two modules independently owned the same four facts** about a sidecar: its path, its temp-and-rename write, its load-or-rebuild lifecycle, and the deletion compaction performs.

`storage/index.rs` and `storage/node_index.rs` each had their own `sidecar_path`, their own copy of the JSONL load loop (~45 lines, identical but for a log label), their own `save`, their own `append_to`.
`compact/rewrite.rs` invalidated them by listing the two names it happened to know.

A second copy of a filename is exactly how a rename leaves a whole generation behind and nobody notices.
So the fix has two halves, and the GC is the *smaller* one.

## Part 1 — the seam

`outl_core::storage::sidecar` becomes the single owner of everything about a sidecar except its payload.

```
storage/sidecar/
├── mod.rs      naming, write_atomic, load/save/append helpers
└── gc/         which files may be collected
```

| Fact | Owner |
|------|-------|
| filename for `(actor, scope, kind)` | `sidecar::file_name` / `path_for` |
| the **complete** set for one `(actor, scope)` | `sidecar::paths_for` |
| temp-and-rename publishing | `sidecar::write_atomic` |
| how a bulk write reaches the kernel | `sidecar::buffered` / `WRITE_BUF_BYTES` |
| load-or-signal-rebuild / save / append | `load_entries` / `save_entries` / `append_entry` |
| which files may be collected | `sidecar::gc` |

`ActorIndex::sidecar_path` and `ActorNodeIndex::sidecar_path` are **gone**, not wrapped: every caller (`jsonl/append.rs`, `jsonl/read.rs`, `compact/rewrite.rs`, `doctor/oplog.rs`) now asks `sidecar::path_for`.
A wrapper would have kept two names for one fact, which is the thing being removed.

The two index *types* stay distinct.
Their payloads genuinely differ — `OffsetIndex` is `HLC → offset`, `NodeIndex` is `NodeId → [(HLC, offset)]` — and collapsing them would trade a real duplication for a fake one.
What is unified is the lifecycle around them.

Net effect on the crate: `index.rs` 570 → 481 lines, `node_index.rs` 380 → 310, with 357 lines of shared module replacing ~160 lines of duplication and adding the GC.

### The per-page collision the seam exposed

Making the path derivation take a `PageScope` made a latent bug visible immediately.

Under `PageScope::PerPage`, one actor owns many `.jsonl` files inside `ops/<actor>/`, and the old `sidecar_path(dir, actor)` derived the name from the **actor**.
So every page shard of one actor wrote into the same `ops/<actor>/.ops-<actor>.idx`: offsets into `home.jsonl` and `work.jsonl` interleaved in one file, attributed to whichever shard loaded it next.
`outl migrate-to-per-page-ops` ships, and `outl_actions::storage_scope::register_per_page_storages` opens every shard at once, so this is reachable by a real user.

**It is not a wrong read today**, and the honest reason matters: the boot freshness check in `try_fresh_actor_indexes` rejects the mixture and rebuilds.
A regression test written against the old behaviour (`a_per_page_shard_reads_back_only_its_own_ops`) passes on it.
That is a guard catching a design error every time, not a design — and it means the cost today is a rebuild on every shard open, forever, instead of a cached index.

Fixed: per-page sidecars carry the **slug** (`ops/<actor>/.home.idx`), so they cannot collide.

## Part 2 — the GC

Modelled on `snapshot/gc.rs` ([RFC 0258](0258-snapshot-cache-lifecycle.md)), whose lesson is that a cleanup rule is only as good as what it refuses.

### The question is not "does this actor still exist"

`ops/` is full of files from other devices.
An `ops-<actor>.jsonl` for an actor this device has never seen is normal, and RFC 0211's trap applies unchanged: an unplugged drive and a deleted device are the same observation.

The answerable question is **"can the reader ever compose this name again"**, and that comes from the filename alone.

| Verdict | Meaning | Prunable |
|---|---|---|
| `Live` | the name `sidecar::file_name` composes today | no |
| `Legacy` | the same name without its leading dot | yes |
| `AbandonedScratch` | a `*.tmp.<ulid>` older than the TTL | yes |
| `Inconclusive` | a temp that may still be in flight, or a file that could not be stat'd | no |

### Calibrating the caution honestly

An index is a pure cache rebuilt from the `.jsonl` sitting next to it.
The whole cost of a wrong verdict is **one slower boot**, never an op.

That is milder than RFC 0211's stake, where a wrong verdict forks a workspace's write actor, and milder than RFC 0258's, where a snapshot encodes a materialized tree.
This RFC says so rather than inheriting RFC 0211's paranoia wholesale — copying a caution level you have not re-derived is how a cheap operation acquires an expensive guard nobody can justify later.

What does **not** relax is the shape of the evidence.

- **A file we could not read is not a file we proved dead** — no `(len, mtime)` stamp means `prune` refuses.
- **A verdict is re-checked against the bytes it was computed from** — `ops/` is a sync target, and a file can be replaced between the survey and the unlink.
- **Nothing outside the surveyed directory is touched** — `parent ==`, not `starts_with`, for the reason `device/gc.rs` spells out.
- **A file we cannot attribute is never surveyed at all** — not `Inconclusive`, absent; `ops-not-a-ulid.idx`, `notes.idx` and `.DS_Store` produce no entry and no opinion.
- **No `.jsonl` in any spelling ever enters**, including a sync tool's conflict copies (`ops-<a> 2.jsonl`, `ops-<a>.jsonl.sync-conflict-…`).

### The rule disarms itself

The entire basis for calling `ops-<actor>.idx` dead is that `file_name` emits a leading dot, so nothing composes the undotted spelling.

If that ever stops being true, the undotted file becomes the **live** cache and this GC becomes a data-losing bug.
A comment would not survive that change.
So `gc::legacy_name` is *derived* from `file_name` by stripping the dot, and `classify` refuses — every verdict becomes `Inconclusive` — when the live and legacy spellings come out equal.

Two tests hold the proof up:

- `the_dead_name_is_derived_from_the_live_one` — the derivation, over an exhaustive loop on `SidecarKind::ALL`.
- `an_undotted_sidecar_is_never_read` (`tests/sidecar_lifecycle.rs`) — plants a *plausible but wrong* undotted index next to a real log and proves a boot ignores it.
  If someone reintroduces an undotted reader, those poisoned offsets become live and this test fails, before the GC turns into a deletion bug.

### Making the write fast enough to finish

`write_atomic` now hands `write_body` a `BufWriter` (`sidecar::buffered`, 256KiB) instead of a bare `File`.

Two details that are not incidental:

- **The flush is `into_inner()`, not `Drop`.**
  `BufWriter`'s `Drop` flushes and *discards* the result, so on a path whose entire job is durability an ENOSPC on the last buffer would look exactly like a successful write.
  `into_inner()` flushes and returns the error, and the fsync only happens after it succeeds.
- **`append_entry` is deliberately left unbuffered.**
  It is the per-op hot path: one line per call into a file it opens and closes.
  A buffer there has nothing to batch and only adds a flush that can fail.
  The two writers have different shapes and now say so in their doc comments, so the next person does not "fix" the wrong one.

Measured after, same fixture, release build, three cold runs: **0.89s / 1.30s / 2.28s** (warm: 0.42s), against 70.0–72.0s before.

`the_bulk_writer_batches_lines_into_few_syscalls` pins it: 50,000 lines through `buffered()` must reach the sink in under 500 `write` calls.
Set `WRITE_BUF_BYTES` to 1 and it reports 250,000.

### Closing the leak at the source

`write_atomic` now publishes through an RAII `TempFile` that unlinks on drop unless `keep()` is called after a successful rename.
That closes **every in-process exit path** — error return, `?`, panic — pinned by `a_failed_write_leaves_no_temp_behind` and `a_panicking_write_leaves_no_temp_behind`.

It does **not** close `SIGKILL`, iOS jetsam, or power loss.
What the buffered writer does to *those* is shrink the window: the interruptible period per file drops from ~70s to ~1.4s, so the production rate of new scratch falls by roughly the same 50×.
That is the difference between a TTL that is cleaning up after a routine event and one cleaning up after a rare one — and it is why a one-day TTL is generous rather than merely safe.
No code inside `write_atomic` can: the process never runs again.
That residue is structurally unreachable from the producer, which is precisely why the GC carries a TTL.
It is the same TTL constant `snapshot::gc` uses (`pub use`, not a second copy), because both answer the same question: "a scratch file whose publishing rename may still be seconds away".

The scratch name also now *appends* rather than replacing the extension, so it inherits the published name's leading dot.
A temp a killed process abandons today is therefore already off the sync surface — which is why the 16 real ones are undotted: they predate the rename.

## What is deliberately out of scope

**`.lock-<actor>` — 23 orphans, and they stay.**

A lock is not a cache.
`ActorWriteLock` flocks that path; the file's *existence* is not the lock.
Deleting one while a process holds it lets the next process create a fresh inode, flock it successfully, and believe it owns the same actor.
That is two writers appending to one `ops-<actor>.jsonl` — the interleaved-append corruption the read path already has to recover from.

And the cost side does not argue for it either ([root `CLAUDE.md` invariant 11](../../CLAUDE.md)): every one of those files is **0 bytes**.
The reclaim is a directory entry.
Trading an arbitration failure for that is not a trade, and "they were in the same directory as the thing I was cleaning" is not an attribution.

`outl doctor` reports the **count** instead, as `info`.
23 locks next to 20 op logs is a real signal.
Each lock is an actor this device has written under, so a count well above the number of logs means the workspace has been minting ephemeral actors.

**Per-page shard directories.**
`gc::survey` reads one directory, files only, and does not descend into `ops/<actor>/`.
Deciding which `.<slug>.idx` in there is live needs that directory's `.jsonl` set, and a wrong call deletes a live cache to reclaim a few KB.
Consequence: a workspace that migrated to per-page ops before this RFC keeps one stale `.ops-<actor>.idx` pair per actor directory, unreachable and uncollected.
Known, bounded, and left.

## Part 3 — the surface

- **`outl doctor`** surveys `ops/` *before* the storage open, because a boot rebuilds sidecars and a survey taken afterwards would be judging files the command itself created.
  It names what it found, plus a `repairable[]` line per file carrying the GC's own `verdict.reason()`.
- **`outl doctor --repair`** collects them, re-asking the GC at delete time (the plan is a listing, not an authorisation).
- **`outl compact --apply`** clears the complete set for every actor it rewrote, via `sidecar::remove_all`.

### The `ops_guard` exception

`doctor` promises to leave `ops/` byte-identical, in both modes, enforced by `OpsDirGuard` photographing the directory and restoring it.
These deletions are the one announced exception, so the guard is **told** about them up front (`capture(dir, ignore)`) rather than discovering the deletion afterwards and undoing it.

Passing them in also keeps them out of RAM: `capture` reads every non-`.jsonl` file into memory so it can restore it byte-for-byte, which on this workspace meant reading 134MB of dead cache on every `outl doctor` run.

`repair_collects_the_dead_generations_and_keeps_the_live_ones` fails if the ignore list is dropped — the guard restores every file and the repair becomes a no-op.

### No backup, unlike every other deletion `--repair` performs

RFC 0258 chose to back snapshots up.
This deliberately does not, for three reasons in descending weight:

1. **Nothing can read what would be saved** — every file here is either a name no code path composes or a temp that never became an index, so the copy is still unreadable, one directory further away.
2. **It is reconstructible from a file sitting next to it** — a snapshot encodes a materialized tree a restore would otherwise replay for, while an offset index is a byte map of an unchanged `.jsonl`.
3. **The volume inverts the point** — copying 134MB into `.outl/repair-backup/` doubles the disk the user is trying to reclaim, and hands the backup pruner a job it did not need.

## Measured outcome

Verified on a scratchpad copy of the real workspace (the original is read-only):

```
before   ops/  262MB   140 entries
after    ops/  128MB    84 entries
removed  56 files, 140,974,615 bytes (134.4MB)
         40 Legacy + 16 AbandonedScratch
kept     20 ops-*.jsonl  — byte-for-byte identical (sha256 diff clean)
         20 .ops-*.idx + 20 .ops-*.nodes.idx  (Live)
         23 .lock-* + .append.lock            (never surveyed)

cold boot, no .idx and no snapshot, release:
before   70.0–72.0 s   (71.50 s of it in save_entries)
after     0.89–2.28 s
```

`outl doctor` on the same copy also reports `20 offset index file(s) agree with their .jsonl` — the live-health check (`check_offset_indexes`) and this GC are disjoint questions and neither was taught the other's opinion.

## The general rule this is an instance of

Root `CLAUDE.md` invariant 9 asks, of state that crosses a boundary: who can write it, who can read it, how does a test get its own copy, and **what cleans it up?**

The indexes crossed a boundary twice — once into `ops/` in RFC 0137, once from undotted to dotted — and the fourth question went unanswered both times.
The first time cost 84MB of temps nothing removes.
The second cost 50MB of a generation nothing reads, *still being synced between devices*, because the rename moved the file without moving the responsibility.

A rename is a boundary crossing.
