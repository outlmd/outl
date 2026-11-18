# Performance

outl's premise is that it is instant on a graph nobody else opens.
A 750,000-op workspace with 168,000 blocks has to feel like a text file, on a laptop and on a phone.

That is not a quality bar applied once a feature works.
It is the feature — and it fails in a shape no test catches: every function is reasonable on its own, every assertion passes, and the app cooks.

Root [`CLAUDE.md`](../CLAUDE.md#critical-invariants-never-violate) invariant 15 is the rule.
This page is the incident that bought it, the checklist it generalises to, and how to measure.

Launch specifically has its own page: [`boot-performance.md`](boot-performance.md).

## The incident: a cheap read, asked six times

`outl_sync_iroh::oplog_census` answers *which ops does this device hold*.
It was **already the optimised path**: an earlier round replaced `JsonlStorage::open` + `all_ops` (342 ms, 132 MB) with a read of the op log's offset sidecar (100 ms), after the iOS scene-update watchdog killed the app on launch over it.

That fix cut the constant and left the shape.
The read stayed `O(total ops)`, nothing memoised it, and no caller knew another had just asked the same question.

Counted on the maintainer's workspace — 748,684 ops, 23 actors, 4 peers — **one sync exchange asked for it six times**:

| Call site | When |
|---|---|
| `pending.rs` ← `peer_status` | the desktop dot's 8 s poll, plus once per `peer-ops-changed` |
| `engine_sync.rs` (clock) | once per peer per exchange, initiator |
| `engine_sync.rs` (`ops_missing_for`) | once per peer per exchange |
| `engine_serve.rs` (clock) | once per inbound connection served |
| `engine_serve.rs` (`ops_missing_for`) | once per inbound connection served |
| `oplog.rs` (ingest dedup) | once per received batch, **holding the device-wide append flock** |

On top of that, `ops_missing_for` called `JsonlStorage::open` whenever the selection was non-empty: a full load of **both** index sidecars for every actor, 160 MB resident and 55 ms, to re-derive byte offsets the census already held.

Measured cost per exchange, per peer: **~710 ms of CPU**, to move a number of ops that on a converged device is zero.
The catch-up loop runs it every 8 s against every peer; `MAINTENANCE_RESYNC` re-dials a healthy peer after 10 s.

What the user saw on a real machine (136 samples over 4.5 min of an idle app):

| | |
|---|---|
| p50 CPU | 0.5 % |
| p90 CPU | 30 % |
| peak CPU | **131 %** (more than a full core) |
| RSS | 753-779 MB, with 160 MB spikes per concurrent `open` |

The freeze was not the CPU.
It was those allocation spikes against an already-large process plus 306 MB of op log in the page cache: macOS started compressing memory and the window stopped taking keystrokes.

**No test failed.**
The entire suite was green before, during and after.

## What fixed it

Three changes, in order of how much they bought:

1. **One census per exchange, carrying the offsets.**
   `Census::read` is one pass that answers all three questions an exchange asks — the clock (`vector_clock`), the selection (`ops_missing_for` ranges over the same indexes) and the op bodies (the offsets, spent by `read_ops_at_offsets`).
   The responder shares one census between its clock and its selection, because nothing mutates the log between them.
   The initiator deliberately does **not**: its ingest appends the peer's ops in between, so the step-1 census describes a log that no longer exists.
2. **No `JsonlStorage::open` on the send path.**
   `JsonlStorage::read_ops_at_offsets(ops_dir, &[(actor, ts, offset)])` seeks straight to each op.
   `read_op_at_offset` is the one owner of "seek to an offset the index gave us and parse the op back"; the instance method `read_op_at` is its other caller.
3. **The census stops rebuilding what it just read.**
   One producer per file is the invariant every writer upholds, so the regrouping by `Hlc::actor` is almost always the identity — and taking the loaded `OffsetIndex` whole in that case skips re-inserting 748k HLCs into a second `BTreeMap`.
   That one change was 100 ms → 38 ms; the parallel read across actors on top of it is worth ~10 %, because a single actor holds 68 % of the ops ("the ceiling is one file, not the thread count", as `index_load.rs` already said).

Plus the client half: `peer_status(with_pending)`.
`pending_ops` is the only field that reads the op log and `pendingLabel` renders it in exactly one place — the Sync panel's device list.
The always-visible status dot reads `online` alone, so it asks for reachability (an in-memory read of the transport's own dials) and nothing else.

Result, on the same workspace:

| | before | after |
|---|---|---|
| per exchange side | ~221 ms (3 censuses + 1 open) | **39 ms** (1 census, 0 opens) |
| census itself | 100 ms | 38 ms |
| status-dot poll | 100 ms every 8 s + per delivered op | ~0 |
| resident index per exchange | 160 MB | 0 |

## What was rejected, and why

**A memoised census keyed on a per-actor `(len, mtime)` fingerprint.**
The op log is append-only, `Coverage::Grew` already expresses "re-scan the tail", and `outl_tauri_shared::ops_watermark` is the same idea one layer up — so this would have taken the converged case to ~0 instead of 38 ms.

It was not built, for two reasons that are worth keeping:

- **38 ms per exchange is already inside the budget.**
  Two reads per exchange against a peer re-dialled every 10 s is under 1 % of a core.
  Invariant 11's question applies to the fix as well as to the problem: attribute the remaining cost before spending a design on it.
- **A wrong invalidation here is silent non-convergence.**
  A stale census advertises a short clock (the safe direction, since a peer concludes we have a gap and resends) *or* under-sends ops we actually hold, which stalls a direction of sync with a green dot.
  That is the failure mode this crate has paid for twice already.

If it is ever needed, the cache belongs in `outl-sync-iroh`: all six call sites live there, so the owner is single and the scope closed.
It owes invariant 9's four questions, in particular *what cleans it up* — a global keyed by workspace path grows with every `TempDir` a test opens.

## The checklist

Before merging anything on a hot path — a sync exchange, a boot phase, a command the frontend polls, a render:

1. **Who else is asking this, in the same breath?**
   Grep the read's callers, not just the one you are writing.
   Six call sites with one answer is one read, passed along.
2. **Is anyone looking at this answer?**
   An expensive column computed behind a closed panel is waste, and a polling loop is where waste multiplies.
   The fix is usually not a faster computation; it is not computing it.
3. **What does it cost at the size a real graph reaches?**
   Not the fixture's nine ops.
   An `O(total ops)` read gets slower every day the user keeps using the app, which is the worst direction for a tool people live in.
4. **Does the cheap path stay cheap on the branch that does something?**
   An early return is not a budget.
   `oplog_select` had a guard for "nothing to send" and opened the whole workspace on any non-empty selection, so the fast path covered the idle device and the busy one paid everything.
5. **Does it hold a lock while it does this?**
   The ingest's census runs under the device-wide append flock, by necessity (a census taken before the lock could miss a concurrent writer).
   That makes its cost every other writer's problem, which is a reason to make it cheap rather than a reason to move it.
6. **Is a background thread actually invisible?**
   No. The UI reads the same disk and waits on the same workspace mutex.
   [`outl-tauri-shared/CLAUDE.md`](../crates/outl-tauri-shared/CLAUDE.md) → "Background passes yield, they do not race" owns that rule.

## Every client, or it does not count

A fix in one client is half a fix.
The census was reached by the desktop's status dot, mobile's 3 s poll and the TUI's `pending_by_peer` alike, because it lives under a shared command body.

- **Shared logic goes in `outl-actions` / `outl-tauri-shared` / `@outl/shared`**, so a performance fix lands once for every client.
  See [root `CLAUDE.md` → Shared logic](../CLAUDE.md#shared-logic-outl-actions).
- **A per-client polling interval is a per-client cost.**
  Mobile polls peers every 3 s, the desktop every 8 s; a read that is fine at one cadence is not automatically fine at the other.
- **The phone is the ceiling.**
  iOS kills an app that blocks the main thread (the scene-update watchdog) and jetsams one that allocates too much.
  A cost the desktop absorbs is a crash there, which is why both of this module's earlier rounds were bought by iOS and not by a laptop.

## How to measure

**On the device, on a real graph.**
A fixture reports nine ops and no problem.

A running process, by sample:

```sh
pgrep -f '/Applications/outl.app/Contents/MacOS/outl-desktop'
sample <pid> 10 -f /tmp/outl-sample.txt
# the section that matters:
sed -n '/Sort by top of stack/,$p' /tmp/outl-sample.txt | head -20
```

Read "top of stack" first: it is where CPU is actually being spent, as opposed to the call graph's depth.
Frames in `outl-desktop` above the system libraries are ours.

CPU and RSS over time, which is what shows a spike a single sample misses:

```sh
while :; do
  ps -o %cpu=,rss= -p <pid>
  sleep 2
done
```

A launch reports itself: `outl_core::boot_timing` writes one `info!` line per workspace open naming each phase, the delta size and the resident block count.
On iOS that lands in `$HOME/Library/Logs/outl-backend.log` inside the sandbox ([`outl-mobile/CLAUDE.md`](../crates/outl-mobile/CLAUDE.md) → "The backend's log").

A one-off micro-benchmark against the real workspace, with no risk to it:

```sh
cp -R ~/outl-p2p/ops /tmp/ops-copy   # sidecars are dotfiles; cp -R takes them
# then a tiny crate with a path dependency on outl-core, pointed at the copy
```

**Copy, never open the live workspace** — a read path that persists a sidecar or mints an actor would change the thing being measured.

## Make the net structural

A timing assertion is flaky in CI and tells you nothing about *why*.
A structural one does both.

The pattern this repo uses: **delete the expensive artefact and fail if it comes back.**

- `a_selection_with_ops_to_send_still_costs_no_open`, `a_gap_resend_also_costs_no_open` (`outl-sync-iroh/src/oplog_select/tests.rs`) — the sidecars stay deleted, because a reroute through `JsonlStorage::open` rebuilds and persists them.
- `the_actor_census_neither_reads_nor_writes_the_node_index` (`outl-core/tests/index_census.rs`) — the same tell, for the census.
- `ops_are_read_back_from_census_offsets_without_opening_storage`, `the_timestamp_census_is_the_projection_of_the_offset_census` — the offsets address real ops, and the two census shapes cannot disagree about what the workspace holds.
- `Workspace::resident_text_count()` is `pub` for this reason: a downstream test can assert a read path did **not** force the whole workspace to materialise.
- `peer_status_cost.rs` (`outl-tauri-shared/tests/`) — the same signature one layer up: the status dot's call must leave the op log untouched, and a sidecar appearing is the proof it did not.

**Three questions, three kinds of test**, because no single one covers them:

| Question | Test | Catches |
|---|---|---|
| does it still do the right thing? | `reading_by_offset_returns_the_same_ops_as_opening_the_storage` | the cheap read and the read it replaced disagreeing about the op log |
| is it still cheap? | the no-open tests, `peer_status_cost.rs` | an expensive read coming back on any branch |
| is it cheap *once*? | `census_per_exchange.rs` | a caller re-reading what it already holds |

The first is the one that answers "did I break sync": what crosses the wire is op **bodies**, so it reads every variant back by offset and compares against `JsonlStorage::open` + `read_ops_at`, with an `Op::Edit` carrying a real Yrs delta and text holding a newline and non-ASCII.
Equal bodies mean sync behaviour cannot have moved.

The third reads the **source** of the call sites, which is the only way to count reads a type cannot bound.
It guards both directions and they fail differently: adding a read fails the test, while *collapsing* the initiator's two into one — the tidy-up that was correct for the responder and drops ops here, because its ingest sits between them — does not compile, since step 1 holds a clock and the selection needs a census.
The test records why the count is two, so nobody makes that collapse expressible.

**And cover the working branch, not just the fast one.**
`a_peer_that_is_current_costs_no_op_read` existed throughout this incident and passed throughout, because a converged peer returns early and never reaches the code that opened storage.
A fast-path test that skips the branch that does something is how a cost this size stays hidden.
