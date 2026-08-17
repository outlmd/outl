# CLAUDE.md — outl

Context for Claude Code sessions working on this repo.
Read this before making any change.

## What this project is

**outl** is a local-first outliner (Roam/Logseq replacement) with:

- **Markdown as source of truth** — `.md` files are 100% clean, no visible IDs.
- **Conflict-free sync** via a tree CRDT (Kleppmann et al. 2022).
- **Trait-based storage** — JSONL (one file per actor) is the only persistent backend; ChronDB on the roadmap.
- **TUI as a first-class citizen**, not an afterthought.
- **Journal-first** — daily notes are the primary entry point.

Full spec lives in the README and `docs/`.
Don't skim — read.

## Critical invariants (NEVER violate)

These are the non-negotiables.
Violating any one breaks user trust irreversibly.

1. **Op log is source of truth.**
   All mutations go through `Op` → `apply_op` → log.
   The materialized tree and `.md` files are projections.
   Never edit `.md` to "fix" state.

2. **Markdown stays 100% clean.**
   No `id::`, no UUID inline, no HTML comments, nothing.
   IDs live ONLY in the `.outl` sidecar (JSON file next to the `.md`, e.g. `pages/foo.outl`).
   The sidecar is **not** a dotfile, because iCloud Documents (when used as the file transport) drops dotted paths during cross-device sync and silently breaks multi-device workspaces.
   Same rule applies to `ops/`.

3. **CRDT follows Kleppmann 2022 literally.**
   `do_op` / `undo_op` / `apply_op` / `creates_cycle` must match the paper.
   100% coverage on these four is non-negotiable.

4. **Move that creates a cycle is a no-op on the materialized tree, but the op still goes into the log.**
   Removing it breaks correctness of future reordering.

5. **Storage is a trait, not a struct.**
   `JsonlStorage` is the only persistent impl, and tests use `MemoryStorage`.
   Anything that wants to persist ops goes through `dyn Storage`.
   No second persistent backend lands without an issue + RFC first, because divergence between storages is exactly what we paid to remove in 0.5.0.

6. **Delete is `Move(node, TRASH_ROOT)`, not physical removal.**
   Simplifies the algorithm and preserves history.

7. **Any state that must converge between devices goes through the op log.**
   If two users (or one user on two devices) can disagree about a value and you want them to reconcile, the state belongs in an `Op`, *never* in a shared file with last-write-wins semantics.
   The op log gives each actor its own `ops-<actor>.jsonl`, lets iroh / iCloud / Syncthing / shared FS sync per-file (no merge conflicts), and replays through the CRDT with HLC ordering for deterministic convergence.
   Writing the state into the sidecar (or any single shared file) bypasses all of that and loses concurrent writes silently.
   **Default position: model it as an Op.**
   `Op::SetCollapsed` for the fold flag is the canonical example.
   The sidecar carries only **structural matching metadata** (ids, position, content hash, ref handle), not a sync surface.

