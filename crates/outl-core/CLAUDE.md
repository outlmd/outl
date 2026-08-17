# CLAUDE.md — outl-core

The kernel.
Tree CRDT, op log, storage trait.
Nothing else.

If you break this crate, you corrupt the user's tree on sync.
**There is no second chance** to win back trust if that happens.
Treat every change as production-bound.

## What this crate owns

- `Op` enum and `LogOp` envelope
- HLC timestamps (wrapper over `uhlc`)
- `NodeId`, `ActorId` (ULID-based).
  `NodeId::from_slug(slug)` is the **single owner** of the deterministic page/journal-root id derivation (`sha256("outl-page:" + slug)[..16]`).
  Every path that materialises a page root routes here — in-app `open_or_create`, `outl-md`'s external-`.md` reconcile, `outl-actions::desync` recovery.
  Two paths (or two devices) then converge on the **same** root id for a slug instead of splitting the page across two competing roots.
  `outl_actions::page::page_id_from_slug` is a thin wrapper kept for its call sites.
- `WorkspaceId` — the stable, **shared** workspace identity (one per workspace, the same bytes on every paired device), persisted at `<root>/.outl/workspace-id`.
  This is NOT the local path: the P2P transport keys its gossip topic on this id so two devices at different paths sync as one workspace, and pairing makes the joiner adopt the host's id.
  Read-or-generated on first open (migration-safe); never written into the clean markdown.
  See `outl-sync-iroh/CLAUDE.md` → "Workspace identity is a stable shared id, NOT the path".
- `DeviceStore` / `MachineId` / `device_dir` (`device/`) — the **device-local** half of that pair: which `ActorId` this machine writes under.
  See "Actor id is device-local, and the workspace cannot hold it" below.
