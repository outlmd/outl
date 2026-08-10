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

### Why JSONL specifically

- **Append-only writes** map to the filesystem cleanly.
  No WAL, no schema, no transactions to coordinate.
- **Line-delimited** means partial-write recovery is trivial: the loader skips any malformed tail line and keeps going.
- **Human-readable in a pinch.** `tail -f ops-*.jsonl` to watch what's happening; `jq` to inspect a single op.
- **`serde_json` already in the dependency graph** for the JSON envelope.
  Zero new C dependencies.

### Boot reads an index, not the whole log (RFC #137 Front A)

> **Why constant RSS came before constant boot, with the measurements:** [RFC 0137](rfcs/0137-storage-scale.md).

`JsonlStorage` keeps a bounded LRU of hot ops plus a per-actor **offset index** (`ops-<actor>.idx`, HLC → byte offset) and a per-node **secondary index** (`ops-<actor>.nodes.idx`).

On `reload` (boot) the loader streams each `.jsonl` line with a **parse-lite** pass.
That pass pulls only the two fields index-building needs — the op's HLC and the node it touches — and deliberately skips deserializing the heavy payload (`Op::Edit`'s `text_op` byte array above all).
It builds the offset + node indexes and leaves the LRU **empty**.
It does not reparse or re-allocate every op into RAM, which is what made open time (and iOS memory) scale with total history rather than with what boot actually needs — the offset index plus a small snapshot delta.

The full ops are read back **lazily on demand** through the offset index: a single `seek` + one-line parse per op (`read_op_at`), preferring a warm LRU hit when there is one.
The `Storage` read methods are driven off the index — `all_ops`, `ops_since`, `ops_since_per_actor`, `ops_for_actor`, `ops_for_node`, `last_ts_per_actor`.
So they return the **complete** op set — the same set + HLC order as before — regardless of what the LRU currently holds.
The LRU is purely a RAM bound now; the index is the complete logical view.
`last_ts_per_actor` and `ops_since_per_actor` (the snapshot-boot delta) answer straight from the index keys, so the common boot touches only the index and the recent tail, never the full log.

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

Tracked: <https://github.com/avelino/outl/issues/1>.

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
| HLC clock skew | `uhlc` clamps to avoid runaway logical counter | Tracked in HLC config; rare in practice |

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
