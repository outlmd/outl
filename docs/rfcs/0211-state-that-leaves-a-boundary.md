# RFC 0211 — State that leaves a boundary arrives somewhere with different rules

| | |
|---|---|
| **Status** | Accepted |
| **Issue** | [#211](https://github.com/outlmd/outl/issues/211) |
| **PR** | — |
| **Date** | 2026-08-06 |
| **Reference doc** | [development.md](../development.md), [storage.md](../storage.md) |
| **Invariant** | root `CLAUDE.md` invariant 9 |
| **Guarded by** | see **How it cannot regress** — the guard is the point of this RFC |

## Why

Three defects found in one session share a shape, and the shape is more useful than any of them individually.

**One.** [RFC 0210](0210-md-content-outside-op-log.md): #166 fixed "the tree ran ahead of the `.md`" and created "the `.md` ran ahead of the tree".
The first hides content, the second deletes it.
233 pages, 1,426 lines.

**Two.** 0.11.0 moved the write actor out of `.outl/config.toml` into a device-local store.
That fix was right: the file rode every sync transport except iCloud, so two devices resolved the same actor and appended to one `ops-<actor>.jsonl` with `flock(2)` unable to arbitrate.
It also moved the state somewhere with **no test-isolation story**.
`device_dir()` reads `OUTL_DEVICE_DIR` / `XDG_CONFIG_HOME` — process-wide and machine-wide — and only `outl-core`'s own tests isolate it.
Result on a dev machine: 64 entries in `~/.config/outl/actors/`, **15 of them orphaned `TempDir` paths written by the test suite**, plus three doctor tests that fail once in seven runs ([#211](https://github.com/outlmd/outl/issues/211)).

**Three.**
Three per-crate `CLAUDE.md` files grew past the 40,000-char ceiling the `markdown-size-guard` hook enforces.
The ceiling is correct — those files load whole into an LLM's context, and past ~40k the attention spent re-reading instructions comes out of the actual task.
But a guard that only says *no* becomes a wall: the hook blocked adding RFC links to exactly the three crates that needed them, and the only way forward was work nobody had scheduled.

None of the three was a careless change.
Each was a correct fix whose **relocated** problem went unexamined.

That is the generalization worth writing down: a fix rarely deletes a problem, it moves it.
The question after "did I fix it?" is **"where does the problem live now, and what does that place require that the old one did not?"**

- State leaving the workspace stopped being sync-visible and started being machine-global — so it needed an isolation story it never got.
- A divergence fixed in one direction left the mirrored direction unstated — so the mirror shipped as a deletion bug.
- A limit with no escape hatch stopped being a limit and became a blocker.

## What we chose

Three things, in increasing generality.

**The isolation, which already existed.**
`.cargo/config.toml` sets `OUTL_DEVICE_DIR` under `[env]` to a path inside the repo, so every process cargo launches resolves the store there rather than in `~/.config/outl`.
It pointed at `target/device-store` when this was written and now points at `.dev-device-store`.
`cargo clean` deletes `target/`, including the iroh identity key that **is** this device's node id, so every clean turned a `cargo run` desktop into a brand-new device and broke its pairings.
One repo-wide value, not a temp dir per test: `std::env::set_var` is process-global and Rust tests share a process, so mutating it per test would be racy by construction, and the fix cannot be another instance of the bug.
Two gaps the mechanism does not cover, both worth knowing: a git worktree created before that file existed has no copy of it, and cargo only exports `[env]` to processes it launches, so invoking a built binary directly still reaches the real store.

**The guard.**
`outl-core`'s `the_test_suite_runs_against_an_isolated_device_store` fails outright when the override is missing.
`outl-ws`'s `device_isolation` asserts it from the other side: `open` binds its actor under `device_dir()` and leaks no identity into the workspace.
Without them, nothing catches the next path that escapes, and "64 entries" is what "nothing catches it" looks like after a year.

**The production bug the shared store exposed.**
`DeviceStore::machine_id` minted with a plain write while `bind` used a compare-and-swap, so two processes reaching a fresh store both minted an id and the last writer won.
`outl init` racing `outl serve` on a new machine permanently broke that workspace's actor claim.
The compare-and-swap was also failing open: a bare `O_EXCL` open creates an empty file, and every reader maps a blank record to "absent", which is exactly the answer that licenses overwriting.
Minting now goes through `create_new_record`, which composes in a temp file and publishes with `link(2)`.

**The rule, as invariant 9 in the root `CLAUDE.md`.**
When state moves across a boundary — out of the workspace, out of a file, out of the op log, into a global — the RFC that moves it must state what the new location requires.
The checklist is short because it has to be remembered: *who can write it, who can read it, how does a test get its own copy, and what cleans it up?*
The 15 orphans are the answer to the fourth question being "nothing".

## Why not the alternatives

**Fix the three flaky tests and move on.**
The flakiness is a symptom, and the cheap read of it ("tests are racy, add a mutex") leaves the suite still writing to the user's machine.
It also leaves the next test free to do the same, since nothing would object.

**Set `OUTL_DEVICE_DIR` from a test harness (a `#[ctor]`, a shared `setup()`).**
Rejected, and the distinction against what shipped is narrow enough to be worth stating.
A *harness* makes correctness depend on the harness running, so a single `cargo test --bin`, a doctest, or a run through an IDE could skip it and quietly touch the real store.
`[env]` in `.cargo/config.toml` is set by cargo itself before the process starts, so it holds for every one of those, including `cargo run`.
It buys that at the cost of not covering what cargo does not launch, which is why invoking a built binary by hand still writes to the real store.

**Make `device_dir()` panic outside a temp dir under `cfg!(test)`.**
Attractive as a hard guarantee, and wrong at the boundary: `cfg!(test)` is false in integration tests (`tests/*.rs`), which is where most of the offenders live, so it would guarantee the wrong half.

**Write this as a checklist in `docs/contributing.md` instead of an invariant.**
The contributing guide is read when someone joins and rarely after.
`CLAUDE.md` is read on every edit to the crate, which is when the question needs to be in front of whoever is moving the state.
That is the same reasoning as RFC 0210's second layer, and the reason this project keeps rules in `CLAUDE.md` and rationale in RFCs.

## The opposite direction

**What this makes worse.**
Every test that opens a workspace now pays a temp-directory setup it did not before — small, but real on a suite this size, and one more thing to get right in a new test.
More honestly: a test that isolates the device store is a test that no longer exercises the real resolution path.
`device_dir()`'s actual precedence chain (`OUTL_DEVICE_DIR` → `XDG_CONFIG_HOME` → per-OS default) is now covered only by `outl-core`'s own tests, and a regression in the per-OS branch would surface on a user's machine before it surfaces in CI.
That is the trade accepted here, and it is the mirrored risk this section exists to name.

**The mirror of the rule itself.**
An invariant that asks four questions of every boundary crossing can become ceremony — asked of changes where the answers are obvious, and eventually answered by rote.
The guard against that is that it applies to *state crossing a boundary*, not to every change; a new function that reads existing state crosses nothing.

**What this does not make better.**
The 15 orphans are still there, and the store still has no GC — a workspace the user deletes leaves its entry forever.
Preventing new pollution does not clean up old, and nothing in this RFC does ([#211](https://github.com/outlmd/outl/issues/211) item 3).

> **Followed up.**
> The GC landed later as `outl-core`'s `device/gc.rs`, surfaced through `outl doctor` (reports) and `outl doctor --repair` (drops, after a backup).
> Its design is one long refusal: a binding goes only when its root is gone, its root's *parent* is still present, and the record is over 30 days old, because dropping a binding forks that workspace's actor and an unmounted volume is indistinguishable from a deleted folder without the parent check.
> Wiring it also produced this RFC's own lesson a second time: `doctor` reads machine-global state, so `collect_internal` had to take the `DeviceStore` as a **parameter** — resolving it inside the pass would have made every doctor test judge and delete from one shared store, which is the flakiness in this issue's title, reintroduced by its fix.
> See [`doctor.md`](../doctor.md#the-device-stores-stale-actor-bindings).

## How it cannot regress

1. **Invariant 9** in the root `CLAUDE.md` carries the four questions and points here for the three incidents that produced them.
   It is stated as a requirement on the RFC that moves state, so it is checked in review, where the boundary crossing is visible.

2. **The escape guard** is the mechanical half: a test that fails when the suite writes outside its temp directories.
   Its doc comment says why it exists so nobody "cleans it up" as redundant.

3. **The template already asks the general question.**
   [`0000-template.md`](0000-template.md)'s **The opposite direction** is required and non-deletable, and this RFC is the third incident that section exists to catch.
   Invariant 9 is the specialization for boundary crossings, where the answer is usually "the new location has rules nobody wrote down".

## Scope

**Not covered — GC for the device store.**
Pruning the 15 orphans and handling workspaces the user deletes is [#211](https://github.com/outlmd/outl/issues/211) item 3.
It is a product decision (when is an entry safe to drop?), not a test-hygiene one.

**Not covered — the `CLAUDE.md` ceiling.**
The three oversized files are being extracted into `docs/`.
The general problem the third incident revealed — a guard with no escape hatch — has no mechanism here.
The ceiling does already cover the new path-scoped instruction files, since `*` matches `/` in a bash `case` — a redundant arm added on the assumption it did not has been removed.

**Not covered — the other five test gaps.**
Writing the retroactive RFCs surfaced behaviour nothing pins, listed in [`README.md`](README.md#gaps-these-rfcs-surfaced).
Each is recorded as `none found — gap` in its RFC rather than papered over, and each needs its own change.
