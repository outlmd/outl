# Storage

`outl-core` does not know what disk looks like.
It speaks to storage through a single trait.

## The trait

```rust
pub trait Storage: Send + Sync {
    /// Append an op. Must be durable before returning Ok.
    fn append_op(&mut self, op: &LogOp) -> Result<(), StorageError>;

    /// Append a batch of ops. Durable before returning Ok — one fsync
    /// for the whole batch. Default impl loops `append_op`; backends
    /// override to amortize the durability cost.
    fn append_ops(&mut self, ops: &[LogOp]) -> Result<(), StorageError>;

    /// Return all ops with HLC > ts, in HLC order.
    fn ops_since(&self, ts: Hlc) -> Result<Vec<LogOp>, StorageError>;

    /// Return all ops touching the given node.
    fn ops_for_node(&self, id: NodeId) -> Result<Vec<LogOp>, StorageError>;

    /// Return all ops created by the given actor.
    fn ops_for_actor(&self, id: ActorId) -> Result<Vec<LogOp>, StorageError>;

    /// Return the most recent HLC per actor (vector clock for sync).
    fn last_ts_per_actor(&self) -> Result<HashMap<ActorId, Hlc>, StorageError>;

    /// Return all ops in HLC order. Used for full replay on open.
    fn all_ops(&self) -> Result<Vec<LogOp>, StorageError>;

    /// Per-actor delta for snapshot boot: every op whose HLC is above the
    /// cutoff of its OWN actor (or whose actor is absent from the map).
    /// Default impl filters `all_ops`; backends may override for speed.
    fn ops_since_per_actor(
        &self,
        cutoff: &BTreeMap<ActorId, Hlc>,
    ) -> Result<Vec<LogOp>, StorageError>;
}
```