8. **A sidecar hash match is NOT evidence that the `.md` came from the op log.**
   `sidecar.last_synced_hash == file_hash(disk)` answers exactly one question: *did outl write these bytes last?*
   It does **not** answer *did these bytes come from the op log?*
   Those are different questions, and a page can answer yes to the first and no to the second — that is precisely the state a `reconcile_md` leaves behind when it rewrites the sidecar without emitting ops covering everything it read.
   So **never overwrite a `.md` on the strength of the hash gate alone.**
   Before re-projecting, ask `outl_actions::content_lines_missing_from(disk, &sidecar.blocks)` and refuse when it returns anything (`ActionError::PageMarkdownAheadOfLog`).
   **Ask it against the sidecar's blocks, never against a fresh render.**
   The sidecar is what the log held when the two last agreed, so it answers *"does the log know this line"*; a render answers *"do disk and tree disagree"*, and that is also yes for every remote edit, remote delete and reorder.
   The first version of this guard compared against the render and therefore refused to re-project any page a peer had touched — issue #166 reintroduced, with the blame moved.
   That function is the single owner of the verdict; a second opinion about which pages are safe to overwrite is how a read-only listing promises a repair the writing pass then refuses.
   **A refusal has to reach the user.**
   Refusing freezes the page in both directions until `outl reconcile --ahead-of-log` runs.
   A client that swallows the error into a log line therefore ships a page that silently stopped syncing — the same "silence is the defect" failure this invariant exists to prevent, moved one layer up.
   Every surface names it: [`docs/clients.md` → Surfacing a page that stopped syncing](docs/clients.md#surfacing-a-page-that-stopped-syncing).

   **Why this is an invariant and not a code comment.**
   Issue #166 fixed a real bug (tree ahead of `.md`, page renders empty) by re-projecting whenever the tree outran a *faithful* projection, where faithful meant the hash matched.
   That traded one divergence direction for its mirror image, and the mirror is worse: `.md` ahead of tree is content **deleted**, while tree ahead of `.md` was only content **hidden** — the op log still had it.
   On a real 2,560-page workspace that cost 233 pages holding 1,426 lines of work that existed in no op.
   They were deleted by `doctor --repair` (which printed `708 fixed`) and by every GUI page open, with the rebuilt sidecar making the loss undetectable afterwards.
   Full reasoning, rejected alternatives and what this change makes *worse*: [RFC 0210](docs/rfcs/0210-md-content-outside-op-log.md).

   **The producer is fixed too, not just the guard.**
   The state above only exists because `render → parse` was not a roundtrip for a block whose text held a blank line or its own indentation.
   `outl-md`'s permissive parser used to close continuation on the first blank line (dropping everything after it) and silently skip an unplaceable line at any depth below 0.
   Both are fixed in `crates/outl-md/src/parse.rs`; see `crates/outl-md/CLAUDE.md` → "What this crate owns" and its own invariant 8 entry.
   `reconcile_md` now enforces invariant 8 in code — it will not advance `last_synced_hash` over content it could not emit an op for.
   The matching side has the mirrored guard: `reconcile_md_with_guard` refuses a bulk delete instead of trashing an oversized orphan list (`crates/outl-md/src/matching/guard.rs`).
   `outl recover` (op log, a truncating `Op::Edit`'s still-live predecessor) and `outl reconcile --ahead-of-log` (the `.md`, content the log never saw) are the two routes to recover the 1,426 lines this incident already produced.
   See `docs/cli.md` → "outl recover".
   Full status: [issue #210](https://github.com/outlmd/outl/issues/210).

   **The general rule this is an instance of:** when you fix one direction of a `.md` ↔ tree divergence, state what happens in the opposite direction *before* merging.
   Reconciliation bugs come in mirrored pairs, and the pair that deletes is never the one being reported.

   **The regression net.**
   These tests exist to fail if someone re-simplifies the gate back to a hash comparison — do not delete or relax them:
   `if_stale_refuses_when_the_md_carries_content_the_log_lacks`,
   `if_stale_still_reprojects_when_the_md_holds_no_unlogged_content`,
   `if_stale_ignores_whitespace_only_differences_when_deciding`,
   `if_stale_declines_when_the_sidecar_cannot_answer` (an empty verdict from a reference that *cannot* answer is not permission to write — that is how a peer on an older binary re-arms the loss),
   `if_stale_still_projects_a_page_whose_sidecar_has_no_blocks` (the opposite case: nothing on disk to lose)
   (`crates/outl-actions/src/journal/tests.rs`), plus
   `recovery_does_not_reproject_over_text_the_log_never_saw`
   (`crates/outl-actions/tests/desync_recovery.rs`, the same defect reached through the desync recovery's re-projection), plus
   `a_torn_op_log_never_lets_repair_overwrite_a_good_md`
   (`crates/outl-cli/src/cmd/doctor/tests/safety.rs`), which pins the precedence order: a damaged op log is reported as the damaged log, not as 2,000 pages of "unlogged content".
   The producer side has its own net, pinning that a line is never dropped rather than just refused-to-overwrite:
   `crates/outl-md/tests/multiline_block_roundtrip.rs` (blank lines, indentation and unplaceable lines round-trip, never truncating the op log on the next reconcile).
   `crates/outl-actions/src/recover/tests.rs` (the truncation signature and the additive-only restore).

9. **When state crosses a boundary, say what its new home requires.**
   Moving state out of the workspace, out of a file, out of the op log, or into a process global does not delete its problems — it hands them to a place with different rules.
   The RFC that moves state must answer four questions about the destination: **who can write it, who can read it, how does a test get its own copy, and what cleans it up?**

   Not theory.
   0.11.0 moved the write actor out of `.outl/config.toml` into a device-local store, correctly — the file rode every sync transport except iCloud, so two devices resolved the same actor and appended to one `ops-<actor>.jsonl`.
   Question three went unanswered, so the test suite wrote into the developer's real `~/.config/outl` (64 entries, 15 of them orphaned `TempDir` paths) and three doctor tests went flaky.
   Question four went unanswered for longer — nothing pruned a workspace the user deleted, and `actors/` reached 1,208 records on one machine, 1,166 of them orphaned.
   `outl-core`'s `device/gc.rs` answers it now, reported by `outl doctor` and dropped by `--repair`.
   What that took is the part worth carrying.
   The obvious rule ("delete the ones whose directory is gone") is wrong: an unplugged drive and a deleted folder are the same observation, and a wrong delete forks the workspace's actor — the failure the store exists to prevent.
   **A cleanup answer is only as good as what it refuses**, so the rule is three conditions, not one, and every reading it cannot make faithfully keeps the entry.
   [RFC 0211](docs/rfcs/0211-state-that-leaves-a-boundary.md), [issue #211](https://github.com/outlmd/outl/issues/211).

   **The general rule, of which invariant 8 is one instance:** a fix relocates a problem far more often than it removes one.
   After "did I fix it?" comes **"where does the problem live now, and what does that place require that the old one did not?"**
   Three separate defects in this codebase came from skipping that question — a divergence fixed in one direction only (invariant 8), state moved without an isolation story (this one), and a size guard with no escape hatch that turned into a wall.

10. **When you change who decides, enumerate who was standing on the old decision.**
    Replacing a hardcoded policy with a runtime one is usually the right fix, and it is also the change most likely to break something that policy was quietly holding up.
    A policy has beneficiaries that never had to declare themselves, because under the old rule they always won.

    Not theory.
    Issue #220: "only the GUI binds the iroh endpoint" was a policy that fixed a real collision and assumed a GUI exists, so a headless `outl mcp serve` machine synced with nobody, silently.
    Replacing it with a lease was correct.
    What the change missed was everyone leaning on "the GUI always wins":

    - **Desktop pairing** read the transport straight out of app state, because a GUI that always held the endpoint always had one.
      Losing the election made adding a device impossible.
    - **`outl peer status`** was documented as exempt from "never bind a second endpoint", on the grounds that the CLI has no running transport to conflict with.
      True under the old policy.
      False the moment the MCP server could hold the endpoint — and status is the command a user runs *to diagnose sync*.
    - **`$OUTL_DEVICE_DIR`** already meant "throwaway actor".
      Giving it a second job (moving the iroh identity) silently rotated the node id for anyone already exporting it.

    Then the guard itself.
    The lease was stored on the transport rather than on the endpoint thread, so a failed `bind()` killed the thread while the transport kept the claim, locking every process on the device out of an endpoint.
    That is the #220 bug again, this time with a padlock.
    Releasing it any earlier reopens the mirror image: two endpoints on one node id while the first is still closing.

    Before merging a change that moves authority, answer:

    1. **Who consumed the winner?**
       Grep the consumers of the resource, not just its producers.
       Every caller that assumed a particular process would own it is a caller you just broke.
    2. **What does each loser do now?**
       "Degrades gracefully" is a claim, not a design — name the path, and check it is reachable.
    3. **Does the guard die with the thing it authorises?**
       Too late strands the resource forever, too early admits a second owner, and both fail silently.
    4. **Which exemptions existed because the old rule held?**
       An exception is an argument and arguments have premises.
       Re-check them; the exemption may have outlived its reason.
    5. **Did the name already mean something?**
       A flag, env var or file you are giving a second job still has its first one, and somebody is depending on it.

    **The general rule:** invariant 9 asks where the problem moved to.
    This one asks **who was standing on the thing you moved.**

## Repo layout

```
outl/
├── CLAUDE.md                  # this file
├── README.md
├── LICENSE                    # MIT
├── Cargo.toml                 # workspace
├── rust-toolchain.toml
├── .claude/                   # agents, commands, hooks, settings
├── .github/workflows/
├── docs/                      # user + contributor reference (see docs/SUMMARY.md)
└── crates/
    ├── outl-core/             # tree CRDT, op log, storage trait
    ├── outl-md/               # parser, sidecar, matching
    ├── outl-actions/          # UI-agnostic workspace ops (shared by every client)
    ├── outl-shortcuts/        # canonical (chord, action) catalog — every client consumes it
    ├── outl-exec/             # code-block runtime (desktop + mobile)
    ├── outl-import/           # adapter-based graph importers (Roam, Logseq, Obsidian + auto-detect)
    ├── outl-config/           # `outl.toml` parsing + schema
    ├── outl-theme/            # palette + presets (TUI + desktop)
    ├── outl-cli/              # `outl` binary
    ├── outl-tui/              # `outl-tui` binary
    ├── outl-mobile/           # Tauri 2 mobile app (iOS first)
    ├── outl-desktop/          # Tauri 2 desktop app (macOS/Linux/Windows)
    ├── outl-tauri-shared/     # shared Tauri backend (command bodies, DTOs, plugin thread) for desktop + mobile
    └── outl-frontend-shared/  # TS+Solid lib (@outl/shared) consumed by mobile + desktop
```

Full `docs/` index lives at [`docs/SUMMARY.md`](docs/SUMMARY.md).
Per-crate context lives in `crates/<name>/CLAUDE.md` — read it before editing that crate.

## Shared logic: `outl-actions`

Every workspace mutation a client needs to perform (edit a block, toggle TODO, indent / outdent, delete, render today's `.md`) lives in **`outl-actions`**, not in the client crate.
The mobile app and the TUI must call the **same** functions for the same semantics; if a new operation needs more than one client, it goes in `outl-actions` before its first use.

The contract is short:

- Functions take `&mut Workspace` and `&HlcGenerator`.
- They route every mutation through `Workspace::apply` (op log stays source of truth).
- They never hold UI state and never touch storage backends directly.

`outl_actions::reminders::next_fire_at` is the sharpest current example of the rule.
It is the **single owner** of "when does this `remind::` fire next", called by the TUI overlay, the desktop panel, the mobile sheet, and every OS notification bridge.
A second opinion in TypeScript or Swift about a schedule is drift that reaches the user at 3am, on one device, before it reaches a test.

See `crates/outl-actions/CLAUDE.md` for the full surface and the "what this crate does NOT own" list.
**If you find yourself writing tree-walking or op-building helpers inside `outl-tui/`, `outl-mobile/`, or any future client, stop and put them in `outl-actions` first.**
The TUI's `outline_ops.rs` is the one deliberate exception (it manipulates an in-flight AST that hasn't been parsed back to a workspace yet — see that file's module doc).

## Shared frontend: `@outl/shared` (`outl-frontend-shared`)

The same "one owner, every client wraps" policy applies on the TS side.
**`crates/outl-frontend-shared/`** is the Solid + TypeScript library every GUI client (`outl-mobile`, `outl-desktop`) consumes for pieces that are pure, stateless, and identical between clients.
Examples: renderers like `<MarkdownInline />`, helpers like `looksLikeOutline` / `detectRefContext`, DTO interfaces, typed `invoke<T>()` wrappers.

Resolution: bun workspaces in the repo root `package.json` deduplicate `solid-js` / `@tauri-apps/api` across the lib + every client.
**Rule of thumb (TS):** before writing a helper in `outl-mobile/src/lib/` or `outl-desktop/src/lib/`, search `crates/outl-frontend-shared/src/`.
**Chrome stays in the client** (Sidebar, Picker, BlockRow, mode-specific keybindings, OS-specific gestures).
See `crates/outl-frontend-shared/CLAUDE.md` for the full policy.

## Reuse-first

Before adding a helper, struct, or constant, **scan the [shared primitives catalog](docs/shared-primitives.md)** and **grep the workspace** for what already does the same thing.
The catalog is one document in four files: the index, plus [core state](docs/primitives-core.md), [markdown pipeline](docs/primitives-markdown.md) and [editing actions](docs/primitives-actions.md).
Grep them together: `grep -n 'symbol' docs/shared-primitives.md docs/primitives-*.md`.
Two implementations of the same logic drift apart over time, and the user is the one who hits the divergence (backlinks, code-block execution, and external-markdown normalization have all been caught mid-PR for exactly this reason).

The rule, past incidents, and what to do when a primitive doesn't exist yet live in [docs/contributing.md → Reuse-first](docs/contributing.md#reuse-first-no-parallel-implementations).

## How we work in this repo

- **Build / test:** `/check` runs fmt + clippy + test on the whole workspace.
  Full dev loop (slash commands, hooks, agents, CI walkthrough) is in [`docs/development.md`](docs/development.md).
- **Specialized agents** (invoke proactively when their `When to use` matches):
  `crdt-invariant-checker`, `paper-verifier`, `markdown-roundtrip-tester`, `refactor-architect`, `doc-keeper`.
  Mandates live under `.claude/agents/`.
- **Documentation discipline.**
  When your PR touches a workflow, slash command, hook, public API, sidecar, op-log format, or shortcut, the matching docs update in the *same* PR.
  Full "if you changed X, update Y" checklist lives in [`docs/contributing.md` → Keep docs in sync](docs/contributing.md#keep-docs-in-sync-with-code).
- **One owner per fact.**
  Tables (shortcuts, CLI subcommands, config keys, op variants) live in `docs/*.md`, and `CLAUDE.md` files link, never duplicate.
  See [`docs/contributing.md` → One owner per fact](docs/contributing.md#one-owner-per-fact--link-dont-duplicate) for the canonical-home map.
- **Markdown style:** semantic line breaks (one sentence per line, no column reflow).
  Full rule in [`docs/contributing.md` → Markdown / documentation style](docs/contributing.md#markdown--documentation-style).
- **File size discipline.**
  The `file-size-guard.sh` PostToolUse hook nudges at 600 lines and stops at 900.
  When it fires, invoke the `refactor-architect` agent.
- **`cargo doc` is part of CI** with `RUSTDOCFLAGS=-D warnings`.
  Run `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` before reporting "done" on any patch that adds or changes module-level doc comments (`//!` blocks) — `/check` does not include this today.

## Decisions you don't get to revisit

These were settled before code was written.
If you think one is wrong, **stop and ask the user** before changing.
Don't unilaterally pivot.

| Decision | Why |
|----------|-----|
| `ULID` for IDs | Lexicographically sortable, 128 bits, no central server needed |
| `uhlc` for time | HLC with actor tiebreak is total order without coordination |
| Yrs for block text | Battle-tested CRDT for strings, lets us focus on the tree |
| `comrak` for markdown | CommonMark-compliant, fast, customizable |
| `iroh` as the default sync transport | QUIC + hole punching + relay, no central server for data; iroh is `[sync] transport` default |
| `file` transport as the explicit opt-out | `transport = "file"` for iCloud Drive / shared FS users; folder is user-chosen — iCloud is one option, not a dependency |
| Tauri 2 for mobile (replaces earlier uniffi plan) | Single Rust surface across TUI + mobile via `outl-actions`, Solid + Tailwind frontend, ObjC bridge only for iCloud watcher |
| Tauri for desktop (shipping today) | Rust core reuse, smaller than Electron. macOS / Linux / Windows; Solid frontend shares `@outl/shared` with mobile |
| `outl-shortcuts` is the single (chord → action) catalog | Two parallel implementations is the bug we paid to remove (TUI used to define bindings in `input/`, desktop wired its own `KeyboardEvent` handlers — `Cmd+P` and `Ctrl+P` drifted within a sprint). Adding a key on any client without going through `defaults.rs` puts that drift back. See `outl-shortcuts/CLAUDE.md` |
| One `ops-<actor>.jsonl` per device, never shared | Any file transport (iCloud, Syncthing, shared FS) is last-write-wins per file; per-actor files turn that into a non-issue; iroh ships ops directly |
| MIT license | Simple, widely understood, no patent grant baggage |
| `outl.app` domain owned | Use for docs/landing later |
| Repo at `github.com/outlmd/outl` | Moved off the personal profile (`avelino/outl`) into the `outlmd` org. GitHub redirects the old URL and the old `git remote`, but a **Homebrew tap name is not a redirect** and neither is a `github:<owner>/<repo>` plugin source — both are re-resolved by name, so both are spelled `outlmd/outl` everywhere. CI is owner-agnostic (`$GITHUB_REPOSITORY`) |
| `[workspace.package].version` in root `Cargo.toml` is the **single source of truth** | Crate manifests inherit via `version.workspace = true`. `tauri.conf.json` deliberately omits `version`; CI reads `Cargo.toml` and injects it into `cargo tauri ios build` via `--config` (Tauri's iOS path does NOT fall back to `Cargo.toml` on its own — it defaults to `1.0.0`). Bumping the workspace bumps everything. See `crates/outl-mobile/CLAUDE.md` → "Versioning + TestFlight release" before changing release/CI plumbing |

## What you're NOT building yet

Don't add code for these unless explicitly asked:

- Plugin system (`rhai`)
- `ChronDbStorage` backend (issue #1, tracked publicly)
- ~~Android mobile build~~ — **this shipped**, and this line said otherwise long enough to be worth a warning.
  `release.yml`'s `build_android` job signs an arm64 APK on every release; `gen/android/` holds a hand-written `MainActivity.kt`; `android_jni.rs` primes rustls-platform-verifier + `ndk_context` so iroh's first QUIC connection doesn't `SIGABRT`.
  The old rationale ("needs an `NSMetadataQuery` equivalent") named the wrong blocker.
  Mobile storage is a local folder synced by iroh with **no** filesystem watcher in the Rust path, and the real platform work was the JNI TLS/DNS bootstrap.
  What is still open is release plumbing, not the port: see [`docs/android-platform.md`](docs/android-platform.md)
- App-closed reminder delivery — `remind::` fires today only while the app runs.
  The iOS `UNCalendarNotificationTrigger` pre-registration, the macOS launch agent, the Windows scheduled toast and the systemd user timer are all follow-ups to issue #63; see [`docs/reminders.md`](docs/reminders.md) → Background delivery
- Per-page op log shards ([`docs/sync.md` Part 2 — Per-page op log shards](docs/sync.md#per-page-op-log-shards-for-10k-pages); only land it when the single-jsonl-per-device layout hits the 10k-page wall)
- Character cursor inside the selected block in desktop Normal mode.
  TUI-only today.
  The desktop's vim mode has only a selected block id, so the char-level vim ops `x`/`X`/`D`/`C`/`s`/`r`/`f`/`F`/`~`/`e` surface a status-line nudge instead of firing.
  See `outl-desktop/CLAUDE.md` → "Vim parity".

## Coding conventions

- `rustfmt` default config, no overrides.
- `clippy -- -D warnings` blocks CI.
- No `unwrap()` in non-test code.
  Use `expect("explicit reason")` or propagate.
- `thiserror` in libs (`outl-core`, `outl-md`), `anyhow` at boundaries (`outl-cli`, `outl-tui`).
- No `unsafe` in `outl-core` without documented justification.
- Variable names, function names, doc comments: **English** (global audience).
- User-facing strings (CLI help, TUI labels): English for now (i18n later).
- **Conventional Commits are load-bearing.**
  Use `feat:`, `fix:`, `perf:`, `docs:`, `refactor:`, `chore:`, `test:`, `build:`, `ci:` on every commit (and on PR merge commits).
  The Mobile pipeline generates TestFlight release notes by feeding the commit log since the last tag into `conventional-changelog-cli`.
  Commits without a prefix all fall into a single "Other changes" bucket on TestFlight, so the user loses the per-build context.
  If a commit doesn't fit a type, prefer `chore:` over no prefix.

Full review policy (Rust quality, hot paths, architecture, simplicity, testing) lives in [`docs/contributing.md`](docs/contributing.md).

## Anti-patterns (don't do)

- ❌ Calling `.unwrap()` to get out of error handling
- ❌ Writing IDs into the `.md` file ("just for now")
- ❌ Storing op log fields outside the `Op` variant (breaks undo)
- ❌ Overwriting a `.md` because its sidecar hash matches (invariant 8 — that proves outl wrote it last, not that the op log holds it)
- ❌ Rewriting a sidecar to agree with content you did not emit ops for (this is what *produces* the state invariant 8 defends against)
- ❌ Fixing one direction of a `.md` ↔ tree divergence without stating what happens in the other
- ❌ Comparing HLCs without actor tiebreak
- ❌ Treating `Delete` as physical removal
- ❌ Skipping tests because "the algorithm is the same as the paper"
- ❌ Reintroducing SQLite / rusqlite / any binary log format — cross-device sync depends on per-actor append-only files
- ❌ Using `id::` Logseq-style metadata anywhere
- ❌ Marking work "done" without `/check` passing
- ❌ Re-introducing `"version"` in `crates/outl-mobile/src-tauri/tauri.conf.json` — Tauri must keep falling back to `Cargo.toml` (see "Versioning + TestFlight release" in `crates/outl-mobile/CLAUDE.md`)
- ❌ Adding a helper that re-implements something already in `outl-core` / `outl-md` / `outl-actions` (see [Reuse-first](docs/contributing.md#reuse-first-no-parallel-implementations)).
  The fix is to wrap the upstream API, not to write a parallel one.

## When in doubt

1. Read the relevant `docs/*.md`.
2. Read the per-crate `CLAUDE.md`.
3. Read the paper for sync stuff: <https://martin.kleppmann.com/papers/move-op.pdf>.
4. Ask the user.
   The user is `Avelino`, comfortable in Rust/Clojure/Python/Go, prefers direct pt-BR communication.