- Fractional indexing
- The CRDT itself: `do_op`, `undo_op`, `apply_op`, `creates_cycle`
- Append-only `OpLog`
- `Storage` trait + `JsonlStorage` (one file per actor, syncable via iCloud / Syncthing / shared FS) + `MemoryStorage` (test double)
  - **Batch append (`Storage::append_ops`).**
    Durable on `Ok` with ONE `.jsonl` fsync for the whole batch (`F_FULLFSYNC` on macOS is ~4ms; per-op fsync was the write bottleneck).
    Default trait impl loops `append_op`.
    `JsonlStorage` overrides it: it validates the foreign-actor guard over every op and serializes all lines before writing a byte (a rejected batch leaves the disk untouched, an empty batch is a no-op).
    Then it opens once, heals a torn tail once, writes every line, fsyncs once, and mirrors each op into both sidecar indexes + the LRU.
    `append_op` is `append_ops` of one, so the torn-tail heal and index-mirroring live in a single place.
  - **`Workspace::begin_batch()` / `WorkspaceBatch`** (`src/workspace/batch.rs`) is the apply-side half of the same optimization.
    A composite action (multiple `apply` calls for one user-visible mutation) opens the RAII guard and drives every op through the normal `apply` path (dedup, Yrs merge, CRDT unchanged).
    The guard buffers only the *persist* until it commits — one `append_ops` per touched storage destination.
    `WorkspaceBatch` derefs to `&mut Workspace`, so composite actions in `outl-actions` pass it straight to functions written for `&mut Workspace`.
    Batches nest via a depth counter; only the outermost guard flushes.
    A drop without `commit()` still flushes best-effort (the ops already live in the CRDT + in-memory log, so dropping them would violate invariant #1).
    The non-RAII `enter_batch`/`end_batch` pair exists for callers that can't hold a `&mut Workspace` borrow across the batched region.
    `outl-cli`'s `outl batch` is that caller — it re-borrows `&mut WsCtx` per op — and every `enter_batch` must be paired with exactly one `end_batch`.
    The buffer only ever carries local-actor ops; a foreign op (sync replay) never flows through a batch.
  - **Read-side glued-op recovery.**
    `JsonlStorage::reload` parses each line with a streaming `serde_json::Deserializer`.
    A line carrying concatenated JSON objects with no separating newline (`…}}}{"ts":…` — the signature of an interleaved, non-atomic concurrent append) is recovered into all its ops instead of dropped.
    This is a read-side safety net only; writers must still serialize their appends (the corruption was produced by an unsynchronized `outl-sync-iroh` write).
    Dedup-by-op-id makes re-reading a recovered op harmless.
    See `docs/storage.md` → Concurrency / Failure modes.
  - **Lazy, index-driven read path (RFC #137 Front A).**
    `reload` (boot) streams each line with a **parse-lite** pass that extracts only `(ts, node)` per op — it never deserializes `Op::Edit`'s `text_op` bytes — builds the offset + node indexes, and leaves the LRU **empty**.
    Reparsing/re-allocating the whole log into the cache on every open is exactly what made boot O(log size) and froze the mobile app; the index plus a small snapshot delta is all boot needs.
    The full ops are read back lazily via the offset index (`read_op_at`, one `seek` + one-line parse), preferring a warm LRU hit.
    Every `Storage` read method is driven off the index — `all_ops`, `ops_since`, `ops_since_per_actor`, `ops_for_actor`, `ops_for_node`, `last_ts_per_actor`.
    Each returns the **complete** op set in HLC order regardless of what the LRU holds — the LRU is a RAM bound, the index is the logical view.
    `MemoryStorage` is unchanged (no disk, no index).
    See `docs/storage.md` → "Boot reads an index, not the whole log".
  - **The persisted index is a dotfile, kept off the sync surface (RFC #137 Front B).**
    The offset + node indexes are saved next to each op log as `.ops-<actor>.idx` / `.ops-<actor>.nodes.idx` so the next boot loads them instead of reparsing the whole log.
    They are **dot-prefixed on purpose**: a purely local boot cache (every device rebuilds it from its own `.jsonl`) must NOT ride the file-sync surface.
    iCloud drops `.`-prefixed paths across devices and iroh never ships sidecars, so the index stays local.
    The freshness check trusts the sidecar's prefix `[0, max_offset)` after validating the tail byte-exactly.
    A *synced* index could arrive torn-in-the-middle with an intact tail, pass that check, and feed a wrong offset into `read_op_at` → a silently dropped op on the index-driven reads.
    Keeping it local removes that vector, leaving only universal local bit-rot, which is recoverable (a bad parse → rebuild, or the next full replay).
    A stale/missing/corrupt index is always safe: it triggers a full rebuild from the `.jsonl`.
    See `ActorIndex::sidecar_path` and `docs/storage.md` → "Boot reads an index, not the whole log".
- Domain models: `Workspace`, `Page`, `Journal`, `Block`, `Property`, `Tag`
- `Tree::nodes_with_property(key)` — the transpose of `properties_of`: every node carrying a key, without a tree walk.
  `O(total properties)` and touches no block text, so an index-like reader (the `remind::` scan) can find its handful of carriers without forcing a lazy-boot vault to materialize.
- Materialized-state **snapshot** boot cache (`snapshot.rs`): a projection of the tree + block text that short-circuits full op-log replay on boot (#109/#128).
  It is **not** a `Storage` responsibility — the snapshot is a *local* cache and is written straight to `<root>/.outl/snapshots/snap-<actor>.bin` (never on the file-sync surface, never through the op log).
  `Workspace` is the single owner: it reads via `snapshot::read_from_disk` on boot and writes via `snapshot::write_to_disk` (both the synchronous `save_snapshot` and the background threshold writer).
  The op log stays the source of truth — a missing / stale / corrupt snapshot is silently ignored and boot falls back to a full replay, so the snapshot can never corrupt state.
  `<root>/ops/` (the op log) is deliberately **not** a dotfile because it must sync; `<root>/.outl/snapshots/` deliberately **is**, because it must not.
  The replay cutoff is a **per-actor vector clock** (`SnapshotBody.cutoff: BTreeMap<ActorId, Hlc>`), never a single global HLC — boot replays, per actor, every op above that actor's mark plus every op of an actor the snapshot never saw.
  A single global cutoff silently drops a low-HLC op from a lagging peer delivered after the snapshot (#156 Half 2); the delta comes from `Storage::ops_since_per_actor`.
  The body is **postcard**-encoded at `SCHEMA_VERSION = 4` (bincode through schema 3) — see below.

### This crate's dependency graph is public surface

The snapshot encoder is postcard since schema 4 because bincode is unmaintained under RUSTSEC-2025-0141, not because postcard is nicer (#207 — the *why* lives in [`docs/storage.md`](../../docs/storage.md#wire-format-postcard-schema-4)).

The rule that outlives that migration: **`outl-core` is published for embedding, so a dep that trips a downstream `cargo deny` blocks adoption as hard as a missing feature.**
A new direct dep here needs a maintained upstream and a permissive license.
Test-only helpers go in `[dev-dependencies]`, never `[dependencies]`.

Two consequences for anyone touching `snapshot.rs`:

- **`SCHEMA_VERSION` and the encoder move together, and `decode` compares `!=`.**
  Bump one without the other and an old snapshot gets half-parsed as if it were current.
  `fixtures/legacy-snapshot-schema3.bin` — a real captured pre-#207 file, not a synthetic corruption — is what pins this.
- **A format break here never needs a converter.**
  An unreadable snapshot falls back to full op-log replay, so the worst case of *any* format change is one slower boot.
- **`compute_hash` and `from_parts` return `Result`, and that is load-bearing.**
  Degrading an encode failure to a default would hash the empty vector on both the write and the verify side, and `sha256([])` compares equal to `sha256([])` — the integrity check would keep passing while checking nothing.
  The error surfaces as `WorkspaceError::Snapshot`; both snapshot writers warn and skip the cache, which costs a replay and never the workspace.

### Actor id is device-local, and the workspace cannot hold it

"One `ops-<actor>.jsonl` per device, never shared" is what makes last-write-wins-per-file harmless on every file transport.
That invariant is only as strong as where the actor id is stored.

It used to live at `<root>/.outl/config.toml` — **inside** the directory the user syncs.
Syncthing, Dropbox, NFS, a shared network volume and `git clone` all replicate `.outl/`, so both devices read the same `actor_id`.
`ActorWriteLock` does not catch it: `flock(2)` is advisory and machine-local, so each device acquires its lock successfully and both append to one file.
Last write wins, ops vanish, nothing errors.
The only reason this was not a daily disaster is that iCloud Documents drops dot-prefixed paths, so `.outl/` never travelled — an accident of one transport.

`device/` moves the answer outside the workspace:

- `DeviceStore::actor_for_instance(&WorkspaceId, root, fallback)` → `<device_dir>/actors/<workspace-id>`.
  Keyed by `WorkspaceId` because that is the id two paired devices *agree* on, which is what makes the actor they disagree on well-defined — **and** by the workspace directory, because the id lives at `<root>/.outl/workspace-id` and therefore travels inside a `cp -R`.
  Two copies of one directory keyed on the id alone share an actor, and iroh keys its gossip topic on that same id, so the copies reconcile as one workspace and dedup each other's genuinely-distinct ops by `ts`.
  A binding records the root it was made for; a mismatching root forks a second actor unless the recorded one is *provably gone* (a move or rename), and anything unreadable counts as still live.
- `DeviceStore::device_actor()` → `<device_dir>/actor`, the single device-wide actor the Tauri clients have always used (their `HlcGenerator` is bound at app start, before a workspace exists).
  Device-local already, so it never had the *cross-device* bug; it lives here so both GUI clients read one implementation.
- `DeviceStore::machine_id()` → `<device_dir>/machine-id`, a device fingerprint that *may* be published into the shared config, because it is only ever compared against the local value.
  Bound to a hash of an OS identifier of the physical machine (`/etc/machine-id`, `IOPlatformUUID`, `MachineGuid`) and **reminted when that changes**, because `$HOME` is replicated by Migration Assistant, Time Machine, VM images, chezmoi and NFS.
  A remint invalidates every actor binding stamped with the old id, so each workspace forks.
  Platforms exposing no such identifier (iOS above all) are inconclusive and change nothing — a documented gap, not a silent one.

Every device-store file is `key=value` lines composed in a sibling temp file and published in one step, and a create is a compare-and-swap, so two processes racing a first open converge instead of minting two ids.
The publish step is `link(2)`, not an `O_EXCL` open, because an `O_EXCL` open creates the file **empty** and fills it a moment later.
A reader landing in that window parses a blank record as *absent* — exactly the answer that licenses it to overwrite the winner.
`machine_id` mints under that same compare-and-swap, and it matters more there than for an actor binding.
A lost actor costs one extra ops file; a lost machine id invalidates every binding **and** every `actor_claimed_by` claim already written into a workspace config, so those workspaces never adopt their own legacy ops file again.
A bare legacy line (the Tauri clients' plain-ULID `actor` file) still parses.

`device_dir()` honours `$OUTL_DEVICE_DIR` before the XDG layout.
That override is what keeps the test suite (and any container) off the developer's real store — the repo's `.cargo/config.toml` points every cargo-spawned process at `.dev-device-store`.
That path is deliberately **not** under `target/`, which `cargo clean` erases along with the iroh identity key that is this device's node id.
`the_test_suite_runs_against_an_isolated_device_store` fails outright when that file is missing, because a suite that silently writes into `~/.config/outl/` is how 64 entries got there, 15 of them pointing at `TempDir` paths that no longer exist.

#### The store has a GC now, and its whole design is what it refuses

`device/gc.rs` answers invariant 9's fourth question for this store: *what cleans it up?*
Until it existed, nothing did — `actors/` gained one record per workspace this device ever opened and lost none, so a workspace the user deleted kept its binding forever (1,208 records on a dev machine, 1,166 orphaned).

**Dropping a binding is not free**, and that asymmetry is the entire design.
The next open of that workspace mints a *fresh* actor — a second `ops-<actor>.jsonl` for a device that already had one, with every op it previously wrote no longer attributed to it.
That is the fork this store exists to prevent, so a GC that guesses wrong causes the bug it is tidying up after.
Keeping a stale record costs ~190 bytes.

So "the root is missing" is **not** the rule, because an unplugged drive, an unmounted network volume, an undownloaded iCloud folder and an archived workspace all look exactly like a deleted one.
A binding is `BindingVerdict::Stale` only when the root is gone, its *parent* directory is still present, and the record is older than `STALE_BINDING_TTL` (30 days).
The parent check is what does the real work: a deleted folder leaves its parent behind, while a missing mount takes the whole path with it.
A workspace that is *itself* a mount point would defeat the parent check alone (unmounting `/Volumes/Notes` leaves `/Volumes` behind), so each binding also stamps the root's filesystem device id (`dev=`) while the root is there to ask, and a surviving parent on a different filesystem keeps the entry.
Everything else — including a record with no `root=`, and any path we failed to *read* rather than observed to be absent — is `Inconclusive`, which always keeps it.

Two things about that rule are easy to misread, and both are pinned by tests:

- **The TTL is the record's age, not time since the deletion.**
  A binding is written on first open and rewritten only when its workspace *moves*, so nothing records when a directory went away.
  A workspace bound years ago and deleted a minute ago is `Stale` immediately (`an_old_binding_whose_workspace_just_vanished_is_stale`).
  Buying the stronger reading means stamping `seen=` on every open, which turns the common read path into a write on a store that may be read-only — a trade not made here.
- **A record that does not survive the parse is not evidence.**
  `write_record` does not escape and `parse` trims, so a root ending in a space (or holding a newline) reads back as a *different*, non-existent path whose parent exists — the exact shape that authorises a delete, for a workspace that is alive.
  `Record::is_lossy` reports the failed round trip and `judge` drops the root rather than trusting it.
  The same defect exists one layer earlier: the writer serializes the root via `Path::display()`, which replaces non-Unicode path data with U+FFFD before the parser ever sees the text, so `judge` also drops a root carrying the replacement character.
  Before the GC that leniency cost one redundant rewrite per open; the GC is what changed its price.

`gc.rs` is the **single owner** of that verdict.
`DeviceStore::prune_binding` re-asks it immediately before deleting, because listing and pruning are two passes with a user in between, and a workspace can come back in that gap.
It also refuses any path outside `actors/`: `iroh/identity.key` **is** this device's node id.

The same module also collects **abandoned scratch files** (`STALE_SCRATCH_TTL`, 24h).
`record.rs` composes every write in a `.<name>.<pid>.<seq>` sibling and removes it after publishing, so a killed process leaves one behind forever.
They stay out of the binding listing on purpose: a scratch file names no workspace, so reporting one as "a binding whose workspace is gone" invents a graph that never existed.
They are also never backed up, because a half-published write is by definition content that never became a record.
Deleting one a live writer still holds is survivable on both publish paths — `create_new_record`'s `hard_link` fails with something other than `AlreadyExists` and falls through to `exclusive_create`, and `write_record` recomposes a scratch its `rename` found missing and publishes again.

The surface is `outl doctor` (reports the count) and `outl doctor --repair` (drops them, after a backup) — see `outl-cli/CLAUDE.md`.
`scripts/gc-dev-device-store.sh` is the *developer's* faster sweep of `.dev-device-store`, and it now differs in **exactly one** way: no TTL, because test debris does not deserve a 30-day wait.
It carries the parent check, which is the condition that actually protects a live workspace.
It used to skip that too, deleting on `[ -d "$root" ]` alone — the rule this module rejects — and since it reads `$OUTL_DEVICE_DIR`, pointing that at a real store applied the rejected rule to real bindings.
If the two ever diverge again, align the **script** to `gc.rs`, never the reverse.

**Migration lives in `outl_ws::actor`, not here**, because it needs `config.toml`.
`config.toml`'s `actor_id` is a legacy value adopted only by the device named in `[workspace] actor_claimed_by`, and that marker is stamped when the config is **created**, never on first open — the default transport (iroh) never ships `config.toml`, so a claim written at open time propagates to nobody.
A workspace with no claim is adopted by nobody: every device forks once and the old ops file stays readable.
Read that module before changing anything about actor resolution.

Rules that follow:

- ❌ Never re-derive the write actor from anything under `<root>/`.
  A value two devices can read identically is not a device identity.
- ❌ Never put the machine id, or any other device-local value, on the sync surface.
  It is the opposite of invariant #7: state that must **diverge** per device must never travel.

### Snapshot dir has exactly one owner — the `Workspace`, keyed off `root`

> Why boot needs the snapshot, the offset index and the lazy `Doc` together: [RFC 0128](../../docs/rfcs/0128-boot-and-memory-at-scale.md).

The snapshot directory is derived **only** from the workspace `root` (`<root>/.outl/snapshots`), never from the storage's `ops_dir`.
This was a real bug (#156): `JsonlStorage` used to derive its own `ops_dir.parent()/snapshots`.
But production passes `ops_dir = <root>/ops` (not `<root>/.outl/ops`), so the storage read `<root>/snapshots` while the background writer wrote `<root>/.outl/snapshots`.
They never met: snapshot boot was inert in production, while every test (which used `<root>/.outl/ops`) passed.
The fix removed snapshot I/O from the `Storage` trait entirely: storage owns the op log, the workspace owns the snapshot cache, and there is now a single path derivation.
Never re-add `save_snapshot` / `load_snapshot` to `Storage` — that reintroduces the two-owners divergence.

### Block text is two-tier, not one live `Doc` per block

> Why RSS had to become constant before boot did, with the measurements: [RFC 0137](../../docs/rfcs/0137-storage-scale.md).

`Workspace`'s `ContentStore` does **not** keep a live Yrs `Doc` resident for every block.
That was the cause of issue #108: a vault in the hundreds-of-thousands-of-blocks range held 0.5-1GB of resident docs and iOS jetsam killed the app on open.

Instead it keeps two tiers, both reconstructed on open from the op log:

- `text: RefCell<HashMap<NodeId, String>>` — the materialized string of a block.
  The hot read path behind `Workspace::block_text`.
  Cheap, roughly the text size.
  **Lazily populated** on the full-replay boot path (see below); `RefCell` so the `&self` read accessor can cache a rebuilt string without forcing `&mut self` on its ~150 call sites.
- `cache: DocCache` — a bounded LRU (`DOC_CACHE_CAP = 512`) of live `Doc`s, only for blocks being edited or merged right now.
  A cold block is rebuilt on demand via `ContentStore::ensure_doc` (private, in `src/content.rs`), which replays that block's `Edit` ops from the log into a fresh `Doc`.
  Yrs is a CRDT, so update order does not change the result — convergence is preserved.

`open_with_storage` replays in **two passes**.
Pass 1 applies every op to the tree/log (`Edit` is a no-op on the tree) — the tree comes out **fully materialized**.
Pass 2 does **not** materialize block text eagerly (that O(all blocks) pass was a major boot freeze on large snapshotless vaults, #179).
It records which nodes carry `Edit` history in a `pending` set, and `block_text` rebuilds each block's string lazily from the (complete) in-memory log on first read.
A block never touched since boot reads back byte-identical to the old eager pass; a never-edited node reads back as `None`.
The snapshot boot path is unchanged — it hydrates the full text map up front (already materialized strings, not a replay) and leaves `pending` empty.
The snapshot **writer** (`build_snapshot_body`) force-materializes any still-deferred block first, so a snapshot always carries every block's string.

`Workspace::resident_text_count()` (thin wrapper over `ContentStore::resident_text_count`) is `pub` — the observability window into this lazy path.
It reports how many block strings are currently materialized, so a downstream crate's regression test can assert a read path (e.g. `outl-actions`' backlinks index build) does **not** force the whole workspace to materialize.
Cheap (a map length); safe to call in production, not gated behind `#[cfg(test)]`.

This is a materialization change only: the op log stays the source of truth, the `Doc`/string are projections, and the public surface (`block_text`, `build_text_replace_update`, `apply`) is unchanged.
`Workspace` is only ever reached through `Arc<Mutex<..>>`, so it needs `Send` (which `RefCell<T: Send>` keeps) but never `Sync`.
The resident `OpLog` still holds every `Op::Edit`'s `text_op` bytes (the cheaper second copy of history); shrinking that is the separate per-page op-log shards work, not this change.

### `Workspace::block_text_history` — the past, not just the present

`block_text` answers "what does this block say now"; `block_text_history` (`src/workspace/text_history.rs`) answers "what did it say before", replaying a block's `Op::Edit`s in order into every intermediate string.
`Op::Edit` carries a Yrs delta, not a snapshot, and the log is append-only — so an edit that *shrank* a block did not erase what it replaced, only the materialized tree stopped showing it.
Reads from **storage**, never the resident log or text cache — both are boot-mode dependent (a snapshot boot's resident log holds only the post-cutoff delta).
A caller asking "was anything lost here" getting a silently shortened history back is the one wrong answer to give it.
`outl_actions::recover` is the consumer: it scans for a block whose current text is a proper prefix of an earlier entry — the signature a truncating `Op::Edit` leaves — and restores it as a **new** edit.

## What this crate does NOT own

- Markdown parsing/rendering → `outl-md`
- Sidecar `.outl` JSON → `outl-md`
- CLI / TUI → `outl-cli`, `outl-tui`
- Network sync → `outl-sync-iroh` (P2P via iroh, default transport; file/iCloud opt-in)

If you find yourself reaching for `comrak`, `ratatui`, `iroh`, or anything file-format related: **stop**.
You're in the wrong crate.

## The five invariants

This crate exists to maintain these.
They are properties of the algorithm proven in Kleppmann et al. 2022.

1. **Convergence (SEC).**
   All replicas applying the same set of ops in any order produce the same materialized tree.
2. **Commutativity after reordering.** `apply(a, b, c)` == any permutation.
3. **Idempotency.** `apply(op); apply(op)` == `apply(op)`.
4. **Tree invariant.**
   Materialized state is always a valid tree.
5. **No silent loss.**
   Every op stays in the log, even ones turned into no-ops by cycle detection.
   This extends to the **read** side: a damaged log may cost you the damaged bytes, never the healthy bytes after them, and never quietly.
   **Every sequential pass over a `.jsonl` skips an unreadable record and continues** — a `break` there discards every op past the damage and boots a truncated tree as if it were the whole workspace.
   That is `read_ops_file_into` (the full replay) *and* `rebuild_actor_indexes` / `index_stream` (the index build).
   The index build is the one that hides best: a short index never *knows* about the ops past the damage, so `MissingOp` below can never fire for them and the tree comes out short with no error anywhere.
   For the same reason a rebuild that hit a read error refuses to persist its `.idx` sidecars — caching a known-incomplete index is what turns a recoverable omission into a permanent one.
   All four index-driven reads return `StorageError::MissingOp` when the index lists an op the file won't return, rather than a short result set.
   Those four are `ops_since`, `ops_for_actor`, `ops_since_per_actor` and `ops_for_node`; snapshot boot falls back to a full replay on the error.
   A short read there is the worst case in the crate: `build_snapshot_body` derives the next cutoff from the **index**, so an omitted op gets recorded as already-folded-in and no later boot replays it again.
   `ops_for_node` is the sharpest of the four, because its result is replayed into a fresh Yrs `Doc` — a short read there doesn't shorten a list anyone inspects, it produces **wrong block text** (#129).
   Pinned by `tests/op_log_truncation.rs` and `src/storage/jsonl/read_robustness.rs`.
   Full reasoning: [RFC 0129](../../docs/rfcs/0129-op-log-durability.md).

## Op log is the only sync surface

Any per-block (or per-page) state that must converge between devices — fold flags, pinned status, whatever ships next — lands as an `Op` variant on this enum.
Never as a field of `SidecarBlock`, a key in a shared JSON file, or anything else that depends on iCloud / Syncthing to merge file contents.
Those transports are last-write-wins per file and lose concurrent writes silently.

`Op::SetCollapsed` is the canonical example; `Op::SnoozeRemind` (silence a block's `remind::` rule until a wall-clock instant) is the second, and follows the same anatomy with a `HashMap<NodeId, u64>` side table instead of a `HashSet`.
Its `until_ms` is Unix epoch **milliseconds**, deliberately not an `Hlc`: the envelope's `ts` already carries the ordering, and conflating the two would make a clock-skewed device's snooze resolve to the wrong wall time.
Anatomy of a new "per-block UI state that needs to sync" Op:

- A variant with `node`, the desired value, and an `old_*` field.
- `do_op` captures the old value and applies the new one to a side table (`HashMap` / `HashSet`) inside `Tree`.
- `undo_op` restores the captured `old_*`.
- A read accessor on `Tree` (e.g.
  `is_collapsed(node) -> bool`).
- Storage `op_touches_node` covers the new variant.

Anything cheaper than this in the design discussion is wrong — correctness across devices is not optional.

The test battery in `tests/` is the operational expression of these.
If you change `tree.rs`, every one of those tests must still pass.

## Algorithm reference

The paper: **Kleppmann, Mulligan, Gomes, Beresford. "A highly-available move operation for replicated trees.
IEEE TPDS 2022.** <https://martin.kleppmann.com/papers/move-op.pdf>

OCaml reference implementation by the authors: <https://github.com/martinkl/crdt-tree-move>

Core algorithm sketch:

```
apply_op(new_op):
    if new_op.ts > log.last().ts:
        do_op(new_op)
        log.append(new_op)
    else:
        undone = []
        while not log.empty() and log.last().ts > new_op.ts:
            op = log.pop()
            undo_op(op)
            undone.push(op)

        do_op(new_op)
        log.append(new_op)

        for op in undone.reverse():
            do_op(op)
            log.append(op)
```

`do_op` for `Op::Move`:

```
do_op(op):
    if op is Move:
        old_parent = tree.parent(op.node)  // preserved on the LogOp for undo
        old_position = tree.position(op.node)
        if creates_cycle(op.node, op.new_parent):
            // NO-OP on the materialized tree
            // but the LogOp goes into the log unchanged
            return
        tree.set_parent(op.node, op.new_parent, op.position)
```

`creates_cycle(node, new_parent)`:

```
n == new_parent OR new_parent is descendant of n (recursive)
```

Always walk to root or until cycle confirmed.
**A non-transitive cycle check is wrong** and will fail `cycle_chain.rs`.

## Files

```
src/
├── lib.rs              # public API surface
├── id.rs               # NodeId, ActorId (ULID wrappers)
├── workspace_id.rs     # WorkspaceId — stable shared workspace identity (.outl/workspace-id)
├── device/
│   ├── mod.rs          # DeviceStore, MachineId, device_dir — device-local actor, OUTSIDE the workspace
│   ├── host.rs         # host fingerprint (detects a cloned device store)
│   └── record.rs       # key=value device-store files (atomic write, O_EXCL bind)
├── hlc.rs              # HLC timestamps (uhlc wrapper)
├── op.rs               # Op enum, LogOp envelope, serde
├── fractional.rs       # Fractional indexing (position between siblings)
├── tree.rs             # THE algorithm — do_op, undo_op, apply_op, creates_cycle
├── log.rs              # OpLog (append-only, ordered by HLC)
├── storage/
│   ├── mod.rs          # trait Storage
│   ├── jsonl/          # JsonlStorage (only persistent backend)
│   │   ├── mod.rs      # the struct, its ctors, the Storage impl
│   │   ├── append.rs   # write path (batch append, torn-tail heal, index mirroring)
│   │   ├── read.rs     # read path (reload, index build, cold reads)
│   │   └── read_robustness.rs  # tests: what a damaged .jsonl may and may not cost
│   └── memory.rs       # MemoryStorage (test double, no disk)
├── workspace.rs        # Workspace entry point
├── workspace/
│   ├── batch.rs         # Workspace::begin_batch / WorkspaceBatch (deferred-persist batching)
│   └── text_history.rs  # Workspace::block_text_history — replay a block's past text from storage
├── page.rs             # Page model (projection over op log)
├── journal.rs          # Journal (page with date-key)
├── block.rs            # Block (tree node, with Yrs TextRef for content)
├── property.rs         # Property (key-value on block or page)
└── tag.rs              # Tag (page reference with classification semantics)

tests/
├── convergence.rs           # 3 replicas, random ops in different orders
├── cycle.rs                 # classic A↔B move cycle
├── cycle_chain.rs           # A→B→C with concurrent C→A
├── concurrent_edit_move.rs  # block edited and moved simultaneously
├── concurrent_delete_edit.rs# delete wins, edit registered
├── late_op.rs               # old-ts op forces reorder
├── idempotency.rs           # apply N times == apply 1 time
├── fractional_index.rs      # concurrent inserts in same gap
├── large_log.rs             # 10k ops stress test
├── property_based.rs        # proptest: SEC for Create+Move, fwd-vs-reversed
└── convergence_property.rs  # proptest: full-op-mix convergence suite (below)
```

## Convergence property suite (`tests/convergence_property.rs`)

The definitive guard for the SEC claim.
It generates bounded random op programs across up to 4 actors with globally-unique, monotonic-per-actor HLCs.
The op mix is `Create` / `Move` / delete=`Move`→trash / `SetProp` / `SetCollapsed`.
It delivers them to multiple replicas under random permutations and random duplication.
Every op carries a unique HLC so the idempotency dedup never silently drops two distinct ops.
The comparison is a `BTree`-keyed snapshot of the **full** materialized state: node parent+position, every property binding, and the collapsed set.
That is stronger than `common::assert_trees_equal`, which compares nodes only.
It is deterministic (no wall clock; permutations driven by seeded xorshift) and shrinks to a minimal counterexample on failure.

Properties and the invariants (above) they guard:

1. `convergence_under_reordering` — SEC + commutativity under any permutation, not just reverse.
2. `idempotent_under_duplication` — idempotency: 1–3× redelivery == once.
3. `concurrent_moves_never_cycle` — tree invariant + no silent loss.
   Concurrent cycle-forming moves never materialize a cycle, the no-op move still lives in every replica's log, and all replicas converge.
4. `hlc_actor_tiebreak_is_deterministic` — equal physical+logical, different actor resolves to the same winner on every replica.
5. `late_op_undo_redo_round_trips` — the `undo_op`→`do_op` reorder path is a faithful round-trip (a late op forces a full undo/redo of the log).

### Regression: `Op::Create` honors the cycle guard

`Op::Create` runs `creates_cycle` before inserting, exactly like `Op::Move`.
This was a real bug the convergence suite surfaced.
The `Op::Create` branch used to do a bare `entry().or_insert((parent, pos))` with no cycle check.
So a `Create(node, parent)` whose `parent` was already a descendant of `node` inserted `node → parent` and closed a loop (a prior `Move` re-parents something under `node` under reordering).
That violates invariant #4 and then panics `creates_cycle` on the malformed tree.
A cycle-forming `Create` is now a no-op on the materialized tree (the op still goes into the log).
Undo is safe because a node only ever comes into existence through its own `Create` (`Move` never inserts a new entry), so a cycle-skipped `Create` leaves `node` absent and `undo_op`'s `remove(node)` is a no-op.
The deterministic regression is `create_respects_cycle_guard` (asserts no cycle, C stays unmaterialized, all ops logged, across every delivery order); the full-surface `convergence_under_reordering` property exercises it under random programs.

## Coverage targets

- **Crate overall:** > 90%
- **`tree::do_op`, `tree::undo_op`, `tree::apply_op`, `tree::creates_cycle`: 100%** (no exceptions)

Use `/coverage outl-core` to check.

## Things to never do here

- ❌ Take a dependency on `outl-md`, `outl-cli`, `outl-tui`, or `iroh`
- ❌ Bring back SQLite, rusqlite, or any binary store.
  `JsonlStorage` is the only persistent backend; cross-device sync depends on per-actor files that iCloud / Syncthing can merge.
- ❌ Add an `Op` variant without `old_*` fields (undo will be impossible)
- ❌ Skip the cycle check in `do_op` for `Move`
- ❌ Remove an op from the log because it was a no-op (silent loss)
- ❌ Compare HLCs without including actor as tiebreak
- ❌ Use `unwrap()` outside of tests
- ❌ Use `unsafe` without a multi-line comment documenting invariants

## Reuse-first

This crate is the **foundation**: every other crate consumes its types.
Before adding a new primitive (a `Tree` accessor, an `Op` variant, an `id` helper), grep for an existing one — even partial matches are worth wrapping rather than duplicating.
`Tree` accessors in particular cluster around the same `HashMap` — prefer one more `properties_of`-style method over two callers each filtering the map by hand.

Root [`CLAUDE.md`](../../CLAUDE.md#reuse-first) has the workspace-level policy.

## When you're adding a new Op variant

Use the `/new-op <Name>` slash command.
It walks through all 7 places that need to change.

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-core -- -D warnings`
3. `cargo test -p outl-core`
4. `/coverage outl-core` — must show 100% on the four critical functions
5. Invoke `crdt-invariant-checker` agent
6. If you touched `do_op`/`undo_op`/`apply_op`/`creates_cycle`: invoke `paper-verifier`

Only then is the change ready.
