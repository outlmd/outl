# RFC 0137 — Storage scale: constant RSS, then constant boot/sync

| | |
|---|---|
| **Status** | Phase A shipped; Phase B pending |
| **Issue** | [#137](https://github.com/avelino/outl/issues/137) |
| **PR** | 11 PRs across two phases (A done, B pending) — branch `feat/storage-scale-rfc-137` |
| **Date** | 2026-07-04 |
| **Reference doc** | [storage.md § Boot reads an index](../storage.md#boot-reads-an-index-not-the-whole-log-rfc-137-front-a) |
| **Invariant** | none (perf work; `outl-core/CLAUDE.md` → "Snapshot dir has exactly one owner" describes the resulting rules) |
| **Guarded by** | `boot_scale_bench.rs` (bench, not a test — see Scope) |

## Why

`JsonlStorage` keeps every op ever written in resident memory. The cache
field was `RwLock<Vec<LogOp>>` at [`storage/jsonl.rs:42`][src-cache];
`reload()` reads every `ops-<actor>.jsonl` line-by-line into that `Vec`
([`storage/jsonl.rs:92-130`][src-reload]); `boot_from_full_replay`
([`workspace.rs:230-269`][src-full]) loads all of them through `OpLog`
on top. Phase 1 snapshot (#128) speeds up boot but doesn't reduce RSS —
the storage cache still mirrors everything.

Measured on M-series Mac, release build, via `boot_scale_bench.rs`:

| ops    | boot_snap | boot_full | rss_snap | rss_full | source      |
|--------|-----------|-----------|----------|----------|-------------|
| 50k    | 27 ms     | 31 ms     | 37 MB    | 54 MB    | measured    |
| 100k   | 55 ms     | 64 ms     | 66 MB    | 98 MB    | measured    |
| 250k   | ~140 ms   | ~160 ms   | ~160 MB  | ~230 MB  | extrapolated |
| 500k   | ~280 ms   | ~320 ms   | ~300 MB  | ~470 MB  | extrapolated |
| 1M     | ~550 ms   | ~640 ms   | ~600 MB  | ~900 MB  | extrapolated |

Per-op cost: ~0.55 µs boot, ~590 bytes RSS. Both linear.

**Boot is not the wall.** Even at 1M ops, 550 ms is acceptable.
**RSS is the wall.** Each `Op::Edit` costs ~590 bytes of resident memory
forever — even after a snapshot, even after the block was deleted.
Mobile hits the wall first (iOS jetsam kills above ~500 MB).

## Decisions (locked)

1. **LRU + mmap first (Phase A), per-page shards second (Phase B).**
   Phase A bounds RSS at the storage layer without a format break.
   Phase B bounds boot and sync at the workspace layer. Both are needed
   because a single page (10-year journal) can still grow without bound.

2. **`PageScope` default = `PerPage` in new workspaces** (Phase B).
   Existing workspaces stay on `Global` until migrated. Migration CLI
   is opt-in.

3. **Include iroh-blobs snapshot transfer** as PR #9 in Phase B
   (closes Phase 2 of #128). Reduces wire bytes for fresh peer pairing.

4. **Single long-running branch** `feat/storage-scale-rfc-137`, merged
   to `main` as one PR (squash). Each sub-PR is a logical commit.

## Phase A — constant RSS (SHIPPED)

Replace `Vec<LogOp>` with a bounded LRU (cap configurable in
`outl.toml` via `[storage] lru_cap`, default 20k ops desktop, 5k on
mobile). Add a per-actor offset index `(actor, hlc) → file offset` and
a per-node secondary index `NodeId → Vec<(Hlc, offset)>` so cold ops
stay addressable without buffering the whole log.
`Workspace::apply_lru_cap(cap)` is called by every long-lived client
AFTER boot finishes re-materialising Yrs `Doc`s via `ops_for_node` —
boot needs every op in RAM, the long-running client sheds cold history
afterwards.

Cold reads use `File + seek + read_line` (not mmap — see
"Decision: mmap deferred" below).

Result: RSS ≈ constant (LRU cap + index), regardless of
history. No `.jsonl` format change. No migration. No client breaks.

### Phase A PRs (DONE)

| # | Title | Status |
|---|-------|--------|
| 1 | `perf(core): per-actor offset index sidecar` | shipped — `crates/outl-core/src/storage/index.rs` (new), `JsonlStorage::reload` populates + persists `ops-<actor>.idx` |
| 2 | `perf(core): replace Vec<LogOp> cache with bounded LRU` | shipped — `cache: RwLock<LruCache<Hlc, LogOp>>`, `JsonlStorage::open_with_cap`, `[storage] lru_cap` in `outl-config` |
| 3 | `perf(core): Workspace::apply_lru_cap shrinks cache post-boot` | shipped — `Storage::resize_cache` (default no-op), `Workspace::apply_lru_cap`, wired in TUI + desktop + mobile |
| 3.5 | `perf(core): per-node secondary index + cold ops_for_node` | shipped — `crates/outl-core/src/storage/node_index.rs` (new), `ops_for_node` falls through to per-node index + disk read when LRU is empty, regression test `ops_for_node_survives_lru_eviction` |

**Milestone verification**:
- `cargo test -p outl-core` passes (CRDT invariants intact, 100%
  coverage on the four critical functions preserved).
- New unit tests cover the LRU cap (`bounded_lru_evicts_old_ops`,
  `reload_with_bounded_lru_keeps_cap`, `ops_for_node_survives_lru_eviction`)
  and both indexes (HLC roundtrip, missing/corrupt file, rebuild from
  jsonl; per-node roundtrip, rebuild, actor routing).
- `boot_scale_bench.rs` is the long-running scale measurement
  (`cargo test -p outl-core --release --test boot_scale_bench
  boot_scale_100k -- --ignored --nocapture`).

## Decision: mmap deferred

The original Phase A sketch mentioned mmap (`memmap2`) for the cold
read path. After analysis we kept `File + seek + read_line` instead.
This section records why so the next person doesn't re-litigate it
without new evidence.

**Why mmap was tempting**

- Cold `ops_for_node` would be zero-syscall (pointer dereference into
  the kernel page cache) instead of one `lseek` + one `read` per op.
- Mature tool — lmdb, sled, sqlite, redis all use it. Yrs (which we
  already ship) uses it internally.
- Phase B per-page shards would multiply file handles; mmap would let
  the kernel manage the working set instead of us holding N `File`s.

**Why we declined**

- `outl-core` is `#![forbid(unsafe_code)]` (`src/lib.rs:12`).
  `Mmap::map` is `unsafe`. Lifting `forbid` to `deny` + a local
  `#[allow]` was the only path, and `forbid` is a project value, not a
  style choice. Changing it is a philosophical decision, not a
  technical one.
- **SIGBUS.** A file that disappears under a mmap (iCloud lazy
  download, manual truncation, FS corruption) takes the process down.
  Recovery needs a signal handler (more unsafe) or pre-read validation.
  `File::open` returns `Err` cleanly.
- **Mmap counts toward RSS.** On iOS, mapped pages show up in the
  jetsam accounting. We'd be undoing the very RSS bound Phase A
  delivers.
- **Concurrent appender race.** Two processes sharing `ops/` (TUI +
  desktop on the same Mac) can grow the file while another holds a
  stale mmap. Reading past the old end is undefined.
- **Windows divergence.** `CreateFileMapping` + `MapViewOfFile` differ
  from POSIX mmap in locking and resize semantics. Tauri 2 doesn't
  abstract this at our layer.
- **Marginal gain at current scale.** Cold `ops_for_node` is O(K)
  where K = Edit ops on one block (typically < 100). 100 syscalls ×
  ~10 µs ≈ 1 ms. Imperceptible. The offset bug (every entry returning
  offset 0 because `O_APPEND` doesn't update `stream_position`) was
  the real correctness issue and is fixed.
- **Phase B kills the root cause.** Per-page shards make every jsonl
  small by construction. mmap becomes overkill the moment #37 lands.

**When to re-open**

- Real-world workspaces pass 500k ops AND Phase B hasn't landed in
  six months.
- A real flamegraph (not a synthetic bench) shows `ops_for_node` in
  the top 5 hot paths.
- Someone proposes ChronDB (#1) — a new format warrants rethinking
  the whole storage layer, not just the read path.

## Phase B — constant boot/sync (#37 / Part 2 of `sync.md`) — PENDING

`PageScope`, per-page op log shards, `migrate-to-per-page-ops`, four
clients update. Builds on Phase A — each page-scope reuses the LRU +
mmap storage layer.

This is a 1-2 month refactor with a format break and migration CLI.
It cannot be done blind in a single session — the op log format is
the sync correctness invariant, and a bad migration corrupts every
paired device. Tracked separately.

### Phase B PRs (PENDING)

| # | Title | Touches |
|---|-------|---------|
| 5 | `refactor(core): PageScope on Storage trait (default Global)` | `storage/mod.rs`, `storage/jsonl.rs`, `workspace.rs` (back-compat: `Global` matches current behaviour byte-for-byte) |
| 6 | `feat(core): per-page op log shards for new workspaces` | `storage/jsonl.rs` (write path), `outl-cli init` (default scope = `PerPage`) |
| 7 | `feat(cli): migrate-to-per-page-ops command` | `outl-cli/src/cmd/migrate_to_per_page_ops.rs`, idempotent + `.v0.bak` backup |
| 8 | `refactor: clients use page scopes` | TUI, mobile, desktop, CLI — `open_or_create` dispatches by scope, boot loads page lazily on navigation |
| 9 | `feat(sync): iroh-blobs snapshot transfer (closes #128 phase 2)` | `outl-sync-iroh` — ship snapshot binary then delta ops on fresh pair |

**Phase B prerequisites before landing**:
1. Workspace backup to test the migration against (a real 3-year vault).
2. Wire-format review — per-page shards change what iroh ships.
3. `crdt-invariant-checker` agent pass on any change to `apply_op`.
4. `markdown-roundtrip-tester` agent pass on the migration output.

## Open questions (Phase B)

- **LRU cap default for mobile** — resolved: 5k (mobile pins
  `min(cfg.lru_cap, 5_000)` in `outl-mobile/src-tauri/src/workspace_open.rs`);
  desktop default 20k.
- **mmap cold-read path** — resolved for Phase A: declined (see
  "Decision: mmap deferred"). Reopen only if real-world flamegraphs
  show `ops_for_node` in the hot path AND Phase B hasn't landed.
- **Index persistence** — resolved: sidecar `ops-<actor>.idx` (Phase A).
- **mmap strategy** — deferred to follow-up; Phase A reads cold ops via
  `File + seek + parse`, which is correct but slower than mmap. Land
  mmap when a profile shows the cold-read path is hot.
- **Snapshot interaction** — resolved: `apply_lru_cap` runs AFTER
  `ops_for_node` finishes re-materialising Yrs `Doc`s at boot (see
  `Workspace::apply_lru_cap`).
- **Throughput cost of cold reads** — pending; needs a micro-bench
  before mmap lands. Today `ops_for_node` returns only the resident
  tail, which is what boot-from-snapshot needs (delta-edited nodes
  are still warm).

## Non-goals (explicit)

- **Op log compaction** (#110). Orthogonal. The LRU doesn't delete ops
  from disk; compaction is a separate horizon (`Undo horizon`).
- **Block text CRDT replacement**. Yrs stays. The two-tier `Doc` cache
  (#108's fix) stays.
- **ChronDB backend** (#1). Out of scope. The `Storage` trait changes
  planned for Phase B (PageScope) are forward-compatible with ChronDB.

## References

- Issue: <https://github.com/avelino/outl/issues/137>
- Companion: `docs/sync.md` Part 2 (the design Phase B implements)
- Related issues: #37, #128, #110, #109, #108
- Bench: `crates/outl-core/tests/boot_scale_bench.rs`
- Phase A code:
  - `crates/outl-core/src/storage/index.rs` (offset index sidecar)
  - `crates/outl-core/src/storage/jsonl.rs` (LRU + cap + index wiring)
  - `crates/outl-core/src/storage/mod.rs` (`Storage::resize_cache`)
  - `crates/outl-core/src/workspace.rs` (`Workspace::apply_lru_cap`)
  - `crates/outl-config/src/schema.rs` (`[storage] lru_cap`)

[src-cache]: https://github.com/avelino/outl/blob/main/crates/outl-core/src/storage/jsonl.rs#L42
[src-reload]: https://github.com/avelino/outl/blob/main/crates/outl-core/src/storage/jsonl.rs#L92
[src-full]: https://github.com/avelino/outl/blob/main/crates/outl-core/src/workspace.rs#L230

