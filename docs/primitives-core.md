# Shared primitives — core state, sync, and durability

Everything that owns **converged workspace state**: the op log and the mutation path into it, the materialized CRDT tree, HLC and identity.
Plus what carries and protects it — the sync engine and its transports, the cross-process locks, the `Storage` trait, and the local backup safety net.
If a primitive decides *what the workspace is*, it lives here.

Part of the **Shared primitives catalog** — the index of every part lives in [`shared-primitives.md`](shared-primitives.md).

**Before writing any helper, scan these tables first.**
Most "I need a small string transform / id helper / md coercion / tree walk" needs already have an owner here —
the cost of finding the existing one is a `grep`;
the cost of missing it shows up later as drift between two parallel implementations (the user is the one who hits the divergence).

For the reuse-first rule (why this matters, past drift incidents, what to do when a primitive doesn't exist yet), see [Contributing → Reuse-first](contributing.md#reuse-first-no-parallel-implementations).

---

## 1. Workspace lifecycle, op log, and HLC (outl-core)

| Intent | Use this | File |
|---|---|---|
| Open a workspace (in-memory for tests, on-disk JSONL for prod) | `outl_core::Workspace::open_in_memory` / `open_with_storage` | `crates/outl-core/src/workspace.rs` |
| Route an op through the log → tree (the **only** mutation path). Persists the op **as `do_op` left it**, not as the caller built it — the `old_*` fields are the caller's guess until `do_op` derives them. Ops written before this fix keep the wrong values forever (append-only), so a reader of the log as *data* derives from `Create.parent` / `Move.new_parent`, never `old_*` | `outl_core::Workspace::apply(LogOp)` | `crates/outl-core/src/workspace.rs` |
| Batch a composite action so its ops persist in one `append_ops` per destination instead of one fsync per `apply` (RAII guard, derefs to `Workspace`; commit or drop flushes) | `outl_core::Workspace::begin_batch` → `outl_core::WorkspaceBatch` | `crates/outl-core/src/workspace/batch.rs` |
| Read the materialized tree / op log from a workspace | `outl_core::Workspace::tree` / `log` / `block_text` | `crates/outl-core/src/workspace.rs` |
| Every intermediate text a block held, oldest first, one entry per `Op::Edit`, replayed from **storage** (never the resident log or the text cache, so a snapshot boot can't silently shorten it) — what makes a truncating edit's earlier text reconstructible (`outl_actions::recover`) | `outl_core::Workspace::block_text_history` | `crates/outl-core/src/workspace/text_history.rs` |
| The same revisions with the `Hlc` + `ActorId` of the edit that produced each — the owner; `block_text_history` is its text-only projection, so the two cannot disagree about a block's past | `outl_core::Workspace::block_revisions` → `outl_core::workspace::TextRevision` | `crates/outl-core/src/workspace/text_history.rs` |
| Every op naming a node, oldest first, read from **storage** (the general form of the sourcing rule above: the resident log is boot-mode dependent, so anything asking about a node's *past* must read the log on disk) | `outl_core::Workspace::ops_for_node` | `crates/outl-core/src/workspace/text_history.rs` |
| Build a Yrs text-replace update payload for an op | `outl_core::Workspace::build_text_replace_update` | `crates/outl-core/src/workspace.rs` |
| Save / boot from a materialized-state snapshot (local boot cache, workspace-owned) | `outl_core::Workspace::save_snapshot` / `set_snapshot_policy` / `wait_for_snapshots` | `crates/outl-core/src/workspace.rs` |
| Read / write the raw snapshot body on disk (`<root>/.outl/snapshots/snap-<actor>.bin` — NOT a `Storage` method) | `outl_core::snapshot::read_from_disk` / `read_best_from_disk` (adopt a peer's snapshot when this device has none — Phase 2; local ops preserved via the per-actor delta) / `write_to_disk` (`SnapshotBody`) | `crates/outl-core/src/snapshot.rs` |
| Snapshot wire-format version — bump it in lockstep with any `SnapshotBody` or encoder change; `decode` rejects every version but this one, older and newer alike, and the caller falls back to full op-log replay (postcard since `4`, bincode through `3` — #207) | `outl_core::snapshot::SCHEMA_VERSION` | `crates/outl-core/src/snapshot.rs` |
| **Which snapshots the boot cache may drop, and which it may not** — the single owner of that verdict. Asks *"can the selector ever choose this file again"*, never *"is this actor gone"*: an unpulled peer log and a dead actor are one observation (RFC 0211's trap), and a snapshot is never read by its own actor anyway | `outl_core::snapshot::gc::survey` → `outl_core::snapshot::gc::Survey` / `SnapshotEntry` / `SnapshotVerdict` | `crates/outl-core/src/snapshot/gc.rs` |
| Drop what the survey proved droppable (`Superseded` — decodes, not ours, loses under the selector's own key; `Unusable` — read end to end and `decode` refused). Re-stats before unlinking, so a body a peer pull `rename`d in is refused | `outl_core::snapshot::gc::prune` / `prune_all` / `sweep` | `crates/outl-core/src/snapshot/gc.rs` |
| Abandoned `snap-*.bin.tmp` past the TTL — a write that died mid-rename, not a cache the selector can reach | `outl_core::snapshot::gc::stale_tmp` / `prune_tmp` / `STALE_TMP_TTL` | `crates/outl-core/src/snapshot/gc.rs` |
| Generate HLC timestamps with actor tiebreak (required for every op) | `outl_core::HlcGenerator::new` / `next` / `observe` | `crates/outl-core/src/hlc.rs` |
| Raise a generator to at least a given `Hlc` **without** emitting one. `new` starts at `physical_ms: 0` and `next` is monotonic only against that in-memory state, so after a backwards wall-clock move a fresh generator issues timestamps *below* ops already on disk. Not a convergence bug (`apply_op` reorders to the same tree) — a **cost** one: ~74 ms per late op on a 217k-op log, on the foreground thread. `observe` is exactly this plus `next`, sharing one `raise_to` so the two cannot drift | `outl_core::HlcGenerator::seed` | `crates/outl-core/src/hlc.rs` |
| Seed a generator from everything a workspace has already written — the one-call form, and **the API every open site should use**. Derived from the per-actor maxima the snapshot cutoff uses, never `log.last()`: after a snapshot boot the resident log holds only the post-cutoff delta and would seed the clock *too low*, which is the very failure being prevented | `outl_core::Workspace::seed_clock` / `max_known_hlc` | `crates/outl-core/src/workspace/clock.rs` |
| Wrap an `Op` into a `LogOp` (timestamp + actor) for `apply` | `outl_core::Op` + `outl_core::LogOp` | `crates/outl-core/src/op.rs` |
| Extract the `NodeId` an op targets | `outl_core::op::op_node(&Op) -> Option<NodeId>` | `crates/outl-core/src/op.rs` |
| The op with a given `Hlc`, out of the resident log (O(log n)) — how `apply` reads back what `do_op` recorded | `outl_core::OpLog::get_by_ts` | `crates/outl-core/src/log.rs` |
| Sentinel node ids (`root`, `trash`) | `outl_core::NodeId::root()` / `trash()` | `crates/outl-core/src/id.rs` |
| Per-device identity for ops | `outl_core::ActorId` | `crates/outl-core/src/id.rs` |
| Stable, shared workspace identity (read/generate, persist, pairing-adoption) — the gossip-topic key, NOT the path | `outl_core::WorkspaceId::read_or_create` / `write` / `from_raw` (errors: `outl_core::WorkspaceIdError`) | `crates/outl-core/src/workspace_id.rs` |
| Fractional index for sibling ordering | `outl_core::Fractional` | `crates/outl-core/src/fractional.rs` |
| Resolve the page/journal slug a node sits under (walks `tree.parent` up to a registered page root; `None` if unregistered or not yet materialized) | `outl_core::Workspace::slug_for_node` | `crates/outl-core/src/workspace.rs` |

---

## 2. Tree reads (outl-core + outl-actions::tree)

| Intent | Use this | File |
|---|---|---|
| Does a node still exist in the tree? | `Tree::contains` | `crates/outl-core/src/tree/mod.rs` |
| Parent of a node | `Tree::parent` | `crates/outl-core/src/tree/mod.rs` |
| Fractional position of a node | `Tree::position` | `crates/outl-core/src/tree/mod.rs` |
| Single property lookup on a node | `Tree::property` | `crates/outl-core/src/tree/mod.rs` |
| Iterate every property currently set on a node | `Tree::properties_of` | `crates/outl-core/src/tree/mod.rs` |
| Collapsed flag for a node | `Tree::is_collapsed` / `collapsed_ids` | `crates/outl-core/src/tree/mod.rs` |
| Walk every node in the tree | `Tree::iter_nodes` / `node_count` | `crates/outl-core/src/tree/mod.rs` |
| Children of a parent, in fractional order **with a `NodeId` tiebreak** — `Fractional` alone is not a total order once two devices append offline, and `Tree::iter_nodes` walks a per-process-seeded `HashMap`, so without the tiebreak one op log rendered a different block order on every boot and on every device | `outl_actions::tree::children_of` | `crates/outl-actions/src/tree.rs` |
| Walk a subtree applying a closure. **Index-backed since this branch**: it builds a scoped children index itself (one `iter_nodes` scan per level of *depth*, not per *node*), so callers no longer pass one in. Was `O(nodes²)` — `walk_subtree(ROOT)` measured 6,832 ms → 13.2 ms at 64k nodes | `outl_actions::tree::walk_subtree` | `crates/outl-actions/src/tree.rs` |
| Sibling after a node + position helpers (for inserts) | `outl_actions::tree::next_sibling` / `position_after` / `position_for_new_last_child` | `crates/outl-actions/src/tree.rs` |
| Which page (slug-bearing root child) does this node sit under? | `outl_actions::tree::enclosing_page_id` | `crates/outl-actions/src/tree.rs` |
| Slug of the page hosting a node (`enclosing_page_id` + its `page-slug`) | `outl_actions::page_slug_of` | `crates/outl-actions/src/tree.rs` |

---

## 3. Sync engine, locks, storage trait

| Intent | Use this | File |
|---|---|---|
| The shared sync entry point (TUI poller + mobile iCloud watcher both use it) | `outl_actions::SyncEngine::new` | `crates/outl-actions/src/sync.rs` |
| Bind a sync engine to an explicit transport (iroh, test doubles) | `SyncEngine::with_transport` | `crates/outl-actions/src/sync.rs` |
| Start the transport's background tasks once the caller's channel is ready | `SyncEngine::start_transport(tx)` | `crates/outl-actions/src/sync.rs` |
| Announce new local ops to connected peers (no-op for file transport) | `SyncEngine::announce_local_ops(workspace_id, hlc)` | `crates/outl-actions/src/sync.rs` |
| Reload workspace from disk after a peer change (also seeds the caller's `HlcGenerator` from the merged log) | `SyncEngine::reload_workspace` | `crates/outl-actions/src/sync.rs` |
| Re-project a page's `.md` + sidecar to disk / reload + reproject in one call | `SyncEngine::reproject_page` / `refresh_page` | `crates/outl-actions/src/sync.rs` |
| Snapshot every / peer-only `ops-*.jsonl` (size + mtime) for change detection | `SyncEngine::snapshot` / `snapshot_peers` (`OpsFileSnapshot`) | `crates/outl-actions/src/sync.rs` |
| Scan `journals/` + `pages/` for orphan `.md` (no sidecar / stale hash) | `SyncEngine::scan_for_orphans` | `crates/outl-actions/src/sync.rs` |
| Detect projections that ran **ahead of the op log** (sidecar hash-in-sync but referencing ids no op log ever created — e.g. app killed after writing `.md`+sidecar but before the ops append) | `outl_actions::scan_for_desynced_projections(ws, root)` / `SyncEngine::scan_for_desynced_projections(ws)` | `crates/outl-actions/src/desync.rs` |
| Recover a desynced projection: re-emit `Create`/`Edit`/`SetProp` ops for the sidecar ids the tree has never seen (ids preserved, strictly additive — never resurrects a trashed block, never touches existing ones), then re-project the merged page | `outl_actions::recover_desynced_projection(ws, hlc, root, md_path)` | `crates/outl-actions/src/desync.rs` |
| Transport abstraction (iroh QUIC default; file/iCloud polling opt-in) | `outl_actions::SyncTransport` (trait) | `crates/outl-actions/src/sync.rs` |
| Filesystem / iCloud opt-in transport (polls `ops/` every 2 s, delivery is no-op) | `outl_actions::FileSyncTransport` | `crates/outl-actions/src/sync.rs` |
| Per-peer reachability snapshot from the running transport's own dials (GUI status; never bind a probe endpoint) | `SyncTransport::peer_health` → `outl_actions::PeerHealthSnapshot` | `crates/outl-actions/src/sync.rs` |
| Live sync-progress update pushed while a pass runs (connecting / snapshot bytes / ops received-pushed / synced / failed) — purely cosmetic, distinct from the load-bearing reload trigger; the pairing-screen progress feed's payload | `outl_actions::SyncProgress` | `crates/outl-actions/src/sync.rs` |
| Register a channel a transport pushes `SyncProgress` updates through (default no-op; call before `SyncTransport::start`) | `SyncTransport::set_progress_sink` | `crates/outl-actions/src/sync.rs` |
| Acquire the cross-process workspace lock (one writer at a time) | `outl_core::WorkspaceLock::acquire` | `crates/outl-core/src/lock.rs` |
| Acquire the per-actor write lock (one process writing this actor's jsonl) — advisory and **machine-local**, so it can never arbitrate between devices | `outl_core::ActorWriteLock::try_acquire` | `crates/outl-core/src/lock.rs` |
| Resolve which actor this **process** writes as (device actor, or an ephemeral one when a co-resident process holds it) | `outl_core::resolve_write_actor` | `crates/outl-core/src/lock.rs` |
| Resolve which actor this **device** writes as for a workspace — the migration-safe entry point every CLI / TUI / MCP / embedder opener calls | `outl_ws::actor::resolve_device_actor` | `crates/outl-ws/src/actor.rs` |
| Device-local actor store — per workspace *instance* (`actor_for_instance`, keyed by `WorkspaceId` **and** the workspace directory) and device-wide (`device_actor`, the Tauri clients' `<dir>/actor`) | `outl_core::DeviceStore` (errors: `outl_core::DeviceError`) | `crates/outl-core/src/device/` |
| Stable fingerprint of one physical device — the claim marker that lets exactly one device adopt a legacy `config.toml` actor | `outl_core::MachineId` (via `DeviceStore::machine_id`) | `crates/outl-core/src/device/` |
| Directory holding this device's identity files (`$OUTL_DEVICE_DIR`, else `$XDG_CONFIG_HOME/outl`, else `~/.config/outl`) — **never** inside a workspace | `outl_core::device_dir` | `crates/outl-core/src/device/` |
| Whether an actor binding may be dropped — the **single owner** of that verdict, so a listing and a prune cannot disagree (root gone **and** its parent present **and** past the TTL; anything unreadable keeps the binding) | `outl_core::BindingVerdict`, `outl_core::ActorBinding`, `outl_core::STALE_BINDING_TTL` (via `DeviceStore::actor_bindings` / `stale_actor_bindings` / `prune_binding`) | `crates/outl-core/src/device/gc.rs` |
| Device-store scratch files a killed writer left half-published (never bindings, never backed up) | `outl_core::STALE_SCRATCH_TTL` (via `DeviceStore::stale_scratch` / `prune_scratch`) | `crates/outl-core/src/device/gc.rs` |
| The `Storage` trait every persistent backend implements (invariant #5) | `outl_core::Storage` / `StorageError` | `crates/outl-core/src/storage/mod.rs` |
| **Compose an `ops/` index-sidecar filename** from `(actor, scope, kind)` — the only thing that does. Dot-prefixed in every layout so it never rides the file-sync surface; per-page names carry the **slug**, not the actor (the actor-derived name gave every page shard of one actor the same index file) | `outl_core::storage::sidecar::path_for` / `file_name` | `crates/outl-core/src/storage/sidecar/mod.rs` |
| The **complete** sidecar set for one `(actor, scope)`. Anything invalidating byte offsets — compaction is the live caller — asks for it rather than listing names it remembers; a sidecar the caller forgot is a stale offset, which is silent corruption | `outl_core::storage::sidecar::paths_for` / `remove_all` | `crates/outl-core/src/storage/sidecar/mod.rs` |
| Temp-and-rename publishing for a sidecar, buffered at 256 KiB. The flush is `into_inner()`, **not** `Drop` — `Drop` flushes and discards the error, so ENOSPC on the last buffer would read as success on a path whose whole job is durability. `append_entry` stays unbuffered on purpose (one line per call, nothing to batch) | `outl_core::storage::sidecar::write_atomic` / `buffered` | `crates/outl-core/src/storage/sidecar/mod.rs` |
| Single owner of "may this `ops/` cache file be collected". Verdicts `Live` / `Legacy` / `AbandonedScratch` / `Inconclusive`, and `verdict.reason()` carries the user-facing wording so no client writes its own. Asks *can the reader compose this name again*, never *does this actor still exist* | `outl_core::storage::sidecar::gc` | `crates/outl-core/src/storage/sidecar/gc/` |
| **Plan a log compaction** (writes nothing) — the single owner of "is this op inert?". Six conditions over the *merged* log in HLC order, because `Op::Create` is idempotent: an adjacent `Create`+`Move` pair reads two ways, and in the trashed-then-restored one the `Create` is inert and the **`Move` is doing the work** | `outl_core::storage::compact::plan_compaction` → `CompactPlan` / `CompactOptions` / `DEFAULT_HORIZON_MS` | `crates/outl-core/src/storage/compact/plan.rs` |
| **Execute that plan for *this device's* file only** — a peer's `ops-<actor>.jsonl` is that device's own append-only log, and every file transport reconciles per path last-write-wins, so a shortened copy deletes the ops it had not shipped. Narrow with `CompactPlan::restricted_to`; anything else is `CompactError::ForeignActorFile`. The unguarded `apply_compaction` is the `--force` path | `outl_core::storage::compact::apply_compaction_as` | `crates/outl-core/src/storage/compact/rewrite.rs` |
| Execute that plan — exclusive `flock` on `.outl/.lock` + a write lock on **every** actor, backup and fsync to `.outl/compact-backup/<ts>/` before any write, temp + `rename`, kept lines copied byte for byte, and the `.idx` / `.nodes.idx` sidecars **deleted** rather than rebuilt (compaction invalidates every byte offset; deleting has no failure mode where a rebuild writes a wrong one) | `outl_core::storage::compact::apply_compaction` → `CompactReport` / `ActorSaving` / `CompactError` | `crates/outl-core/src/storage/compact/rewrite.rs` |

---

## 4. Local backups (outl-actions::backup)

The safety net under every other primitive here.
Git-backed (shells out to the `git` binary — no `libgit2`, so nothing new reaches a dependent's `cargo deny`), device-local, and never part of the sync surface.

The git directory lives **outside the workspace** (`outl_core::device_dir()/backups/<slug>-<hash>.git`, workspace as `--work-tree`), for two reasons that both cost data.
The workspace is a file-sync surface, and a replicated `.git/` is a corrupted `.git/`; and a `git init` in the workspace root used to adopt the repo the user already kept there — their index, their branch, their hooks, their signing key.
A user's `.gitignore` cannot exclude the required paths (`ops/`, `pages/`, `journals/`, `templates/`, `assets/`, `.outl/config.toml`): they are force-staged, and every snapshot verifies the op log made it into the commit.

| Intent | Use this | File |
|---|---|---|
| Create / refresh the backup repo for a workspace (idempotent; nothing is written inside the workspace) | `outl_actions::backup::init` | `crates/outl-actions/src/backup/repo.rs` |
| Snapshot now — returns `Ok(None)` when nothing changed, which is the normal case on a timer and **not** an error | `outl_actions::backup::snapshot` | `crates/outl-actions/src/backup/repo.rs` |
| Snapshot from an automatic caller — swallows every failure into a `warn!` | `outl_actions::backup::snapshot_best_effort` | `crates/outl-actions/src/backup/mod.rs` |
| **Wire a client up to periodic backups** — one call, detached background thread, interval floor read back out of git | `outl_actions::backup::spawn_auto_pass` (`maybe_snapshot` / `STARTUP_DELAY` underneath) | `crates/outl-actions/src/backup/auto.rs` |
| List history, newest first | `outl_actions::backup::list` → `Vec<BackupEntry>` | `crates/outl-actions/src/backup/repo.rs` |
| Recover a past state **into a separate directory** (never in place — a recovery tool must not overwrite the live op log) | `outl_actions::backup::restore` | `crates/outl-actions/src/backup/repo.rs` |
| Is `git` usable? / does this workspace have a repo? / where is it? | `outl_actions::backup::git_available` / `is_initialized` / `repo_dir` | `crates/outl-actions/src/backup/mod.rs` |
| Drive an explicit `(git dir, work tree)` pair instead of the derived one | `outl_actions::backup::BackupRepo::at` | `crates/outl-actions/src/backup/repo.rs` |
| Why a backup failed (incl. `OpLogNotCaptured` — a snapshot missing `ops/` is an error, not a success) | `outl_actions::backup::BackupError` | `crates/outl-actions/src/backup/mod.rs` |
| Device-local preference (`enabled` defaults **on**, `interval_minutes`) | `outl_config::BackupCfg` | `crates/outl-config/src/schema.rs` |

Backed up: `ops/` (the source of truth), `pages/`, `journals/`, `templates/`, `assets/`, `.outl/config.toml`.
Excluded: derived caches that the next boot rebuilds — `.outl/snapshots/`, `*.idx`, locks, `*.tmp`.
