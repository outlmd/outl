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
   It proves outl wrote those bytes last, never that the log holds them.
   So never overwrite a `.md` on the hash gate alone: ask `outl_md::unlogged::sidecar_can_vouch_for` (against the **sidecar's** blocks, never a fresh render), and a refusal has to reach the user.
   Bought by 233 pages / 1,426 lines of real work deleted by `doctor --repair`.
   **The general rule:** when you fix one direction of a `.md` ↔ tree divergence, state what happens in the other *before* merging — reconciliation bugs come in mirrored pairs, and the pair that deletes is never the one being reported.

9. **When state crosses a boundary, say what its new home requires.**
   Moving state out of a file, out of the op log, or into a process global hands its problems to a place with different rules.
   Answer four questions about the destination: **who can write it, who can read it, how does a test get its own copy, and what cleans it up?**
   A cleanup answer is only as good as what it refuses.
   **The general rule:** a fix relocates a problem far more often than it removes one, so after "did I fix it?" comes **"where does the problem live now?"**

10. **When you change who decides, enumerate who was standing on the old decision.**
    A policy has beneficiaries that never had to declare themselves, because under the old rule they always won.
    Ask: who consumed the winner, what does each loser do now (name the path, check it is reachable), does the guard die with the thing it authorises, which exemptions existed only because the old rule held, and did the name already mean something?

11. **A cost only argues against a design if that design causes it.**
    Attribute the expense before you let it decide.
    Would the alternative pay it too?
    Does the cost track the feature or the lifetime?
    What is the smallest thing that avoids it?
    Did it already exist?
    A pre-existing cost you discovered while investigating deserves its own issue, not a vote on the change in front of you.

12. **A capability without a per-client verdict is a gap nobody recorded.**
    Not every client does everything; what is not fine is that the *difference* has no owner, so it is discovered by a user pressing a key that does nothing.
    `outl_shortcuts::support` and `capability_support` are exhaustive `match`es, so a new variant does not compile until all three clients declare their verdict, and `docs/client-parity.md` is generated from them.
    The nudge wording belongs in the catalog, never in a client.

13. **A colour token has exactly one meaning, on every client.**
    Colours come from `outl_theme::Palette` through `applyPaletteToRoot`; a hex literal in a client stylesheet is a second definition, and a token name that resolves to two things gets picked by whatever signal happens to be wired to it.
    The one exception is a client's `@theme` boot block, which exists so the first painted frame is branded.

14. **A filter that cannot be negated is half a filter, and the negation is never a second implementation.**
    `not-<key>` parses `<key>` and wraps it in `Filter::Not`, so every predicate is negatable the moment it exists and the two sides cannot disagree.
    **Refuse a hand-written `NotFoo` variant in review.**
    A negative that drifts deletes rows the user cannot see are missing, and a value that can only match nothing (an empty tag, a dangling `prop: k:`) is a parse error for the same reason.

15. **Being fast is the product, so a repeated question is answered once and a cheap answer is never bought with an expensive read.**
    Speed is not a bar applied after a feature works; it is the feature, and it fails in a shape no test catches — every function reasonable on its own, everything correct, and the app cooks.
    Bought by a sync census that was already the *cheap* read, and still `O(total ops)` per call: one exchange asked six times with two full `JsonlStorage::open`s on top.
    That is ~710 ms of CPU per peer every 8 s to move zero ops on a converged device, 131% of a core at peak, and a window that stopped taking keystrokes.
    No test failed.

    Four questions, and the first was skipped three times:

    1. **Who else is asking this, in the same breath?**
       Grep the read's callers, not just the one you are writing — six call sites with one answer is one read, passed along.
    2. **Is anyone *looking* at this answer?**
       An expensive column computed behind a closed panel is waste, and a polling loop is where waste multiplies.
    3. **What does it cost at the size a real graph reaches?**
       Not the fixture's nine ops: 750k ops, 170 MB of sidecar, 23 actors.
       An `O(total ops)` read gets slower every day the user keeps using the app.
    4. **Does the cheap path stay cheap on the branch that does something?**
       An early return is not a budget.

    **Measure on a real graph, on the device** — a process `sample` found this while the whole suite stayed green — and **make the net structural**, because a timing test is flaky and "the sidecar came back" is not.
    The pre-existing fast-path test passed throughout: it only ever reached the early return.

    A performance fix owes **three** tests, and none of them covers another.
    *Does it still do the right thing*: equivalence against the read it replaced, over real op bodies.
    *Is it still cheap*: structural, on every branch, not just the early return.
    *Is it cheap once*: read the call sites, because a type cannot bound how many times a caller asks.

**Invariants 8-15 are questions to ask before merging, and each was bought by a measured incident.**
The reasoning, the rejected alternatives and what each fix made worse live in [`docs/invariants.md`](docs/invariants.md).
Read the one that covers your change before you touch it; the rule above is the summary, not the argument.
Performance specifically — the incident behind 15, the per-client checklist and how to measure: [`docs/performance.md`](docs/performance.md).


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
├── tools/
│   └── discussions-discord/   # CI-only: mirrors GitHub Discussions into Discord (see docs/discussions-discord.md)
└── crates/
    ├── outl-core/             # tree CRDT, op log, storage trait
    ├── outl-md/               # parser, sidecar, matching
    ├── outl-ws/               # workspace bootstrap: lock/actor protocol, shard registration, slug repair
    ├── outl-actions/          # UI-agnostic workspace ops (shared by every client)
    ├── outl-shortcuts/        # canonical (chord, action) catalog + per-client support matrix
    ├── outl-exec/             # code-block runtime (desktop + mobile)
    ├── outl-plugins/          # plugin runtime, manifest, permission gating, host API
    ├── outl-sync-iroh/        # iroh QUIC transport: endpoint, pairing, op-log delta sync
    ├── outl-import/           # adapter-based graph importers (Roam, Logseq, Logseq DB EDN export, Obsidian + auto-detect)
    ├── outl-config/           # `outl.toml` parsing + schema
    ├── outl-theme/            # palette + the theme file format (`themes/*.toml`, built-in and the user's)
    ├── outl-telemetry/        # crash-report gate + payload scrubber (the one owner of what leaves the machine)
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

- **Build / test:** `/check` runs fmt + clippy + test + doc on the Rust workspace, then vitest + `tsc --noEmit` across every bun workspace package.
  Both halves are required before "done": roughly a third of each GUI client is TypeScript, and two invariants (12 and 13) are enforced only by TS parity tests.
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
  The `file-size-guard.sh` PostToolUse hook nudges at 600 lines and stops at 900, for `.rs`, `.ts` and `.tsx` alike.
  When it fires, invoke the `refactor-architect` agent.
  A hook only fires when Claude Code is the editor, so CI enforces the same limit for everyone as a ratchet: [`docs/development.md` → Hooks](docs/development.md#hooks-run-automatically) is the owner of how that works.
- **`cargo doc` is part of CI** with `RUSTDOCFLAGS=-D warnings`, and is step 4 of `/check`.
  It breaks on intra-doc links to private items — ``[`Foo`]`` where `Foo` is `pub(crate)`; drop the brackets to `` `Foo` ``.

## Decisions you don't get to revisit

These were settled before code was written.
If you think one is wrong, **stop and ask the user** before changing.
Don't unilaterally pivot.

| Decision | Why |
|----------|-----|
| `ULID` for IDs | Lexicographically sortable, 128 bits, no central server needed |
| Hybrid logical clocks for time | Actor tiebreak gives total order without coordination. **Hand-rolled, not `uhlc`** — that crate is in no manifest, so its drift bound was never inherited ([`architecture.md`](docs/architecture.md) §8) |
| Yrs for block text | Battle-tested CRDT for strings, lets us focus on the tree |
| `comrak` for markdown | CommonMark-compliant, fast, customizable |
| `iroh` as the default sync transport | QUIC + hole punching + relay, no central server for data; iroh is `[sync] transport` default |
| `file` transport as the explicit opt-out | `transport = "file"` for iCloud Drive / shared FS users; folder is user-chosen — iCloud is one option, not a dependency |
| Tauri 2 for mobile (replaces earlier uniffi plan) | Single Rust surface across TUI + mobile via `outl-actions`, Solid + Tailwind frontend, ObjC bridge only for iCloud watcher |
| Tauri for desktop (shipping today) | Rust core reuse, smaller than Electron. macOS / Linux / Windows; Solid frontend shares `@outl/shared` with mobile |
| `outl-shortcuts` is the single (chord → action) catalog | Two parallel implementations is the bug we paid to remove (TUI used to define bindings in `input/`, desktop wired its own `KeyboardEvent` handlers — `Cmd+P` and `Ctrl+P` drifted within a sprint). Adding a key on any client without going through `defaults.rs` puts that drift back. See `outl-shortcuts/CLAUDE.md`. **Only the desktop resolves through `lookup()` today** — the TUI still dispatches Normal-mode keys from its own `match` in `input/normal.rs`, and mobile consumes neither; `docs/shortcuts.md` claimed otherwise for months. Finishing that migration is open work, not a settled decision |
| `wrappers/catalog.rs` is the single declaration of the **Tauri command surface** | The bodies were shared; the wrappers were not, and 3,033 lines of hand-written shim diverged by omission (`history`: 183 lines vs 22). A client takes a whole module or records the gap. `tests/command_parity.rs` catches both a skipped module and a generated command missing from `generate_handler!`. See `outl-tauri-shared/CLAUDE.md` |
| `outl_theme::spec` is the single owner of **what a theme file is** | Built-in themes are `themes/*.toml` compiled into the binary with `include_str!`; a user's are the same format from `~/.config/outl/themes/`, parsed by the same code. Rust built-ins plus a TOML parser for the user would be two formats, two validators, and an `extends = "dracula"` with no `fn` to reach. The directory is a **parameter** (`Registry::with_user_dir`), never resolved from `XDG_CONFIG_HOME` in the crate — invariant 9's third question. Names derive from the filename and the section, so `name` is an error rather than an override |
| `outl_md::unlogged::sidecar_can_vouch_for` is the single owner of **can this sidecar vouch for the `.md` on disk** | `sidecar_can_answer` is one arm of it and answers a narrower question. It is not the whole overwrite verdict: a writer still refuses on a non-empty `content_lines_missing_from` line and on a frontmatter loss, phrased by `unlogged_content_error` / `frontmatter_loss_error`. Three callers asked it; only `outl reconcile`'s `collect_ahead` paired it with "is anything actually on disk", and the two that did not froze every empty page a peer wrote into ([#332](https://github.com/outlmd/outl/issues/332)). See [invariant 8](#critical-invariants-never-violate) |
| `outl_actions::commit_page` owns **what happens around a page mutation** | The five-step sequence had one implementation, behind a trait wanting `&Mutex<Option<Workspace>>` — unreachable from the TUI and CLI, which each re-derived a subset. `AppHost` stayed put on purpose ([#264](https://github.com/outlmd/outl/issues/264)) |
| `Filter::Not(Box<Filter>)` is the single owner of **negation** in the query DSL | `not-<key>` parses `<key>` and wraps it, so every filter is negatable the moment it exists and the two sides cannot disagree. The first implementation hand-wrote `NotTag` / `NotProp`: two of six filters negatable, each with its own copy of the predicate. See [invariant 14](#critical-invariants-never-violate) |
| `scripts/release_assets.py` is the single owner of **what a release publishes** | One entry per build-matrix leg: its assets, whether it is `required`, which package manager points at it. Six places used to each hold part of that fact and none could see the others — three build matrices, `update_tap`'s loops, the Homebrew formula, the cask, the Scoop manifest — and every disagreement was silent, because a `sed` that matches no line exits 0 and the bump ends on `git diff --quiet`. It is also what decides whether a release publishes: `publish_release` reads the draft's assets against it rather than reading job results, so one architecture's bundler cannot hold back the other platforms and a gap cannot pass unmentioned. `scripts/check-release-assets.py --manifest` (in `hygiene.yml`) is what keeps the other six honest |
| `outl_shortcuts::support` is the single owner of **which client performs which action** | An exhaustive `match`, so a new `Action` variant does not compile until all three clients declare what they do with it. The lesser states carry the sentence shown to the user, so a client cannot invent its own wording. See [invariant 12](#critical-invariants-never-violate) |
| `outl_tauri_shared::FirstPaintGate` owns **what waits for the first frame** | Sync, the filesystem watcher, the plugin host, the backup pass, the orphan sweep and the snapshot refresh all used to start the moment the opener published, which on a phone is the moment the webview is laying out the first screen. The gate releases them on the frontend's `first_content` mark, and its **deadline is not optional**: the painted signal comes over IPC, so a webview that fails to report would otherwise be an app that syncs with nobody, silently. See [`docs/boot-performance.md`](docs/boot-performance.md) |
| `outl_tauri_shared::WorkspaceReady` owns **"is the workspace open yet"** | A latch with a blocking wait, replacing a 500 ms desktop poll and a 50 × 200 ms mobile retry. Neither could resolve faster than its own interval. `false` means "not yet, ask again" and is never a failure, because a cold 700k-op replay legitimately takes seconds |
| `outl_tauri_shared::commands::boot::BOOT_MARKS` owns **the launch milestone names** | `html_parsed` / `theme_applied` / `first_content` / `interactive`. A wire contract: the log, a CI budget checker and the simulator job all match on the string, so a client spelling `firstContent` is not a type error anywhere, just a mark that never appears. `@outl/shared/boot`'s copy is a declared mirror |
| `applyPaletteToRoot` owns **the first-frame theme cache** | The palette arrives two IPC round-trips after the document parses, so a light-theme user saw a dark flash on every launch. The last installed palette is replayed by an inline `<head>` script; the only writer of that cache sits beside the only writer of the tokens, so the two cannot disagree about what the theme is |
| One `ops-<actor>.jsonl` per device, never shared | Any file transport (iCloud, Syncthing, shared FS) is last-write-wins per file; per-actor files turn that into a non-issue; iroh ships ops directly |
| MIT license | Simple, widely understood, no patent grant baggage |
| `outl.app` domain owned | Use for docs/landing later |
| Repo at `github.com/outlmd/outl` | Moved off the personal profile (`avelino/outl`) into the `outlmd` org. GitHub redirects the old URL and the old `git remote`, but a **Homebrew tap name is not a redirect** and neither is a `github:<owner>/<repo>` plugin source — both are re-resolved by name, so both are spelled `outlmd/outl` everywhere. CI is owner-agnostic (`$GITHUB_REPOSITORY`) |
| `[workspace.package].version` in root `Cargo.toml` is the **single source of truth** | Crate manifests inherit via `version.workspace = true`. `tauri.conf.json` deliberately omits `version`; CI reads `Cargo.toml` and injects it into `cargo tauri ios build` via `--config` (Tauri's iOS path does NOT fall back to `Cargo.toml` on its own — it defaults to `1.0.0`). Bumping the workspace bumps everything. See `crates/outl-mobile/CLAUDE.md` → "Versioning + TestFlight release" before changing release/CI plumbing |

## What you're NOT building yet

Don't add code for these unless explicitly asked:

- ~~Plugin system (`rhai`)~~ — **this shipped**, as JavaScript on the Boa interpreter, not `rhai`.
  `crates/outl-plugins` is a workspace member with `default = ["js"]`; [RFC 0025](docs/rfcs/0025-plugin-system.md) records why Boa and why Rhai was rejected, and [`docs/plugins.md`](docs/plugins.md) is the user-facing page.
- `ChronDbStorage` backend (issue #1, tracked publicly)
- ~~Android mobile build~~ — **this shipped**, and this line said otherwise long enough to be worth a warning.
  `release.yml`'s `build_android` job signs an arm64 APK on every release; `gen/android/` holds a hand-written `MainActivity.kt`; `android_jni.rs` primes rustls-platform-verifier + `ndk_context` so iroh's first QUIC connection doesn't `SIGABRT`.
  The old rationale ("needs an `NSMetadataQuery` equivalent") named the wrong blocker.
  Mobile storage is a local folder synced by iroh with **no** filesystem watcher in the Rust path, and the real platform work was the JNI TLS/DNS bootstrap.
  What is still open is release plumbing, not the port: see [`docs/android-platform.md`](docs/android-platform.md)
- App-closed reminder delivery — `remind::` fires today only while the app runs.
  The iOS `UNCalendarNotificationTrigger` pre-registration, the macOS launch agent, the Windows scheduled toast and the systemd user timer are all follow-ups to issue #63; see [`docs/reminders.md`](docs/reminders.md) → Background delivery
- An automatic switch to per-page op log shards.
  The layout itself **shipped** (RFC 0137 Phase B): `PageScope::PerPage(slug)` writes `ops/<actor>/<slug>.jsonl`, `outl init --scope=per-page` starts a workspace on it and `outl migrate-to-per-page-ops` converts one.
  What does not exist is a trigger — nothing counts pages and nothing flips at the 10k wall, so the layout is opt-in per workspace.
  Tracked with the rest of the op-log growth work in [#110](https://github.com/outlmd/outl/issues/110)
- Character cursor inside the selected block in desktop Normal mode.
  TUI-only today.
  The desktop's vim mode has only a selected block id, so the char-level vim ops `x`/`X`/`D`/`C`/`s`/`r`/`f`/`F`/`~`/`e` surface a status-line nudge instead of firing.
  See `docs/desktop-keybindings.md` → "Vim parity".

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
- ❌ Refusing a `.md` on the strength of `sidecar_can_answer` alone (invariant 8 — that answers "can these blocks say what the log held", not "is anything on this disk at risk").
  Ask `sidecar_can_vouch_for`: a page holding only bare bullets has nothing to lose, and refusing it froze every fresh journal day and every unfilled `[[link]]` page, silently.
- ❌ Fixing one direction of a `.md` ↔ tree divergence without stating what happens in the other
- ❌ Rejecting a design over a cost the alternative pays too, or over a cost that was already there before your change (invariant 11 — attribute the cost before you let it decide)
- ❌ Deriving a namespace from the slug, or `slugify` keeping `/` (it's in `title::`)
- ❌ Comparing HLCs without actor tiebreak
- ❌ Treating `Delete` as physical removal
- ❌ Skipping tests because "the algorithm is the same as the paper"
- ❌ Reintroducing SQLite / rusqlite / any binary log format — cross-device sync depends on per-actor append-only files
- ❌ Using `id::` Logseq-style metadata anywhere
- ❌ Adding an `Action`, or any cross-client capability, without recording which clients lack it (invariant 12)
- ❌ Adding a user-writable filter without its negative, or hand-writing a `NotFoo` variant instead of reusing `Filter::Not` (invariant 14).
  The negation is `!` over the positive's matcher; a second matcher drifts, and a negative that drifts deletes results the user cannot see are missing.
- ❌ Accepting a filter value that can only ever match nothing (an empty tag, an empty `text:`, a dangling `prop: k:`).
  Harmless on the positive side, silent over-exclusion on the negative side — parse-error it.
- ❌ Writing the "this isn't available here" wording in a client instead of in the catalog
- ❌ Adding, renaming or removing a release artifact without its entry in `scripts/release_assets.py`.
  A leg the table does not list is gated by nothing; a name the table still spells the old way blocks the publish over a bookkeeping slip. `hygiene.yml` catches both on the PR.
- ❌ Gating a release on `needs.<build job>.result`.
  That made every build job a single point of failure for every other job's assets; the question is which assets are on the draft, and that is what `publish_release` asks.
- ❌ Putting work the first frame does not need in front of it, or releasing that work on anything other than the frontend's `first_content` mark.
  A background thread is not invisible: the premise is the *device*, and the deferred set reads the same disk and takes the same workspace lock the first paint needs.
- ❌ Adding a `FirstPaintGate` without arming its deadline, or a readiness latch a client polls anyway.
  "Degrades gracefully" is a claim until the path is written down and reachable (invariant 10 item 2).
- ❌ Asking for the same `O(total ops)` answer twice in one code path, or recomputing a read a caller one frame up already has (invariant 15).
  Six call sites with one answer is one read, passed along: `Census` carries the clock, the selection and the offsets out of a single pass.
- ❌ Reaching for `JsonlStorage::open` to read a handful of ops.
  That is a full load of both index sidecars for every actor; the census already holds the offsets, and `read_ops_at_offsets` spends them.
- ❌ Computing an expensive field for a caller that does not render it, above all on a polling path.
  `peer_status(with_pending)` is the shape: the status dot asks for reachability, the panel asks for the count.
- ❌ Shipping a fast path whose test only covers the early return.
  A guard for "nothing to do" says nothing about the branch that does something, which is exactly where the cost hides.
- ❌ Marking work "done" without `/check` passing
- ❌ Re-introducing `"version"` in `crates/outl-mobile/src-tauri/tauri.conf.json` — Tauri must keep falling back to `Cargo.toml` (see `outl-mobile/CLAUDE.md`)
- ❌ Hand-writing a `#[tauri::command]` wrapper in a client crate, or taking only part of a `*_commands!` module.
  The surface is declared once in `wrappers/catalog.rs`; a client takes a whole module or adds a `DECLARED_GAPS` row with a reason.
- ❌ Giving a shared command body a `&str` parameter — Tauri hands the wrapper an owned `String`.
- ❌ Calling `apply_page_md_with_sidecar_guarded` for a page you just mutated.
  That is step 5 of five: use `outl_actions::commit_page`, or comment which steps you skip and why.
- ❌ Adding a wire DTO field, an **enum variant**, or an `export interface` / `export type` without a pin in `outl-tauri-shared/tests/` (`wire_types.rs` for struct key sets, `wire_enums.rs` for variant sets, `wire_mirrors.rs` for mirrors living outside `@outl/shared/api/types.ts`).
  The Rust↔TS mirror is hand-written on purpose; the pin is what makes that safe.
- ❌ Adding a field to `Workspace` without asking which of its four owners it belongs to: the document (`tree`/`log`/`content`), `StorageRouter`, `SnapshotPolicy`, or the batch buffer.
- ❌ Destructuring props in a Solid component — reactivity rides the getter, so a destructured prop freezes at first render.
- ❌ Adding a helper that re-implements something already in `outl-core` / `outl-md` / `outl-actions` (see [Reuse-first](docs/contributing.md#reuse-first-no-parallel-implementations)).
  The fix is to wrap the upstream API, not to write a parallel one.
- ❌ A hex colour in a client stylesheet, or a `--color-outl-*` token with no `Palette` field behind it (invariant 13).
  [`DESIGN.md`](DESIGN.md) is the specification — roles, theming, spacing, components, and the live exceptions named so none is cited as precedent.
  Its behavioural counterpart is [`UX.md`](UX.md): user and world models, interaction patterns, the voice rules a nudge has to pass, and the domain glossary.
- ❌ Reintroducing a second token namespace "just for this client"
- ❌ Putting the page in a load state for work that is not the page.
  Anything landing after the first paint (backlinks, nested pages, a query) shows its own placeholder in its own section, and never unmounts, disables or re-keys the editor.
  [`UX.md` → Never block a keystroke](UX.md#2-never-block-a-keystroke) owns the rule and the regression that bought it.

## When in doubt

1. Read the relevant `docs/*.md`.
   Anything visual — a colour, a token, spacing, a component — starts at [`DESIGN.md`](DESIGN.md).
   Anything behavioural — what an interaction does, who it is for, what we say when a client cannot deliver it — starts at [`UX.md`](UX.md).
2. Read the per-crate `CLAUDE.md`.
3. Read the paper for sync stuff: <https://martin.kleppmann.com/papers/move-op.pdf>.
4. Ask the user.
   The user is `Avelino`, comfortable in Rust/Clojure/Python/Go, prefers direct pt-BR communication.
