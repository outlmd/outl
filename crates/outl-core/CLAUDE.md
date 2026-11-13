# CLAUDE.md — outl-core

The kernel.
Tree CRDT, op log, storage trait.
Nothing else.

If you break this crate, you corrupt the user's tree on sync.
**There is no second chance** to win back trust if that happens.
Treat every change as production-bound.

## What this crate owns

- `Op` enum and `LogOp` envelope
- HLC timestamps (**hand-rolled**, not `uhlc`; see `docs/architecture.md`).
  `hlc::MAX_CLOCK_SKEW_MS` is the **single owner** of the 24h future-timestamp window, and `hlc::skew_ahead_ms(ts, now_ms)` is the single owner of the *verdict* built on it: `outl-sync-iroh`'s `ingest_received_ops` and `outl-plugins`' `PluginHost::sync_pull` both call it to drop an incoming op beyond the window, and `seed_clock` clamps the boot seed to the same number.
  Both ingestors call it **before** `observe`, because `observe` is monotonic — one op from the year 584 million raises this device's clock and keeps it there, every op it writes afterwards is past its peers' own gate, and the device keeps working locally while silently syncing nothing it writes.
  The plugin path shipped without the gate for exactly the reason invariant 10 warns about: when the gate was written iroh was the only ingestor, so it lived in the transport crate, and the second ingestor inherited everything except the guard on the far side of a crate boundary ([issue #283](https://github.com/outlmd/outl/issues/283)).
  A third ingestor is a third **caller**, never a third copy: a negative gate that drifts drops ops the user cannot see are missing.
  `hlc::wall_clock_ms_checked()` is public for the same two callers — they read the clock once per batch and judge each op against it, and the `Option` is load-bearing (a `0` fallback would classify every op as 55 years ahead and turn the gate into a sync black hole).
  It lives here, not in the transport, because the transport depends on this crate and not the reverse.
  Two generator properties are load-bearing and easy to undo by "simplifying": the logical counter **carries** into `physical_ms` at `u32::MAX` rather than saturating (a pinned counter makes `Workspace::apply` dedup every later local write and return `Ok(())` without persisting, which is silent total write loss), and the boot seed is **clamped** (unclamped, one bad far-future line in any actor's log is absorbed into this device's own `ops-<own>.jsonl` on first boot and pins the clock forward permanently).
  Both became reachable only when seeding landed; neither was a bug before it
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
- Fractional indexing (`fractional.rs`).
  **A tie between two siblings is representable, and normal.**
  `Fractional::between` is a pure function, so two devices that each create the first child of one parent while offline mint the *same* key; both ops are valid, both replay, and `outl_actions::tree::sort_siblings` breaks the tie by `NodeId` — a tiebreak that only exists because ties do.
  A tie therefore has no interior, and neither does `("a", "aa")`: `a..=z` is dense enough for a midpoint and never for a slot under a key whose tail is a single `a`.
  `between` used to `assert!(left < right)` on exactly those states, which killed the process the first time a user pressed `O` on a tied sibling — on **every** boot, because the tie lives in the op log and replays ([issue #282](https://github.com/outlmd/outl/issues/282)).
  What it does now: **the lower bound wins.**
  The result is always a valid position, always sorts after `left`, and sorts before `right` *iff* a key between the two exists; when the gap is empty `right` is dropped.
  It never panics, and it is still a pure function of the bounds, so two devices converge.
  A caller that needs the upper bound honoured checks it, and **every** caller that places a node with the result must: `outl_actions::tree::interior_between` is the one comparison that asks (behind `position_before` / `position_after`), and `outl_actions::block::slot` then frees a neighbour's key — but only after asking the same question about the gap *above* that neighbour, because a tie there would send it below a block the user never touched.
  Where both gaps are empty the new block shares the neighbour's key and the `NodeId` tiebreak decides the order, which is the honest answer: a tie is not separable without choosing a `NodeId`.
  `Result` was the alternative and buys the same check at all eleven call sites, most of which pass no upper bound and so can never fail.
  **`between` also never mints a key ending in the floor byte**, unless every key in the gap does.
  It used to mint `"ba"` for `("b", "bc")`, so the *next* insert after `"b"` had no interior and landed behind `"ba"` — #282's empty gap producing a wrong tree instead of a crash.
  The reasoning, and why two builds minting different keys for one gap costs nothing, live in the `between` doc comment ([issue #476](https://github.com/outlmd/outl/issues/476)).
  The mint rule stops producing new ones; **the callers' checks are not a migration** — a gap whose tail is all floor bytes still forces one on an all-new workspace, and an un-upgraded peer mints them tomorrow, not only yesterday.
  Pinned by the floor-tail half of the `tests/fractional_index.rs` proptest and by `outl-actions/tests/sibling_order.rs`, which drives the actions rather than the function.
  **`Deserialize` is a constructor and goes through `parse`.**
  The derived impl took the `String` as-is, which made the one constructor fed untrusted bytes — a line of `ops-<actor>.jsonl` off iroh, off iCloud, or half-written after a crash — the one that skipped validation, so a remote peer chose when this process aborted.
  `#[serde(try_from = "String")]` makes a malformed position one skipped record, named in the log, with every healthy op around it still replayed (invariant #5).
  Pinned by `deserialize_rejects_what_parse_rejects`, `an_op_whose_position_skipped_validation_is_skipped_not_applied` and `between_honours_every_bound_that_can_be_honoured` (the proptest in `tests/fractional_index.rs`, which checks `between` against an independently written "does this gap hold a key" predicate — sharing one would let a single bug satisfy both sides).
- The CRDT itself: `do_op`, `undo_op`, `apply_op`, `creates_cycle`
- Append-only `OpLog`
- `Storage` trait + `JsonlStorage` (one file per actor, syncable via iCloud / Syncthing / shared FS) + `MemoryStorage` (test double)
  - **Compaction (`storage/compact/`).** `Create` is idempotent, so a `Create`+`Move` pair also spells trashed-then-restored — there the `Move` *is* the work, hence six conditions ([RFC 0256](../../docs/rfcs/0256-op-log-compaction.md)).
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
  - **A question about the index is not a question for `Storage` (`storage/jsonl/census.rs`).**
    `JsonlStorage::actor_timestamps(ops_dir)` is every distinct HLC the op log holds, grouped by the actor inside it — the whole of what a sync vector clock is.
    It is an associated function **on purpose**: a `Storage` method takes `&self`, which means a storage already opened, and opening is the cost.
    `outl-sync-iroh` used to answer "what is my sync status" with `open` + `all_ops`: both index sidecars for every actor plus every `Op::Edit`'s `text_op`, read to produce twenty numbers.
    That is 52 MB + 80 MB and 342 ms (release) on a 222,536-op workspace, polled every 5s on the iOS **main thread** until the scene-update watchdog killed the app on launch.
    The index-only read is 63 ms and provably the same answer (`the_index_census_matches_an_all_ops_fold`).
    It builds **no node index** — the `HashMap<NodeId, Vec<_>>` filled one `entry()` per op, the innermost frame of that crash.
    It also **persists nothing**: advancing the offset sidecar past the node sidecar would make the next real boot reject the pair and rebuild both, so a status poll would arm a full reindex.
    `census::fresh_offset_index` is the single owner of "what does this sidecar cover".
    `try_fresh_actor_indexes` calls it and adds the node-extent check, because a second opinion about a sidecar's coverage is how a stale index becomes a wrong read.
    Pinned by `the_actor_census_neither_reads_nor_writes_the_node_index` (`tests/index_census.rs`), which deletes both sidecars and fails if either comes back — the signature of a reroute through `JsonlStorage::open`.
  - **The persisted index is a dotfile, kept off the sync surface (RFC #137 Front B).**
    The offset + node indexes are saved next to each op log as `.ops-<actor>.idx` / `.ops-<actor>.nodes.idx` so the next boot loads them instead of reparsing the whole log.
    They are **dot-prefixed on purpose**: a purely local boot cache (every device rebuilds it from its own `.jsonl`) must NOT ride the file-sync surface.
    iCloud drops `.`-prefixed paths across devices and iroh never ships sidecars, so the index stays local.
    The freshness check trusts the sidecar's prefix `[0, max_offset)` after validating the tail byte-exactly.
    A *synced* index could arrive torn-in-the-middle with an intact tail, pass that check, and feed a wrong offset into `read_op_at` → a silently dropped op on the index-driven reads.
    Keeping it local removes that vector, leaving only universal local bit-rot, which is recoverable (a bad parse → rebuild, or the next full replay).
    A stale/missing/corrupt index is always safe: it triggers a full rebuild from the `.jsonl`.
    See `ActorIndex::sidecar_path` and `docs/storage.md` → "Boot reads an index, not the whole log".
  - **The sidecars are fixed-width binary (`storage/sidecar`), not JSON Lines.**
    They were JSON, and on a real workspace that is 170 MB across 23 actors and ~1.45M records, each one a `String` allocated by `BufReader::lines`, parsed by `serde_json`, carrying ULIDs decoded from base32 text.
    An iPhone spent **14.3 s** of a launch there: 12 MB/s on hardware that reads at over a gigabyte, so the cost was never the disk.
    Records are now a fixed number of little-endian bytes behind an 8-byte magic — 170 MB → 61 MB, 310 ms → 57 ms on the same workspace.
    Changing the format needed no converter precisely because this is a cache that must already survive being unreadable: a file without the magic reads as absent and is rebuilt from the `.jsonl`, once per device.
    **`sidecar::Record` is the one place the layout lives**, and `OffsetIndex`/`NodeIndex` each implement it; a test counts records through `sidecar::test_support` rather than restating the width.
    The **offset ordering** is what replaces the parse as a corruption detector (see `load_entries`' doc), which is why `NodeIndex::save` sorts instead of emitting grouped by node.
  - **Each actor's pair loads in parallel (`storage/jsonl/index_load.rs`).**
    One actor's sidecars say nothing about another's, so the sequential loop bought only an ordering nothing reads — the results land in a map keyed by actor.
    It is worth doing because this is the most expensive phase of a launch: 23 actors, **169 MB** of sidecar (72 MB offset, 97 MB node), 450 ms of the 977 ms before today's journal could paint.
    **The ceiling is one file, not the thread count.** A single actor holds 114.7 MB of that 169 MB, so no distribution of *whole files* across lanes beats ~1.45x, and that is what it measures (450 ms → 294 ms).
    Going below it means reading **less**, not reading wider: the node index is 97 MB of the total and a snapshot boot needs it for a handful of nodes (`census.rs` already demonstrates the metadata-only read).
    `std::thread::scope`, never a thread-pool crate — a direct dependency here is public surface (see "This crate's dependency graph is public surface").
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
  - **`Workspace::log().len()` is the in-memory delta after snapshot boot, not the total.**
    `Workspace::total_ops()` reads the storage count (index-driven, no full replay), so reports and UI can show the persisted total without paying for a `Storage::all_ops()` parse.
    See issue #289 and `tests/total_ops_snapshot.rs`.
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

"One `ops-<actor>.jsonl` per device, never shared" is what makes last-write-wins-per-file harmless on every file transport, and that invariant is only as strong as where the actor id is stored.

It used to live at `<root>/.outl/config.toml` — **inside** the directory the user syncs — so Syncthing, Dropbox, NFS and `git clone` all handed two devices the same `actor_id`.
`ActorWriteLock` does not catch it: `flock(2)` is advisory and **machine-local**, so both devices take their lock successfully and append to one file.
Ops vanish, nothing errors.
The only reason it was not a daily disaster is that iCloud drops dot-prefixed paths, so `.outl/` never travelled — an accident of one transport.

`device/` moves the answer outside the workspace: `DeviceStore::actor_for_instance` (per workspace), `device_actor()` (device-wide, what the Tauri clients use), `machine_id()` (reminted when the OS identifier changes, because `$HOME` gets replicated by Migration Assistant, Time Machine and VM images).
`device/gc.rs` is the single owner of "may this binding be dropped", and its design is what it **refuses**.
`$OUTL_DEVICE_DIR` is what keeps the test suite off the developer's real store.

Rules that follow:

- ❌ Never re-derive the write actor from anything under `<root>/`.
  A value two devices can read identically is not a device identity.
- ❌ Never put the machine id, or any other device-local value, on the sync surface.
  It is the opposite of invariant #7: state that must **diverge** per device must never travel.

#### The write actor, and invariant 9's four questions

The id in `ops-<actor>.jsonl`, and the `.lock-<actor>` that gates it, are the write actor.
Issue #298 measured the pile: each one is an entry in every sync handshake and every snapshot cutoff, and nothing was reclaiming them.
[RFC 0298](../../docs/rfcs/0298-read-opens-do-not-mint-actors.md).

1. **Who can write it?**
   The process holding `ActorWriteLock` for that id.
   `resolve_write_actor` takes the device actor's lock and, when that lock is held, mints a new id and locks that.
   A read does not.
   `outl_ws::open_read` takes the shared workspace lock, reports the device actor, and calls `Workspace::disallow_local_writes` after per-page shards are registered.
   `WsCtx::commit` refuses before it mutates.
   Storage refuses an `append` that goes around it, before the `.jsonl` is created.
2. **Who can read it?**
   Every opener.
   Readers merge every `ops-*.jsonl`.
   The per-actor lock is not a read gate.
3. **How does a test get its own copy?**
   A `TempDir` workspace, and the device store redirected by `$OUTL_DEVICE_DIR` (the suite sets it in `.cargo/config.toml`; a test that wants a second device uses `DeviceStore::at`).
   Never the developer's `~/.config/outl`.
4. **What cleans it up?**
   Nothing, and that stays acceptable only because the alternatives are worse.
   Rewriting an actor id changes the HLC tiebreak, so concurrent ops that used to tie become ordered and convergence changes.
   Compaction drops inert ops.
   It does not drop identities.
   A lock file is 0 bytes, and deleting one a live process holds lets a second process append to the same jsonl.
   What bounds the *new* growth is that a read no longer mints.
   `outl doctor` reports the actor-log count and warns when this device's write locks reach `EPHEMERAL_ACTOR_WARN_AT`.
   Retiring an actor from the handshake once every peer's snapshot covers it, and bounding the writers that still mint, stay open on #298.

**Full detail — the compare-and-swap publish, why `link(2)` and not `O_EXCL`, the GC's three-condition staleness rule and every reading it refuses, the TTL and lossy-record traps, and where migration lives: [`docs/device-store.md`](../../docs/device-store.md).**

### Snapshot dir has exactly one owner — the `Workspace`, keyed off `root`

> Why boot needs the snapshot, the offset index and the lazy `Doc` together: [RFC 0128](../../docs/rfcs/0128-boot-and-memory-at-scale.md).

The snapshot directory is derived **only** from the workspace `root` (`<root>/.outl/snapshots`), never from the storage's `ops_dir`.
This was a real bug (#156): `JsonlStorage` used to derive its own `ops_dir.parent()/snapshots`.
But production passes `ops_dir = <root>/ops` (not `<root>/.outl/ops`), so the storage read `<root>/snapshots` while the background writer wrote `<root>/.outl/snapshots`.
They never met: snapshot boot was inert in production, while every test (which used `<root>/.outl/ops`) passed.
The fix removed snapshot I/O from the `Storage` trait entirely: storage owns the op log, the workspace owns the snapshot cache, and there is now a single path derivation.
Never re-add `save_snapshot` / `load_snapshot` to `Storage` — that reintroduces the two-owners divergence.

#### `Workspace::snapshot_refresh_due` is the single owner of "write one now?"

Three call sites ask it — the GUI open (`outl-tauri-shared`'s `boot`), `SyncEngine::reload_workspace`, and the TUI's shutdown flush — and each used to decide for itself with `!booted_from_snapshot()`.

That form covers only the full-replay half, so **the remedy disarmed itself the moment it worked**: a boot that *adopted* a snapshot never refreshed it.
Nothing else does either on a receive-heavy device — the in-band writer in `apply` fires only on a local mutation, and ops arriving over sync are written straight into `ops-<actor>.jsonl`.
So the delta grew without bound and every boot replayed all of it.
Measured on a real 722,943-op workspace: the adopted snapshot covered a third of the log and each boot replayed the other **488,651 ops**, which is the desktop's slow open and the iPhone's 3.45 GB jetsam kill.

The rule is two cases, and conflating them is what broke:

- **Full replay** → always write.
  There was no usable snapshot, so there is no size to compare against.
  Gating *this* on a threshold is a bug the codebase already paid for once (`SyncEngine::reload_workspace`'s comment carries it): a workspace under the threshold whose snapshot the convergence guard rejected never got a fresh cutoff and full-replayed on every later reload, forever.
- **Adopted** → write once the delta passes `threshold`, because the delta is exactly what the next boot replays.

It takes **no threshold argument**: the workspace already holds its policy (`set_snapshot_policy`), and a caller passing a second copy is a second owner of one number.
Regression net: `tests/snapshot_refresh_on_adoption.rs`, which pins both directions (an adopted snapshot with a big delta is refreshed; one with a small delta is left alone, so the fix cannot become "write on every boot").

#### The adopt slot is won on coverage, not on being ours

`read_best_from_disk` used to return this device's own snapshot the moment it decoded ("it has the tightest cutoff for its local ops").
True, and not the question: what sets the cost of a boot is how much of the log the body already holds, and the cutoff answers that **per actor**.
An actor missing from the cutoff reads as "seen none of your ops", so its whole history lands in the delta.
A phone's first-launch snapshot names one actor and shadowed a desktop's naming twenty — the delta became the entire log, the convergence guard refused it, and the boot full-replayed 722,943 ops into an iOS jetsam kill, with the snapshot it needed sitting in the same directory.
`selection_key` ranks `(actors covered, newest cutoff, filename)` now and the own body competes like any other.
The **GC** still ranks peers only (`peer_best`): a peer body the selector would never pick while our own exists is still the one it picks after an actor rotation, so collecting it would strand the rotated device.

**And it ranks on headers, not on bodies.**
`cutoff` is a snapshot's third field and a few hundred bytes; the `block_text` map behind it is essentially the whole file.
Decoding every candidate to choose one cost 124 MB of reads on a real four-snapshot directory, at launch, to pick 48 MB of it.
`snapshot::read_header_from_disk` reads a bounded prefix, `snapshot::select::adopt_order` ranks on that, and only the winner reaches `SnapshotBody::decode`.
`snapshot::select` is the single owner of the ranking for **both** the boot selector and the GC, which is what keeps the cheap read from becoming a second policy — they differ only in what they rank over, and deliberately: the collector judges bodies that decode, because its output is a delete.

Two things that are easy to undo and should not be:

- **A file whose header will not parse is ranked last, never filtered out.**
  Rubble is precisely what has to reach a decode before `drop_own_if_unusable` may delete it, and a device whose own snapshot is garbage pays for it on every boot until something proves it dead.
  Filtering those is the one way this optimization could have made a boot permanently slower.
- **One unreadable entry skips that entry, not the cache.**
  The selector used to read the own snapshot first and *return* its error, so a single corrupt local file sent the boot to a full replay with a healthy peer body in the same directory — 722,943 ops replayed to avoid adopting 48 MB that was right there.
  Candidates are tried best-first until one decodes.
  Regression net: `tests/snapshot_own_does_not_shadow_peer.rs`.

**`decode` verifies the digest against the buffer it was handed**, rather than cloning the decoded body and re-encoding it to hash.
`content_hash` is the final field and a fixed-size `[u8; 32]`, so the encoding of a body and the encoding of that body with the hash zeroed share every byte before the last 32 — the two numbers are the same one.
The old form was ~150 MB of cloned `String`s plus another 48 MB of re-encoded bytes per decode, on the launch path, to check a digest of bytes already in memory.
That equality is a property of **this field order** and of postcard's fixed-array encoding, neither of which is ours to promise: `the_buffer_hash_equals_the_body_hash` fails loudly if a schema bump reorders the fields or postcard starts length-prefixing arrays, rather than every snapshot quietly failing to verify and every boot falling back to a replay.

#### The snapshot cache has a GC, and its rule is about the reader

`snapshot/gc.rs` answers invariant 9's *what cleans it up?* — nothing did (54 MB in four files on a real workspace).
**"Drop the snapshots of actors that are gone" is not the rule**: unanswerable like [RFC 0211](../../docs/rfcs/0211-state-that-leaves-a-boundary.md)'s, *and* irrelevant, since a snapshot's only readers are `read_best_from_disk` and the iroh responder — never its actor.
Prunable is `Superseded` or `Unusable`; `Own`, `Selected` and anything we failed to *read* are kept.
It runs on the background writer's worker, never as a bulk sweep on a read (`doctor` opens read-only).
Rules and rejected dedup: [RFC 0258](../../docs/rfcs/0258-snapshot-cache-lifecycle.md).

`write_to_disk` composes in a **per-write** scratch (`snap-<actor>.bin.tmp.<ulid>`, `snapshot::scratch_path`).
Two writers for one actor are routine — a threshold crossing spawning a worker while the last is still fsyncing 13MB, a reload's second `Workspace` calling `save_snapshot` — and one shared scratch name means one shared inode, so the loser of the `rename` wrote on through a path that already pointed at the published snapshot.
A slow boot, never a lost note (invariant #1 does the work), which is why nothing reported it for so long.
`storage::sidecar::tmp_path_for` took the same failure in production.
The cost moved to the collector: an abandoned scratch is no longer recycled by the next write, so `gc::stale_tmp` is what bounds it past the in-process unlink.

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

**A snapshot boot defers its rebuilds too, against a different source.**
Adopting a snapshot hydrates text as of its cutoff and replays the delta on top, so every node the delta edited has a cached string that is present and **wrong**.
Correcting one means reading its full `Edit` history back from storage and replaying all of it through a fresh `Doc` — unchanged, and still mandatory (#129), because the resident log after a snapshot boot holds **only the delta** and rebuilding from that would drop every pre-cutoff edit.
What moved is *when*: doing it at boot made a launch pay for 2,298 nodes to paint a journal that shows about twenty, and it was the only thing on the boot path that read the per-node index at all, so it pulled 98 MB of sidecar in before the first frame.

`ContentStore` therefore keeps **two** deferred sets, and they must not be merged: `pending` resolves from the resident log, `stale` resolves from storage.
Two rules follow, and both are load-bearing:

- **`block_text` checks `stale` before the cache.**
  For these the cache is not a miss, it is a wrong hit.
- **`force_materialize_pending` drains both.**
  `build_snapshot_body` reads that same cache, so a body written after a boot where nobody happened to read the block would publish the **pre-delta** string as settled — and the next boot would adopt it with a cutoff claiming the newer edit was already folded in. No later replay corrects that. Silent content loss produced by a cache, which is the one thing a cache may never do.

Regression net: `tests/snapshot_equivalence.rs`'s `snapshot_and_delta_match_full_replay` catches a stale read, and `tests/deferred_text_settles_before_a_snapshot.rs` catches the write — nothing in the suite caught the second one until it was written.

The two boot paths live in `workspace/boot.rs`, split out because they are the one part of `Workspace` a user *waits on*, and `boot_timing` puts one `info!` line per open naming each phase.
A measurement you have to rebuild to collect is one nobody takes on the device where it matters.

`Workspace::resident_text_count()` (thin wrapper over `ContentStore::resident_text_count`) is `pub` — the observability window into this lazy path.
It reports how many block strings are currently materialized, so a downstream crate's regression test can assert a read path (e.g. `outl-actions`' backlinks index build) does **not** force the whole workspace to materialize.
Cheap (a map length); safe to call in production, not gated behind `#[cfg(test)]`.

This is a materialization change only: the op log stays the source of truth, the `Doc`/string are projections, and the public surface (`block_text`, `build_text_replace_update`, `apply`) is unchanged.
`Workspace` is only ever reached through `Arc<Mutex<..>>`, so it needs `Send` (which `RefCell<T: Send>` keeps) but never `Sync`.
The resident `OpLog` still holds every `Op::Edit`'s `text_op` bytes (the cheaper second copy of history); shrinking that is the separate per-page op-log shards work, not this change.

### `Workspace::apply` persists the op the tree recorded, not the caller's

`Tree::do_op` fills `old_parent` / `old_position` / `old_value` on the `LogOp` it is handed, so those fields are the caller's guess until it runs.
`apply` used to hand `apply_op` a **clone** and then persist the original, which left `ops-<actor>.jsonl` disagreeing with the resident log about every `Move` and every `SetProp`.
It now reads the op back out of the log by `Hlc` (`OpLog::get_by_ts`) and stores that.

**Convergence never depended on this**, which is why it survived so long: `undo_op` is the only reader of those fields, its only caller is `apply_op`, and it only ever sees ops that already went through `do_op`.
Every path into the **resident log** overwrites the stored value before it can matter.
The claim is deliberately that narrow: peer-sync ingest writes a line straight into `ops-<actor>.jsonl` without going through `apply`, so a peer's stored op carries *their* derivation until this device's next boot replay runs `do_op` over it.
What *did* depend on it is every reader of the log **as data** — a page history, `outl doctor`, a human reading the jsonl.

**The old values are still on disk and always will be.**
The log is append-only, so the fix reaches ops written after it and nothing before: the reference workspace holds 65,141 `Move` ops (of 65,703) naming `root` as the old parent, and all 14,191 of its `SetProp` ops carry a null `old_value`.
Anything reading the log as data must derive from the fields that describe the op's *own effect* — `Create.parent`, `Move.new_parent`, the value being set — never from `old_*`.
`outl_actions::timeline` is the worked example.

Pinned by `tests/stored_op_matches_the_log.rs`, which asserts the general property (storage equals the resident log for every op) rather than one field at a time, because a per-field test keeps missing the next field — `SetCollapsed` carries an `old_value` nobody has read yet.

**That property holds for in-order application only, and the fourth test pins the exception rather than hiding it.** A reorder re-derives `old_*` in the resident log via `redo_op` (paper Fig. 4 l.37-40, §3.4: the record "might have changed due to the effect of the new operation"), and an append-only log cannot follow it there — the persisted line keeps the derivation from first application. So storage and the resident log legitimately diverge on any workspace that has ever received a late op, which is every synced workspace. A `doctor` check built on the stronger reading of the test's name would fire on all of them.

### `Workspace::block_text_history` — the past, not just the present

`block_text` answers "what does this block say now"; `block_text_history` (`src/workspace/text_history.rs`) answers "what did it say before", replaying a block's `Op::Edit`s in order into every intermediate string.
`Op::Edit` carries a Yrs delta, not a snapshot, and the log is append-only — so an edit that *shrank* a block did not erase what it replaced, only the materialized tree stopped showing it.
Reads from **storage**, never the resident log or text cache — both are boot-mode dependent (a snapshot boot's resident log holds only the post-cutoff delta).
A caller asking "was anything lost here" getting a silently shortened history back is the one wrong answer to give it.
`outl_actions::recover` is one consumer: it scans for a block whose current text is a proper prefix of an earlier entry — the signature a truncating `Op::Edit` leaves — and restores it as a **new** edit.

**`block_revisions` is the owner; `block_text_history` is its text-only projection.** The revisions carry the `Hlc` and `ActorId` of the edit that produced each state, which `recover` does not need and a timeline does. Both go through the same read, so the two can never disagree about what a block's past was. `ops_for_node` is the general form underneath — every op naming a node, read from storage for the same reason. The second consumer is `outl_actions::timeline`, which turns them into a page's history.

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
   The index build hides best: a short index never *knows* about the ops past the damage, so `MissingOp` can never fire for them and the tree comes out short with no error anywhere — which is why a rebuild that hit a read error refuses to persist its sidecars.
   The four index-driven reads (`ops_since`, `ops_for_actor`, `ops_since_per_actor`, `ops_for_node`) return `StorageError::MissingOp` rather than a short result set; snapshot boot falls back to a full replay.
   A short read there is the worst case in the crate: `build_snapshot_body` derives the next cutoff from the **index**, so an omitted op gets recorded as already-folded-in and no later boot replays it again.
   `ops_for_node` is the sharpest of the four, because its result is replayed into a fresh Yrs `Doc` — a short read there doesn't shorten a list anyone inspects, it produces **wrong block text** (#129).
   Pinned by `tests/op_log_truncation.rs` and `src/storage/jsonl/read_robustness.rs`.
   Full reasoning: [RFC 0129](../../docs/rfcs/0129-op-log-durability.md).
6. **`undo_op` is the exact inverse of `do_op` for every op — including the ones `do_op` ignored.**
   That is the paper's `do_undo_op_inv`, whose only hypothesis is a well-formed tree.
   `Move` / `SetProp` / `SetCollapsed` / `SnoozeRemind` record what `do_op` found in their `old_*` fields; `Create` records it in `Tree::created_by`, because the variant has no field to carry it and the paper keeps that record **off** the transmitted op anyway (§3.2).
   A `Create` that found the node already there must undo to *nothing*, not to a delete: `NodeId::from_slug` is deterministic, so two devices opening one journal offline both emit `Create` for the same id, and the unconditional remove let a delete be replayed away — a trashed page reappearing under `root`.
   [RFC 0263](../../docs/rfcs/0263-create-is-invertible.md).

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
├── hlc.rs              # HLC timestamps (hand-rolled)
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
│   ├── router.rs        # StorageRouter — which storage owns an op, and how the shards read back as one log
│   ├── router/tests.rs  #   routing + the HLC merge rule, against MemoryStorage
│   ├── snapshot_policy.rs # SnapshotPolicy — when the boot cache is written, and the workers writing it
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

## Convergence property suite

What it generates, what it asserts, and why each generator shape exists:
[`docs/crdt.md`](../../docs/crdt.md) → "Convergence property suite".

It lives in `tests/convergence_property.rs` and is **not optional** — it is the evidence for invariant 3.

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