Snapshots are **not** a `Storage` responsibility — see [Snapshot strategy](#snapshot-strategy).
The op log is all `Storage` owns.

`StorageError` is the storage trait's typed error (`thiserror`).

---

## The only persistent backend: JsonlStorage

`JsonlStorage` is the storage.
It's what every client (`outl-cli`, `outl-tui`, `outl-mobile`) opens.
There is no flag, no config knob, no fallback to anything else.

### Layout

```text
<workspace>/
└── ops/
    ├── ops-<this-actor>.jsonl    ← we only ever write here
    ├── ops-<peer-actor>.jsonl    ← read-only mirror of another device
    └── ...
```

Each device writes to **exactly one** file, named by its actor id.
Reads merge every `ops-*.jsonl` in the directory back into a single HLC-ordered op log.
That's it.

### Why "one file per actor"

This is the whole reason JSONL exists in the first place. iCloud Drive, Syncthing, Dropbox, any folder-level sync transport: they all reconcile **per file**.
Last-write-wins per path.
If two devices share one log file they race on every byte; the loser's ops vanish silently.

Per-actor files turn that race into a no-op.
Each device's file is append-only and owned by exactly one writer.
Sync transport ships the bytes; the merge happens inside `outl-core`'s CRDT, not at the filesystem layer.
Zero coordination, zero conflicts, zero data loss.

**One thing in this codebase can write a file it does not own, and it is why the sentence above is a premise rather than a fact about the filesystem.**
`outl compact --apply` rewrites `ops-<actor>.jsonl` in place ([Compaction](#compaction)).
Nothing stops it from rewriting a *peer's* file: `flock(2)` is advisory and machine-local, so it can never arbitrate between devices ([`docs/clients.md`](clients.md)).
A shortened copy of a peer's path is a competing version of that path, and every file transport resolves that last-write-wins.
On iCloud / Syncthing / a shared FS the peer's longer copy loses, and its unshipped ops are gone with no error anywhere.
Compaction therefore narrows itself to the actor **this device writes under**, and lifting that needs an explicit `--force`.
The ownership rule holds because compaction keeps it, not because the filesystem enforces it.

### Where the actor id lives — outside the workspace

The guarantee above is only as strong as "one actor id per device".
The id therefore lives in the **device store**, a directory outside every workspace:

```text
$OUTL_DEVICE_DIR, else $XDG_CONFIG_HOME/outl, else ~/.config/outl/
├── machine-id                  ← this device's id + its host binding
├── actor                       ← device-wide actor (desktop / mobile)
└── actors/
    ├── <workspace-id>          ← that workspace, at the directory it was bound to
    └── <workspace-id>.<hash>   ← a second *copy* of it on this same device
```

It used to live at `<workspace>/.outl/config.toml`, and that was silent data loss waiting for the right transport.
Syncthing, Dropbox, NFS, a shared volume and `git clone` all replicate `.outl/`, so two devices read the same `actor_id`.
The per-actor `flock` does not save you — `flock(2)` is advisory and machine-local, so each device takes its lock successfully and both append to one file.
iCloud Documents hid the bug by dropping dot-prefixed paths, so `.outl/` never travelled; that is an accident of one transport, not a design.

#### The binding is per *directory*, not per workspace id

`WorkspaceId` is persisted at `<root>/.outl/workspace-id`, i.e. inside the bytes a copy carries away.
`cp -R notes notes-backup` therefore yields two directories holding one workspace id, and keying the actor on the id alone hands both of them the same actor.

That is worse than sharing a file.
The P2P transport hashes the workspace id into its gossip topic, so the two copies reconcile *as one workspace*; op identity is `Hlc { physical, logical, actor }` and dedup is by `ts`, so two independent HLC generators running under one actor mint colliding identities and genuinely distinct ops are dropped as duplicates.

So each binding records the workspace **root** next to the actor.
A root that does not match forks a second actor — unless the recorded root is *provably gone*, which is what a plain move or rename looks like and must keep its actor.
"Provably gone" means the path no longer exists, or no longer holds this workspace id.
Anything unreadable (unmounted volume, permission error) counts as still live: one extra actor wastes a file, one shared actor loses data.

#### The device store is fingerprinted too

`~/.config/outl` is outside the *workspace*, not outside everything.
Migration Assistant, a Time Machine restore, a VM or container image, `chezmoi` / Mackup, and `$HOME` on NFS all replicate it, and the clone carries the same `machine-id`.
Without a second signal both machines would answer "yes, that claim is mine" and nothing would self-heal.

`machine-id` therefore binds its ULID to a hash of an OS-provided identifier of the physical machine — `/etc/machine-id` on Linux, `IOPlatformUUID` on macOS, `MachineGuid` on Windows.
When the stored binding and the current host disagree the store was cloned: the machine id is reminted, every actor binding stamped with the old one reads as stale, and each workspace forks a fresh actor on its next open.

Where the platform exposes no such identifier the check is inconclusive and deliberately does nothing.
That is **iOS today**, where a restored device backup is exactly this hazard — a known gap, closable only by putting a writer fingerprint in the op log itself.
Forking on every open would be its own bug.

#### Adopting the legacy `actor_id`

`[workspace] actor_id` in `config.toml` is a **legacy** value, adopted by a device only when `[workspace] actor_claimed_by` names that device.
The marker is stamped when the config is **created**, not on first open, because it is only trustworthy if it is already inside the bytes a copy carries away: the default transport is iroh, which ships ops, `workspace-id` and snapshots and never `config.toml`, so a claim written on first open reaches nobody.

A workspace created before the device store existed has no claim, so **every** device forks on its first open, leaving `ops-<legacy>.jsonl` untouched and still read.
Nothing is lost — readers merge every `ops-*.jsonl` in the directory — and no two devices can land on one file.
The full table is in `crates/outl-ws/src/actor.rs`.

`$OUTL_DEVICE_DIR` overrides the location — used by the test suite (via the repo's `.cargo/config.toml`) and by containers that need a throwaway identity without discarding the user's preferences.

**It also moves the iroh identity now, and that rotates the device's node id.**
`outl_sync_iroh::default_device_dir` (`crates/outl-sync-iroh/src/device.rs`) honors the same variable, joining an `iroh/` subdir, so `~/.outl/identity.key` moves to `$OUTL_DEVICE_DIR/iroh/identity.key` too.
The iroh identity key **is** the device's node id, so a deployment that already exports `$OUTL_DEVICE_DIR` — a container, a sandboxed CI job — comes back up under a **new** node id the first time it runs a build carrying this change.
Every peer's `peers.json` still lists the old one, so the device reads as permanently offline until it is re-paired.
This is deliberate: the actor binding and the iroh identity are both device-local state about the same device-local resource, so a variable that says "this process is a different device" has to move both, or it isn't isolating anything.
Point the variable at a persistent path (not a fresh tmpdir per run) if you want a stable node id under it, and re-pair once after the move.

#### Cleaning up bindings for workspaces that no longer exist

`actors/` gains a record per workspace this device opens and, until recently, never lost one — so a workspace you deleted keeps its binding forever.
`outl doctor` reports how many name a directory that is gone, and `outl doctor --repair` drops them after copying each into that run's `.outl/repair-backup/` generation.

The rule is deliberately conservative, because dropping a binding is not free: the next open of that workspace mints a **fresh** actor, which is a second `ops-<actor>.jsonl` for a device that already had one.
A binding goes only when its root is gone, its root's **parent** directory is still present, and the record is past the TTL.
The parent check is what separates a deleted folder from an unmounted drive, an unmapped network volume, or an iCloud folder this machine has not downloaded — all of which read as "missing" and must keep their bindings.
Anything the check could not *read* (rather than observed to be absent) is kept too.

Full behaviour and the backup path: [doctor.md → The device store's stale actor bindings](doctor.md#the-device-stores-stale-actor-bindings).

### Why JSONL specifically

- **Append-only writes** map to the filesystem cleanly.
  No WAL, no schema, no transactions to coordinate.
- **Line-delimited** means partial-write recovery is trivial: the loader skips any malformed tail line and keeps going.
- **Human-readable in a pinch.** `tail -f ops-*.jsonl` to watch what's happening; `jq` to inspect a single op.
- **`serde_json` already in the dependency graph** for the JSON envelope.
  Zero new C dependencies.

### Boot reads an index, not the whole log (RFC #137 Front A)

> **Why constant RSS came before constant boot, with the measurements:** [RFC 0137](rfcs/0137-storage-scale.md).

`JsonlStorage` keeps a bounded LRU of hot ops plus a per-actor **offset index** (`.ops-<actor>.idx`, HLC → byte offset) and a per-node **secondary index** (`.ops-<actor>.nodes.idx`).

On `reload` (boot) the loader streams each `.jsonl` line with a **parse-lite** pass.
That pass pulls only the two fields index-building needs — the op's HLC and the node it touches — and deliberately skips deserializing the heavy payload (`Op::Edit`'s `text_op` byte array above all).
It builds the offset + node indexes and leaves the LRU **empty**.
It does not reparse or re-allocate every op into RAM, which is what made open time (and iOS memory) scale with total history rather than with what boot actually needs — the offset index plus a small snapshot delta.

The full ops are read back **lazily on demand** through the offset index: a single `seek` + one-line parse per op (`read_op_at`), preferring a warm LRU hit when there is one.
The `Storage` read methods are driven off the index — `all_ops`, `ops_since`, `ops_since_per_actor`, `ops_for_actor`, `ops_for_node`, `last_ts_per_actor`.
So they return the **complete** op set — the same set + HLC order as before — regardless of what the LRU currently holds.
The LRU is purely a RAM bound now; the index is the complete logical view.
`last_ts_per_actor` and `ops_since_per_actor` (the snapshot-boot delta) answer straight from the index keys, so the common boot touches only the index and the recent tail, never the full log.

### The index sidecars: one owner, and a lifecycle

> **What 134MB of dead cache cost, and what the GC refuses to touch:** [RFC 0265](rfcs/0265-index-sidecar-lifecycle.md).

`outl_core::storage::sidecar` is the **single owner** of everything about a sidecar except its payload.

| Fact | Owner |
|------|-------|
| The filename for `(actor, scope, kind)` | `sidecar::file_name` / `sidecar::path_for` |
| The complete set for one `(actor, scope)` | `sidecar::paths_for` |
| Temp-and-rename publishing | `sidecar::write_atomic` |
| How a bulk write reaches the kernel | `sidecar::buffered` / `WRITE_BUF_BYTES` |
| Load-or-signal-rebuild, save, append | `sidecar::load_entries` / `save_entries` / `append_entry` |
| Which files may be collected | `sidecar::gc` |

The two index *types* stay distinct — `OffsetIndex` maps HLCs to offsets, `NodeIndex` maps nodes to `(HLC, offset)` lists — because their payloads genuinely differ.
What was duplicated was the lifecycle around them, and a second copy of the filename is exactly how a whole generation of sidecars survived a rename unnoticed.

Names:

| Layout | `.jsonl` | sidecars |
|--------|----------|----------|
| `PageScope::Global` | `ops/ops-<actor>.jsonl` | `ops/.ops-<actor>.idx`, `ops/.ops-<actor>.nodes.idx` |
| `PageScope::PerPage(slug)` | `ops/<actor>/<slug>.jsonl` | `ops/<actor>/.<slug>.idx`, `ops/<actor>/.<slug>.nodes.idx` |

Every name is **dot-prefixed**, which is the point of the section below: `ops/` is deliberately not a dotfile so that it syncs, so an undotted sidecar inside it rides the file transport to every other device.
A synced index can arrive torn in the middle with an intact tail, pass the boot freshness check, and feed a wrong middle offset into `read_op_at` — silent op loss.

The per-page names carry the **slug**, not the actor.
An actor-derived name gives every page shard of one actor the same index file, with offsets into two different `.jsonl` files interleaved in it.

#### When a sidecar may be deleted

`sidecar::gc` answers that, and the question it asks is **not** "does this actor still exist".
`ops/` is full of peers whose logs have not been pulled yet, and [RFC 0211](rfcs/0211-state-that-leaves-a-boundary.md)'s trap applies unchanged: an unplugged drive and a deleted device are the same observation.
The question is "can the reader ever compose this name again", which is answerable from the filename alone.

| Verdict | Meaning | Prunable |
|---------|---------|----------|
| `Live` | the name `sidecar::file_name` composes today | no |
| `Legacy` | the same name without its leading dot — nothing composes it, so nothing reads it | yes |
| `AbandonedScratch` | a `*.tmp.<ulid>` older than the TTL: a write that never renamed | yes |
| `Inconclusive` | a temp that may still be in flight, or a file we could not stat | no |

Everything else in `ops/` is never surveyed at all: a `.jsonl` in any spelling (a peer's, or a sync tool's conflict copy), a `.lock-<actor>`, and any name that cannot be attributed to an actor and a kind.
A file we cannot account for is not a file we proved dead.

An index is a pure cache rebuilt from the `.jsonl` beside it, so the whole cost of a wrong verdict here is **one slower boot**.
That is milder than RFC 0211's stake and milder than [RFC 0258](rfcs/0258-snapshot-cache-lifecycle.md)'s.
The caution here is calibrated to it rather than inherited.
What does not relax: a file that cannot be stat'd is kept, a verdict is re-checked against the `(len, mtime)` it was computed from before the unlink, and nothing outside the surveyed directory is touched.
The rule also **disarms itself** — the legacy name is derived from the live one, so if `file_name` ever stopped emitting a dot the two collapse into one string and every verdict becomes `Inconclusive`.

`.lock-<actor>` is deliberately out of scope.
A lock is arbitration state, not cache: `ActorWriteLock` flocks that path, and removing one a live process holds lets the next process create a fresh inode, flock it, and write to the same `ops-<actor>.jsonl`.
Every one of them is also 0 bytes, so there is no reclaim to weigh against that.
`outl doctor` reports the **count** instead, because far more locks than op logs means the device has been minting ephemeral actors.

#### The bulk write is buffered, and that is load-bearing

`save_entries` emits one line per index entry, and a cold boot on a 217,811-op workspace rebuilds 435,622 of them across 40 files.
Unbuffered that was one `write(2)` per entry: **71.5s of a 72.0s cold boot**, ~95% of it blocked in the kernel.
Through `sidecar::buffered` (256KiB) the same 47.5MB takes under a second, and the measured cold boot drops to 0.9–2.3s.

Two things keep it correct.
The flush is `BufWriter::into_inner()`, not `Drop` — `Drop` flushes and discards the error, which on a durability path makes an ENOSPC on the last buffer look like success.
And `append_entry`, the per-op hot path, stays unbuffered on purpose: one line per call into a file it opens and closes has nothing to batch.

This is why the fix could not wait for a follow-up.
`outl compact --apply` deletes every sidecar it invalidates, so each compaction armed a 70-second freeze on the next open.

Where it runs: `outl doctor` reports, `outl doctor --repair` collects, and `outl compact` clears the full set for every actor it rewrote.
It is the one thing `--repair` deletes inside `ops/`, so `OpsDirGuard` is told about the announced paths up front rather than restoring them afterwards.
Unlike every other deletion `--repair` performs, these are **not** backed up first.
Nothing could read the copy, they are reconstructible from a file sitting next to them, and copying 134MB into `.outl/repair-backup/` would double the disk the user is trying to reclaim.

### Why the directory is named `ops/`, not `.ops/`

iCloud Documents and a few other sync transports skip dot-prefixed paths during cross-device sync.
A dotted directory silently breaks multi-device workspaces, with no visible failure mode until the user opens the second device and sees nothing.
The non-dotted name pays a "visible directory" cost for guaranteed sync coverage.

### What lives outside `ops/`

- `.outl/config.toml` — creation timestamp, the legacy `actor_id`, and the `actor_claimed_by` marker naming the one device that adopted it.
  **Do not assume this file is device-local**: it is inside the workspace, so every transport except iCloud replicates it.
  The actor a device actually writes under comes from the device store above, never from here.
- `.outl/.lock` — workspace lock file.
  Local, never synced.
- `.outl/orphans.log` — diagnostic from the reconcile pipeline.
  Local.
- `.outl/peers.toml` — peer registry for P2P sync.
  Local.

Anything that doesn't make sense to share between devices stays under `.outl/`.
The synced surface is `ops/` plus the `.md` / `.outl` (sidecar) projection.

> **A sidecar hash match is not evidence the `.md` came from the op log** — what that cost on a real workspace, and what re-projection is allowed to overwrite: [RFC 0210](rfcs/0210-md-content-outside-op-log.md).

---

## The test double: MemoryStorage

`MemoryStorage` is a pure `Vec<LogOp>`, no disk (and no snapshot — an in-memory workspace has no `root` to cache under).
Used by:

- `Workspace::open_in_memory` — when a caller wants a workspace that never touches the filesystem.
- The test suites of `outl-core`, `outl-actions`, `outl-cli` — every place that previously called `SqliteStorage::open_in_memory()`.

Not a sync backend.
No per-actor file, no merging.
Lives only to keep tests fast.

---

## Roadmap backend: ChronDbStorage (issue #1)

[ChronDB](https://chrondb.com/) is a git-backed database with native time-travel queries.
The win for outl:

- **History as a feature**, not an afterthought.
  Every op is a git commit.
- **Time-travel queries**: "show me the workspace as of 2026-04-01".
- **Branching**: workspace branches that can be merged.

### What ChronDB needs to gain first

- **Embedded mode** — no external server, ships as a library.
- **Secondary indices** — fast lookup by `node_id` and `actor`.
- **Stable Rust client** — without that, integration is painful.

Until those land, ChronDB is the future, not the present.

### How the switch will happen

When ChronDB is ready, the PR adds `outl-core/src/storage/chrondb.rs` implementing `Storage`, plus an `outl init --backend chrondb` flag in `outl-cli`.
The `Storage` trait absorbs the new impl — no change in `outl-core/src/tree.rs`, no change in `outl-md`, no change in the TUI.
That's the whole point of the trait.

Tracked: <https://github.com/outlmd/outl/issues/1>.

---

## What `outl-core` does NOT know

- File paths — storage opens itself.
- Locking — `outl-core::WorkspaceLock` is a separate concern, handled at the workspace boundary, not inside storage.
- Workspace layout — storage knows nothing about `pages/` or `journals/`.
  Those live one layer up.
- Whether it's running on disk or in memory.

---

## Concurrency

- `Storage` is `Send + Sync`.
  `JsonlStorage` uses `RwLock` around its in-memory cache; reads are concurrent, writes serialize.
- `append_op` writes one line, then flushes.
  Crash-safe at line granularity: a partial write produces an unparseable tail line, which the loader skips on next open.
- `append_ops` writes every line, then flushes **once** for the whole batch.
  Same crash-safety at line granularity, but durability is amortized: `sync_all` (`F_FULLFSYNC` on macOS, ~4ms) fires once per batch instead of once per op.
  The batch is validated (foreign-actor guard) and serialized before a single byte is written, so a rejected batch leaves the file untouched; an empty batch is a no-op.
  On `Ok` the whole batch is durable; a crash mid-batch leaves a durable prefix with a possibly-torn last line, which the next append's torn-tail self-heal recovers.
  `append_op` is just `append_ops` of one — both share the torn-tail heal and index-mirroring path.
- **Glued-op recovery on read.**
  `JsonlStorage::reload` parses each line with a streaming `serde_json::Deserializer`.
  A line carrying two (or more) concatenated JSON objects with no separating newline (`…}}}{"ts":…`) is recovered into all its ops instead of being dropped.
  That signature is what an interleaved, non-atomic concurrent append produces; the recovery means an external writer that glued two ops together never silently loses the user's content.
  A recovered line is logged at `warn` (it still signals a writer that should have serialized).
  The op log dedups by op id, so re-reading a recovered op that another file also carries is harmless.
  Writers inside this repo must still serialize their appends — recovery is the read-side safety net, not a license to write unsynchronized (see `outl-sync-iroh` → append-serialization invariant).

---

## Snapshot strategy

> **Why boot has a snapshot, an offset index and a lazy `Doc` rather than one of the three:** [RFC 0128](rfcs/0128-boot-and-memory-at-scale.md).

A snapshot is a **local boot cache** — a projection of the materialized tree + block text that short-circuits full op-log replay on open (#109/#128).
It is owned by `Workspace`, **not** by `Storage`: `Storage` owns the op log, and the snapshot is written straight to `<root>/.outl/snapshots/snap-<actor>.bin`, never through the backend.

Why `<root>/.outl/snapshots` and not next to `ops/`?
The op log at `<root>/ops` must sync (iCloud / Syncthing), so it is deliberately not a dotfile.
The snapshot must **not** sync — it is a per-device cache — so it lives under the dotted `.outl/`.
Deriving the snapshot dir from the storage's `ops_dir` was the #156 bug.
Production passes `ops_dir = <root>/ops`, so the reader looked in `<root>/snapshots` while the writer used `<root>/.outl/snapshots`, and boot was silently inert.
The workspace `root` is now the single source of the snapshot dir.

Boot + delta:

1. `snapshot::read_from_disk` loads the body; a missing / stale / corrupt snapshot is silently ignored and boot falls back to a full replay (the op log is the source of truth — the snapshot can never corrupt state).
2. Hydrate the tree + block text from the body.
3. Replay the **per-actor delta**: for each actor `A`, every op with `hlc > cutoff[A]`, plus every op of an actor absent from the cutoff (unseen when the snapshot was taken).

The cutoff is a per-actor vector clock (`BTreeMap<ActorId, Hlc>`), not a single global HLC.
A single cutoff tracks only the snapshotting actor's high-water mark, so a legitimately-low-HLC op from a lagging peer delivered after the snapshot would fall below it and vanish from the tree though it's durably in storage (#156).
Per-actor, each op is compared against its own actor's mark, and because an actor's HLCs are monotonic the boundary is exact — no drop, no double-apply (idempotency covers the equal-HLC boundary).

Writing is driven by `Workspace::set_snapshot_policy(enabled, op_threshold)` (in-band background writer, off the calling thread) and `Workspace::save_snapshot` (synchronous, on graceful shutdown).
Snapshots are optional: a workspace with none replays the full log.

Both writers compose the body in a **per-write** scratch file — `snap-<actor>.bin.tmp.<ulid>` — and publish it with one `rename`.
The ULID is not decoration.
Two writers for one actor are routine: a threshold crossing can spawn a worker while the previous one is still fsyncing a ~13MB body, and a reload builds a second `Workspace` for the same actor whose `save_snapshot` runs alongside it.
Sharing one scratch name means sharing an *inode*, so the loser of the `rename` goes on writing through a path that now points at the published snapshot — a boot then reads a torn body, and the loser's own `rename` fails `ENOENT`.
That costs a slow boot and never a note (the op log is the source of truth and an undecodable snapshot falls back to a full replay), which is exactly why nothing reported it.
`storage::sidecar`'s index writer took the identical failure in production and was fixed the same way.

The price is that an abandoned scratch is no longer recycled by the next write the way one shared name was, so `gc::stale_tmp` has more to collect: `write_to_disk` unlinks its own scratch on every in-process failure, and what reaches the 24h sweep is a process killed between `create` and `rename`.

Nothing deleted a snapshot until [RFC 0258](rfcs/0258-snapshot-cache-lifecycle.md).
`.outl/snapshots/` gained one `snap-<actor>.bin` per actor that ever wrote one on the device and lost none, so a real workspace reached **54MB in four files** — one of them a schema-3 bincode body no build since #207 can read.

The rule is about the **reader**, not the actor.
"Drop the snapshots of actors that are gone" is unanswerable — an unpulled peer log, an undownloaded iCloud placeholder and a dead actor are one observation — and it is also irrelevant, because a snapshot is never read by its own actor.
Its only readers are `read_best_from_disk` and iroh's snapshot responder, so the deciding question is *can the selector ever choose this file again*, which the directory alone answers.
`snapshot::gc::survey` is the single owner of that verdict; `read_best_from_disk` is built on it, so the boot selector and the GC cannot disagree.

Prunable is `Superseded` (decodes, is not ours, and loses under the selector's own key) or `Unusable` (read end to end, `decode` refused).
Kept: our own snapshot, the selected one, a **newer** schema than this build's, and anything we failed to *read* at all — "could not read" is not "read and proved bad".
It sweeps on the background writer's worker thread, never as a bulk pass on a read, because `outl doctor` opens a workspace read-only and a read-only command that reclaims 28MB has broken that promise.

### Wire format: postcard, schema 4

The body is encoded with [`postcard`](https://crates.io/crates/postcard) — serde-native, varint, deterministic given a fixed in-memory layout (which is why every map in `SnapshotBody` is a `BTreeMap`, never a `HashMap`).

It was bincode through schema 3.
Every published version of bincode — 1.x and 2.x alike — is flagged unmaintained by [RUSTSEC-2025-0141](https://rustsec.org/advisories/RUSTSEC-2025-0141), with `patched = []` and no successor release.
Because `outl-core` is [published for embedding](embedding.md), that advisory failed any downstream `cargo deny` / `cargo audit` gate the moment a project added the crate.
A version bump inside bincode would not have fixed that (issue #207).
postcard is actively maintained, `MIT OR Apache-2.0`, and smaller on the wire, which also matters because snapshots ship between peers over iroh.

The change is deliberately not backwards compatible, and that costs nothing:

- `SCHEMA_VERSION` went `3` → `4`.
- A pre-#207 snapshot fails `SnapshotBody::decode` — either a parse error or a `content_hash` mismatch if it happens to parse.
- Both land on the path a corrupt snapshot always took: full op-log replay, nothing surfaced to the user.
- A peer still on the old build ships a snapshot this build skips (`read_best_from_disk` drops undecodable candidates and keeps scanning), so cross-version pairing degrades to a replay rather than an error.

The cost is one slower boot per device, once.
The op log is the source of truth; no snapshot format change can lose data.

---

## Compaction

The op log is append-only and nothing compacted it until [RFC 0256](rfcs/0256-op-log-compaction.md) ([issue #110](https://github.com/outlmd/outl/issues/110)).
It replays on every boot and ships whole to every newly paired device, so its size is the ceiling on both.
A real 2,574-page workspace carried **262MB of `ops/` for 28MB of markdown**.

`outl compact` removes one shape and only that shape: a `Move` that restates the placement its own `Create` made immediately before it.
Every other op is copied through byte for byte.

**Why the predicate is six conditions and not one.**
`Op::Create` is idempotent — re-applying it for an existing node does nothing — so the adjacent pair reads two ways that are indistinguishable in the file.
When the `Create` really created the node, the `Move` after it is inert.
When the node already existed, the `Create` is the inert one and **the `Move` is doing the work**, which is exactly what a trashed-then-restored block looks like, since deletion is `Move(node, TRASH_ROOT)`.
So a pair qualifies only when, over the *merged* log of every actor in `Hlc` order (whose `Ord` is `(physical_ms, logical, actor)` — the actor tiebreak is part of the type), the `Create` is that node's first appearance, is the sole `Create` for it, the parent is not `NodeId::root()`, and a full replay confirms the node already sat at that exact parent and position when the `Move` arrived.

Soundness against the log we hold is a decision procedure, not a heuristic: HLC-order application makes the tree a pure function of the sequence, so an op that leaves the state unchanged can be removed and every later op observes the identical state.
`Move` is the only variant that can change a node's placement once it exists, and `old_parent` / `old_position` are read by nothing but `undo_op`, which never sees an op absent from the log.

Against ops we do **not** hold, the argument is causal and narrower, which is what the 30-day settling horizon is for.
The ops are inert the moment they are written; the horizon is about peers that have not delivered theirs yet.
A device offline longer than the horizon could hold a `Move` on one of these nodes, stamped in a window where this device wrote nothing, and would then materialize that block at a different position — not data loss, still a divergence.
`--no-horizon` is for a workspace with no other device.

**The horizon is measured against the wall clock, not against the log alone.**
The cutoff is `min(newest op, now) - 30 days`.
Anchoring it on the newest op by itself hands the setting to whoever has the worst clock.
`hlc.rs` is hand-rolled with no drift bound and `observe` adopts a peer's higher `physical_ms` unconditionally (see the failure-modes table below), so one op stamped in 2040 puts the cutoff ten years in the past.
Every real op then falls below it, condition 6 stops rejecting anything, and a run the user deliberately did *not* give `--no-horizon` silently behaves as if they had.
Clamping is strictly more conservative — on a healthy log the newest op is in the past and `min` returns it unchanged.
Pinned by `a_future_stamped_op_cannot_disarm_the_settling_horizon` and its mirror `the_horizon_is_unchanged_when_every_op_is_in_the_past`.

### Whose file `--apply` may rewrite

**This device's own `ops-<actor>.jsonl`, and nothing else.**
See [Why "one file per actor"](#why-one-file-per-actor): shortening a peer's file publishes a competing version of a path that every file transport resolves last-write-wins, and the peer's unshipped ops die there silently.
Whether a given workspace rides such a transport is not knowable from the code — a `transport = "iroh"` workspace living in a Dropbox folder is exactly the trap — so the condition compaction refuses on is the provable one: *is this file mine?*

The actor comes from `outl_ws::actor::resolve_device_actor`, the same resolution every other command uses, so "the file compaction may rewrite" and "the file this device appends to" cannot become two answers.
`plan_compaction` still decides against the **merged** log of every actor, and the dry run still reports the whole log's dead weight; only the rewrite narrows (`CompactPlan::restricted_to`).
`apply_compaction_as` is the guarded entry point and returns `CompactError::ForeignActorFile` for anything else; the unguarded `apply_compaction` is the `--force` path, and the caller owns the claim that no file transport carries the workspace.

Two consequences worth stating plainly:

- **Running `outl compact --apply` on each device is the intended usage**, which [RFC 0256](rfcs/0256-op-log-compaction.md) already said for a different reason (a peer holding the full history re-delivers the dropped ops).
- **An `ops-<ephemeral>.jsonl` this device wrote in a past session is ours in fact but not provably**, since an ephemeral actor leaves no binding behind.
  Those need `--force` too.

Measured on the workspace above: **62,209 of 217,811 ops dropped (28.6%)**, `.jsonl` down 22.6%, `ops/` 262MB → 195MB, materialized tree, properties and collapsed flags byte-identical, `outl doctor` reporting `integrity OK`.
The naive "adjacent pair" rule would have matched 99.9% of all `Move` ops against the sound predicate's 94.7%; the 2,074 pairs it declines are the trashed-and-restored ones.

`apply_compaction` takes an exclusive `flock` on `.outl/.lock` plus a write lock on **every** actor, copies and fsyncs each file into `.outl/compact-backup/<timestamp>-<ulid>/` before any write, and replaces via temp + `rename`.
It **deletes** every index sidecar for each actor it rewrote rather than rebuilding them: compaction invalidates every byte offset they hold, and deleting has no failure mode in which a rebuild writes a wrong one (#129).
The set comes from `sidecar::remove_all`, not from two names spelled out at the call site, so it covers the dead generations as well — an undotted `ops-<actor>.idx` and an abandoned `*.idx.tmp.<ulid>` hold offsets into the same renumbered file.
It refuses on an unparseable record and on the `PerPage` layout.

Details of *when* it refuses, every one of them found by testing the refusal arms rather than the happy path:

- **Every file it will rewrite is re-read before any of them is written.**
  `rewrite_one` used to read its own file, so a record that stopped parsing between the plan and the apply was discovered mid-loop, with the actors sorted ahead of it already replaced.
  A refusal that has already written half of `ops/` is not a refusal.
- **A subdirectory of `ops/` that cannot be read counts as a per-page shard dir, not as an empty one.**
  The plan decides inertness against the *merged* log, so proceeding past a directory that turns out to hold shards can drop a `Move` those shards made meaningful.
  Same rule as the device store's ([RFC 0211](rfcs/0211-state-that-leaves-a-boundary.md)): an unreadable thing counts as present, because one spurious refusal costs a re-run and one wrong proceed costs the op log.

- **The actor set is re-listed under the locks, and a set that differs from the plan's is `PlanStale`.**
  The byte-length check catches a file that *changed*; it cannot see one that was *created*.
  An `ops-<actor>.jsonl` appearing between plan and apply is unlocked, unmeasured, and — the part that matters — its ops never entered the merged log inertness was decided against, so a `Move` it makes meaningful can still be dropped.
  A file that vanished reports the same way, instead of surfacing as a bare `NotFound` that reads like a bug in compaction.
- **`plan_compaction` refuses while the workspace is open, with `Busy`.**
  It writes nothing, but it reads: an append caught mid-`write(2)` looks like a truncated record, and any record that does not parse becomes `DamagedLog`, whose message sends the user to `outl doctor` over a log that was never damaged.
  A false corruption alarm on the op log is an expensive thing to spend.
- **A failure after the backup exists names the backup.**
  `report.backup_dir` used to be filled in on success only, so the one moment somebody needs that path — a rewrite that died mid-loop, with `ops/` partly replaced — was the one moment nothing printed it.
  `CompactError::RewriteFailed` carries it now.
  An index-sidecar deletion that fails does **not** abort the loop: stopping there leaves *more* stale sidecars than continuing does, so the error is recorded and the remaining actors are still invalidated.

These are pinned in `crates/outl-core/tests/compaction_refusals.rs` and in `storage/compact/tests.rs`, which exist because `tests/compaction.rs` covers what compaction *drops* and nothing covered what it *declines to touch*.

> Compaction does not shrink the *live* indexes, which are the larger half — the sidecars were ~181MB of that 262MB, more than twice the log they index.
> They are deleted and the next boot rebuilds them at the same ratio.
> What used to have no owner was the **dead** half of that number: 134MB of it was sidecars no code path reads.
> That now belongs to `sidecar::gc` — see [The index sidecars](#the-index-sidecars-one-owner-and-a-lifecycle).

## Failure modes

> **Why an acknowledged op must survive the crash, the reader and the rebuild:** [RFC 0129](rfcs/0129-op-log-durability.md).

| Failure | Detection | Recovery |
|---------|-----------|----------|
| `append_op` fails to flush | `Result` propagated to caller | Caller decides; the in-memory tree should be considered stale; `outl doctor` can reload from disk |
| Partial-write tail in a `.jsonl` | `JsonlStorage::reload` logs the unparseable line via `tracing::warn!` and skips it | Truncate that line; the next valid op is fine |
| Glued ops on one line (`…}}}{"ts":…`) from an interleaved concurrent append | `JsonlStorage::reload` streams every concatenated JSON object off the line and warns | No action — both ops are recovered on next open; dedup makes a double-read harmless |
| I/O error on one line during full replay | `warn!` per line; the read **skips that line and keeps going** | Only the damaged line is lost. After 64 consecutive I/O errors the file is treated as gone and the read stops, saying so — a hard stop on the *first* error used to discard every op after it, silently shrinking the workspace to whatever preceded the damage |
| I/O error while **building** the offset index (`rebuild_actor_indexes`) | `warn!` per record; the pass **skips that record and keeps indexing**, and refuses to persist the `.idx` / `.nodes.idx` sidecars for a run that hit any read error | Only the damaged record is missing from the index. This is the worst place to stop early: a short index never *knows* about the ops past the damage, so `MissingOp` can never fire for them and the whole row below is bypassed — the tree boots short with no error anywhere. Not caching a known-incomplete index keeps the next boot free to rebuild and recover |
| Op present in the offset index but unreadable from disk (truncated file, partial sync, bad sector) | All four index-driven reads (`ops_since`, `ops_for_actor`, `ops_since_per_actor`, `ops_for_node`) return `StorageError::MissingOp` instead of a shorter result set | Snapshot boot degrades to a full sequential replay, which re-reads the file and recovers everything around the damage. Dropping the op quietly was permanent loss: the next snapshot's cutoff comes from the *index*, so the omission would be recorded as "already folded in" and never replayed again. `ops_for_node` is the sharpest of the four — its result replays a block's `Edit` history into a fresh Yrs `Doc`, so a short read there does not shorten a visible list, it hands the user block text they never wrote (#129). `Workspace` warns and keeps the text it already has |
| A whole peer's `ops-<actor>.jsonl` exists but won't open during full replay | `error!` (not `warn!`) naming the file, and none of that actor's ops enter the replay | Deliberately **not** fatal: `reload`'s readability guard skips the same file, so the actor is absent from the offset index too, so the snapshot cutoff never claims to have folded its ops in. The first boot that can open the file replays all of them. Failing the open instead would cost availability without buying correctness |
| Sidecar lost | `outl doctor` detects missing `.outl` | `outl doctor --repair` regenerates it from the op log by re-rendering the page |
| HLC clock skew | **Nothing detects it.** `hlc.rs` is hand-rolled (there is no `uhlc` dependency and never was) and has no drift bound: `HlcGenerator::observe` adopts any peer's higher `physical_ms` unconditionally, so one device with a far-future clock drags every device that syncs with it forward permanently | No recovery path, and none is needed for *correctness*: the total order is `(physical_ms, logical, actor)`, so a skewed clock reorders ops relative to wall time but converges identically everywhere. What it costs is the 30-day compaction horizon (measured against `physical_ms`) and any user-visible timestamp. The logical counter is `saturating_add` on `u32`, so a stuck wall clock pins it at `u32::MAX` rather than wrapping — ops stay distinct via the actor tiebreak, but stop being ordered among themselves. Untracked; no issue open |

The rule behind every read-side row: **a read that returns fewer ops than the log holds must be impossible to confuse with a healthy read.**
Either recover the rest (skip the bad record and keep going) or fail loudly enough that the caller falls back to a path that can.
Anything in between writes the loss into the next snapshot's cutoff, where nothing can find it again.

Which of the two applies depends on one question: **does the offset index know about the op?**
If it does, dropping it is invisible *and* gets baked into the next cutoff — so it is a `MissingOp` error.
If the index does not know about it either (an unindexed record, an unopenable file), the omission is self-correcting on a later boot — so it is a loud log line, not a failed open.
Which makes the index build itself the load-bearing case: it is what decides which of the two a damaged byte range gets to be.

---

## What is **not** here anymore

Pre-0.5.0, outl shipped a second persistent backend: `SqliteStorage` (`.outl/log.db`, WAL mode).
It was the default for local-only workspaces and the source of an entire class of "writes go through but vanish on the other client" bugs.
`outl-cli` opened it via SQLite, `outl-tui` and mobile followed `config.toml` and opened JSONL on the same workspace, and the two backends diverged silently.

0.5.0 dropped SQLite entirely.
There is one persistent backend.
Cross-device sync is no longer a config decision; it's the only mode.
See `CHANGELOG.md` for the migration path from a 0.4.x SQLite workspace.

---

## The 3-level matching algorithm

Owned by [`markdown-format.md`](markdown-format.md#3-level-matching), which carries the levels, the tiebreakers and the edge cases.

It lives there rather than here because the algorithm is defined by the markdown dialect and the sidecar, and splitting it across two docs is the same "one owner per fact" break that this repo keeps paying for. The bulk-delete guard moved there with it.
