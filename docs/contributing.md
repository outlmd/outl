# Contributing & code review

This is the canonical guide for **what review measures your PR against** — the rules of the game, why each rule exists, and what is explicitly *not* a reason to block your PR.

If you're looking for **how to set up, run, test, and debug the project**, that's the [Development guide](development.md).
The two pages are deliberately split: that one is workflow, this one is policy.

The [root `CONTRIBUTING.md`](https://github.com/avelino/outl/blob/main/CONTRIBUTING.md) on GitHub is a short pointer (clone, build, commit format, license) that links into both.
Everything substantive lives across these two pages.

We want outl to be a project where you can show up, read this page, and know exactly what you're walking into.
No tribal knowledge, no hidden quality bar.

The same priorities are encoded for automated review in [`.github/copilot-instructions.md`](https://github.com/avelino/outl/blob/main/.github/copilot-instructions.md) and [`.pr_agent.toml`](https://github.com/avelino/outl/blob/main/.pr_agent.toml).
If you ever feel a reviewer comment came out of nowhere, it almost certainly traces back to this page or one of those files.

### The automated reviewers

Three bots review every PR.
Where we configure one, we configure it in a file in the repo and never in a web UI, so what a reviewer checks is itself reviewable in a diff.

| Reviewer | Configured by | Reads standards from |
|---|---|---|
| GitHub Copilot | [`.github/copilot-instructions.md`](https://github.com/avelino/outl/blob/main/.github/copilot-instructions.md) + [`.github/instructions/*.instructions.md`](https://github.com/avelino/outl/tree/main/.github/instructions) | those files, path-scoped by their `applyTo` frontmatter |
| Qodo | [`.pr_agent.toml`](https://github.com/avelino/outl/blob/main/.pr_agent.toml) | root and per-crate `CLAUDE.md`, imported automatically and scoped to the folder holding each file |
| CodeRabbit | (defaults) | the diff |

Two consequences worth knowing before you wonder why a bot did or didn't say something.

**Qodo reads `.pr_agent.toml` from the default branch**, so editing it in your branch does not change how your branch is reviewed — the change applies to PRs opened after it merges.

**A per-crate `CLAUDE.md` is a scoped rule set, for humans and for Qodo alike.**
Qodo's rule import scopes each file's rules to the directory containing it at any depth, which is why `crates/outl-md/CLAUDE.md` earns stricter review inside that crate and is silent elsewhere.
It is also why this repo has no `best_practices.md`: that would be a second copy of rules `CLAUDE.md` already owns, and the copy is the one that goes stale.
The gap in that mechanism is `.github/instructions/*.instructions.md` — Copilot reads those, Qodo does not, so anything that must reach both reviewers belongs in a `CLAUDE.md`.

---

## Philosophy

outl is a sync engine first and a notes app second.
The CRDT layer underneath has to be **correct**, because the cost of a sync bug is silently-corrupted user data — exactly the failure mode Roam and Logseq ship and exactly the one we exist to not repeat.

That shapes how we review.
We are strict about the things that touch correctness, scalability, and the public contract.
We are deliberately relaxed about the things that don't.

Concretely:

- **Correctness > cleverness.** The CRDT follows [Kleppmann et al. 2022](https://martin.kleppmann.com/papers/move-op.pdf) literally.
  A "smarter" version is a regression.
- **Simple > abstract.** We invoke the Rule of Three before introducing a trait or generic.
  One implementation gets a concrete type.
- **Real problems > hypothetical ones.** A refactor needs to unblock something or pay down a named debt. "Cleaner code" alone is not a merge reason.
- **The user's `.md` is sacred.** It belongs to the user.
  We never write metadata into it, even when it would make our life easier.

If you read this and think "they're going to be brutal in review" — not the goal.
The goal is for you to read this *before* writing the patch, so the review goes "ship it" instead of "back to draft".

---

## Before opening a PR

Reviewers (human and automated) check the PR description **before** the diff.
If the description doesn't answer these, the PR gets a top-level comment requesting changes and the line-level review is deferred.

1. **Link an issue.** `Closes #N`, `Fixes #N`, or `Related to #N`.
   - For typo fixes, doc-only changes, or dependency bumps with a changelog link, you can skip this.
   - For everything else, an issue is the place to debate scope and approach.
     PRs are the place to debate implementation.
2. **State the problem in plain language.** One paragraph.
   The user-facing problem first, then the technical approach.
3. **For a refactor, answer "why now?".** "The code is cleaner" is not enough — name the concrete thing this unblocks or the debt it pays down.
4. **For a bug fix, describe the bug.** Steps to reproduce, observed behaviour, expected behaviour.
   Ideally a failing test that the patch turns green.
5. **For a feature, point at an approved issue.** If a feature isn't in an accepted issue, open the issue first.

The template at [`.github/PULL_REQUEST_TEMPLATE.md`](https://github.com/avelino/outl/blob/main/.github/PULL_REQUEST_TEMPLATE.md) walks you through this.
Use it.

---

## How a change lands

Four steps, and the split between them is deliberate.

1. **Issue — the problem only.**
   Expose the bug or request the capability.
   **Do not put a proposed solution in the issue body.**
   A solution written into the body stops reading as a proposal and starts reading as the plan, before anyone has argued with it — and six months later nobody can tell what was actually wrong from what someone guessed the fix would be.
2. **Discussion — in the comments.**
   Alternatives, trade-offs, objections, benchmarks all belong in the thread.
   The body stays a problem statement so a newcomer can read what broke without reading how three people thought about fixing it.
3. **PR — implementation *and* its RFC together.**
   One PR, not two.
   An RFC-only PR gets accepted and then the implementation drifts from it; an implementation-only PR gets merged and the reasoning survives only in the diff.
   Whether you need an RFC at all is answered in [docs/rfcs/README.md](rfcs/README.md#do-i-need-an-rfc).
   Roughly: yes if it touches an invariant, a data format, the CRDT, sync, or a projection path, or carries a trade-off someone might want to reverse.
   No for a localized fix — that belongs in `CHANGELOG.md`.
4. **Review and merge.**
   The reviewer checks the RFC against the diff, not only the diff.
   A diff that contradicts its own RFC is a blocker, the same as one that contradicts an invariant.

### RFCs, invariants and tests are one mechanism

An RFC on its own prevents nothing — nobody reads `docs/rfcs/` while editing `reconcile.rs`.
So a change that can cost a user data lands three things together:

| Layer | Where | Role |
|---|---|---|
| Reasoning | `docs/rfcs/NNNN-*.md` | Why the rule exists, what was rejected, what got worse |
| Rule | root or per-crate `CLAUDE.md` | Read on every edit to that crate — the enforcing surface |
| Proof | a named test | Fails mechanically when someone reverts the behaviour |

The `CLAUDE.md` entry links to the RFC; the RFC names the tests in its **Guarded by** row.
A rule with no RFC has no rationale and gets argued away in review.
An RFC with no `CLAUDE.md` entry is never read at the moment it matters.
Either one without a test is a comment.

**Changing behaviour an RFC pinned means updating that RFC in the same PR** — amend it, or supersede it and mark the old one `Superseded by RFC NNNN`.
[RFC 0210](rfcs/0210-md-content-outside-op-log.md) exists because #166 shipped a correct fix for one direction of a divergence and nobody wrote down what happened in the other; the mirrored case deleted user content for months.
That is why the template's **The opposite direction** section is required and not optional.

---

## Non-negotiable invariants

These are blockers in review.
A PR that violates one will not merge regardless of how clean the code looks.
You're welcome to disagree with any of them — the path for that is an issue, not a PR.

### 1. The op log is the source of truth

Every mutation flows through `Op` → `Workspace::apply` → log.
The materialized tree and the `.md` files are projections of the log.

**Why:** sync replays the log.
If a mutation skips the log, the second device never sees it.
If you write to `.md` to "fix" state, you've created a divergence the next sync round won't catch.

### 2. Markdown stays 100% clean

No `id::` lines.
No inline UUIDs.
No HTML comments carrying state.
Stable IDs live in the `.outl` sidecar (a JSON file next to the `.md`).

**Why:** the `.md` belongs to the user.
They open it in vim, ship it to a wiki, paste it into a chat.
The day we make their file ugly to serve our internal needs is the day we became Logseq.

The sidecar isn't a dotfile, by the way — iCloud silently drops dotted paths during cross-device sync.

### 3. The CRDT matches the paper

`do_op`, `undo_op`, `apply_op`, and `creates_cycle` in `outl-core/src/tree.rs` follow Kleppmann et al. 2022 literally.
These four functions carry a **100% line and branch coverage rule**.

**Why:** the paper has a formal correctness proof.
Any deviation — even one that looks like a perf win — moves us off the proof, and we have no other way to argue we converge.
A new branch without a test is a blocker.

### 4. A cycle-creating move is a deterministic no-op

If a `Move` op would create a cycle, the materialized tree ignores it.
**But the op still goes into the log.**

**Why:** the log is total-ordered, and replaying it on every device must produce the same materialized tree.
If device A drops the op and device B keeps it, a future re-parenting op that *would* have been valid now diverges.
Keep the op, no-op the effect.

### 5. `Storage` is a trait, not a struct

`outl-core` does not import `rusqlite`, doesn't write JSON directly, doesn't know about files.
Everything goes through `dyn Storage`.
Today the only persistent implementation is `JsonlStorage`; `MemoryStorage` is used in tests.

**Why:** ChronDB ([issue #1](https://github.com/avelino/outl/issues/1)) is the next backend, and we've already paid the cost of removing SQLite in 0.5.0.
Locking the core to a concrete backend reintroduces that cost.
A second persistent backend doesn't land without an RFC issue first.

### 6. Delete is `Move(node, TRASH_ROOT)`

There is no physical removal of a node.
Delete moves it under a sentinel root.

**Why:** the algorithm in the paper is defined over a tree where nodes are never removed.
Physical deletion would either break undo, break replay, or both.

### 7. Convergent state goes through the op log

If two actors can disagree about a value and you want them to reconcile, model it as an `Op`.
Writing convergent state into a shared file — including the sidecar — bypasses the CRDT and loses concurrent writes silently.

**Why:** every shared file is last-write-wins under any file-system transport (iCloud, Syncthing, Dropbox).
Per-actor `ops-<actor>.jsonl` files turn that into a non-issue: each device writes its own file, the CRDT merges on replay, with HLC ordering for determinism.
The canonical example is `Op::SetCollapsed` for fold state.

The sidecar is reserved for **structural matching metadata only** — ids, position, content hash, ref handle.
Not a sync surface.

### 8. Layering

- `outl-core` never imports a UI or CLI crate.
- `outl-actions` is the shared workspace-mutation surface.
  Every client (`outl-tui`, `outl-mobile`, future shells) calls into it.
- A client reimplementing logic that belongs in `outl-actions` is a blocker.
  Reviewers will point at the existing function and ask you to call or extend it.

**Why:** the mobile app and the TUI must behave identically for the same semantic operation.
If toggling a TODO in the TUI emits different ops from toggling one in the mobile app, users see ghosts on sync.
One source of truth per concept.

### 9. No reintroduction of SQLite, rusqlite, or binary log formats

Cross-device sync depends on per-actor append-only JSONL.

**Why:** 0.5.0 removed SQLite specifically to enable iCloud / Syncthing / shared-FS workflows.
Binary formats and DB files don't merge across those transports.

### 10. Settled decisions are off-limits in a PR

ULID for IDs, `uhlc` for time, MIT license, JSONL-per-actor, Tauri for mobile, iroh as the default sync transport (file/iCloud opt-in), `comrak` for markdown.
These were debated and chosen at the start.
The full table is in the root [`CLAUDE.md`](https://github.com/avelino/outl/blob/main/CLAUDE.md#decisions-you-dont-get-to-revisit) and [`CONTRIBUTING.md`](https://github.com/avelino/outl/blob/main/CONTRIBUTING.md#decisions-you-dont-get-to-revisit).

**Why:** these are foundational.
Changing one ripples through every crate.
The path to revisit is an issue with rationale, not an inline review comment.

---

## What reviewers will look at

### Rust quality

The bar is **production code**, not "it compiles":

- **No `.unwrap()` outside `#[cfg(test)]`.** Use `.expect("explicit reason")` or propagate with `?`.
  The `expect` message must name the invariant being asserted — "should not fail" is not a reason.
- **No `.unwrap_or_default()` that masks an error path.** If the default would be a silent data-loss bug, return the error.
- **No `unsafe` in `outl-core`** without a `// SAFETY:` comment naming the invariants the caller upholds.
- **`thiserror` in libraries** (`outl-core`, `outl-md`, `outl-actions`) so callers can match on variants.
  `anyhow` is for binary boundaries (`outl-cli`, `outl-tui`).
- **Async hygiene.** No blocking calls inside `async fn`.
  No `Mutex` / `RwLock` held across `.await`.
- **API ergonomics.** Public API prefers `&str` over `String`, `&[T]` over `Vec<T>` when ownership isn't needed.
- **Public API changes are documented.** If you change a function or type that other crates use, update the doc-comment and the per-crate `CLAUDE.md`.

### Performance — hot paths only

We care about performance in the paths that actually run a lot.
Allocations in setup, error paths, or one-shot CLI commands are not worth a review comment.

The hot paths in outl:

- `outl_core::tree` — every op apply, every materialized-tree walk.
- `outl_core::log` — every append, every replay (boot, sync pull).
- `outl_md::parse` / `outl_md::render` — every `.md` read/write, every TUI buffer refresh.
- `outl_md::index` — backlink index rebuild, scales with workspace size.
- `outl_tui` render loop — runs on every keystroke.
- `outl_actions::SyncEngine` work loop — runs on every file event.

In those paths, reviewers will flag:

- `.clone()` on `String`, `Vec`, or large structs where a borrow works and the clone is per-call (not one-time setup).
- `.to_string()` / `format!()` when `&str` or deferred `Display` would do.
- `Vec::new()` + repeated `push` in a loop where capacity is knowable (`Vec::with_capacity`).
- Re-parsing the same markdown or re-walking the same subtree on every keystroke — propose caching with a clear invalidation story.
- Big-O regressions on tree ops or backlink computation.
- **A synchronous write reintroduced on the input path.** The op log write (`Storage::append_op`) is the one step allowed to be synchronous.
  Every client defers the `.md` + sidecar projection, the backlink index rebuild, and plugin `onOp` hooks off the keystroke / command path.
  TUI: coalesced idle-drain.
  Desktop/mobile: background `ProjectionWriter` / `spawn_blocking` / fire-and-forget hooks.
  A new mutation path that renders or rebuilds inline before replying is a regression — see [`docs/architecture.md` → Every client is async-on-write](architecture.md#every-client-is-async-on-write).

If you're unsure whether code is on a hot path, ask in the PR — we'd rather a question than a guess.

### Architecture, scalability, extensibility

This is where review earns its keep:

- **Reuse-first.** Before adding a helper, scan the [shared primitives catalog](shared-primitives.md) and grep upstream crates (`outl-core` → `outl-md` → `outl-actions`).
  The catalog's rows live in [core](primitives-core.md), [markdown](primitives-markdown.md) and [actions](primitives-actions.md) — grep all three at once.
  Two implementations of "compute backlinks" eventually disagree, and the user is the one who notices.
  See the [Reuse-first](#reuse-first-no-parallel-implementations) section below for the rule, past incidents, and what to do when the primitive doesn't exist yet.
- **New `Op` variants come with the full checklist.** A new variant touches `apply_op`, `undo_op` (the inverse must be exact), the sidecar serializer, the markdown projection, replay tests, and per-crate docs.
  See `/new-op` for the walkthrough.
- **Trait surface stays implementable.** A `Storage` method that assumes file semantics (paths, flock) locks out ChronDB.
  Push back on that.
- **Sidecar / op-log format changes need a migration story.** Existing workspaces on disk must still load.
  Either the change is backward-compatible (new optional field) or the PR ships a versioned migration.
- **File size discipline.** Past 600 lines, plan a split by responsibility.
  Past 900, refactor before the next non-trivial edit.
  The `refactor-architect` agent will propose a split.
- **Premature abstraction is rejected.** A new trait or generic with one impl and no named second caller doesn't merge.
  Rule of Three — concrete first, abstract on the third caller.

### Simplicity

- A new dependency for two functions of `std` code needs a real justification — crate size, maintenance status, transitive deps, licence.
- A configuration knob without a concrete user asking for it doesn't merge.
  Defaults that are right for the 90% case beat knobs nobody tunes.
- Cleverness loses to readability.
  If a reviewer has to run code in their head to understand it, the next maintainer pays.

### Testing

- **Bug fix without a regression test → blocker.** The test must fail on `main` and pass with the patch.
- **Critical path touched without coverage proof.** Run `/coverage outl-core` (or the relevant crate) and paste the result. 100% coverage on the four CRDT functions and on `outl_md::reconcile_md` is non-negotiable.
- **Tests assert behaviour, not implementation.** A test that breaks on any refactor is a maintenance tax.
  Assert against the public surface (op log contents, materialized tree shape, rendered markdown), not internal helpers.
- **Integration tests use the real backend.** Mocked storage in a path that should hit `JsonlStorage` hides exactly the bugs that matter.

---

## Reuse-first (no parallel implementations)

Before adding a helper, struct, or constant, **scan the [shared primitives catalog](shared-primitives.md)** and **grep the workspace** for what already does the same thing.
The catalog is one document split across four files, so grep them together: `grep -n 'symbol' docs/shared-primitives.md docs/primitives-*.md`.
Duplication here is a real hazard: two implementations of the same logic drift apart over time, and the user is the one who hits the divergence.

Past incidents:

- `outl_md::index::Backlink` and `outl_actions::Backlink` were two parallel "backlinks" pipelines that started identical and ended up disagreeing on self-references — a bug the user had to spot because each surface looked fine in isolation.
  Collapsed into `outl_actions::backlinks_for_page` in 0.5.3.
- `outl-mobile`'s `run_code_block` Tauri shim was opened as a copy of `outl-desktop/src-tauri/src/commands/exec.rs` — same `flat_index_for` walk, same `journals/<slug>.md || pages/<slug>.md` probe (which already existed as `outl_actions::page_md_path`), same DTO shape.
  The catalog at the time did not list code execution as a cross-client primitive, and the desktop's per-crate `CLAUDE.md` filed `commands/exec.rs` under "owned by the desktop", so nothing pointed at the right home.
  Caught by the user mid-PR: "do que fizemos de rodar code block no mobile, não conseguimos compartilhar?"
  Collapsed into `outl_actions::exec::run_code_block` in 0.6.x — clients now own only the AppState lookup and the `view` wrapper.
  Lesson: if you're staring at a Tauri shim that's mostly `parse_node_id` → outline walk → `outl-exec` call → DTO, **the walk and the DTO belong in `outl-actions`** — every time.
- The Logseq importer's `crates/outl-cli/src/cmd/import/normalize.rs` was opened reimplementing `\r\n` handling, `id::` stripping, and long-form date rewriting — every one of which `outl_actions::paste::normalize_external_syntax` already owned.
  Caught in PR #47 review.
  Lesson: a "normalize markdown from outside" need always starts at `paste::normalize_external_syntax`; outline-level restructuring (headings → bullets, multi-paragraph merge, fence dedent) is the only thing the importer adds on top.
  (The `cmd/import/` directory has since been replaced by the adapter-based `crates/outl-import` — the lesson still applies there: adapters own dialect translation, shared coercions stay upstream.)

The rule:

1. **Grep before writing.** `rg "fn foo"` / `rg "struct Foo"` across `crates/`.
   Look in **upstream crates first** — `outl-core`, `outl-md`, `outl-actions` are where shared primitives live.
2. **Prefer evolving the existing API** over duplicating, even if that means a small refactor (rename, generalize a parameter, move into a sibling module).
   One owner per concept; many callers.
3. **Duplication is OK only when the platforms are genuinely different.** `outl-tui::EditBuffer` and the mobile `<textarea>` are both "cursor + text" — but one is a terminal widget Rust has to render itself, the other is a browser primitive.
   Same role, different runtime; not duplication.
   **Recalculating** `(line, col)` from `cursor` in both places, though, would be — extract to `outl_md::view::char_to_line_col` and let both wrap it.
4. **Refactor *into* the shared crate, not *around* it.** If a TUI helper feels like it could live in `outl-actions`, move it there *now* (the mobile client will need it soon).
   The `flatten_subtree_paths` migration is the canonical pattern.

When in doubt, name the would-be helper, search for it, then ask yourself: "is the existing thing one rename away?"
If yes, rename.

---

## Keep docs in sync with code

`docs/development.md` is the engineer onramp.
It drifts the moment a CI workflow, a slash command, a hook, or a per-area toolchain step changes — and a stale onramp is **worse than no onramp** because a new contributor follows it confidently into a wall.

**Treat the table below as a checklist.**
If your PR touches any row on the left, update the doc on the right **in the same PR** — not "later", not "in a follow-up".
The `doc-keeper` agent runs at the end of a feature to catch what slipped through; the discipline is to not let it slip in the first place.

| If your PR changes... | Update |
|---|---|
| `.github/workflows/ci.yml` (jobs, matrix, excluded crates, `RUSTDOCFLAGS`, paths-ignore) | `docs/development.md` § 9 (CI walkthrough) |
| `.github/workflows/release.yml`, `mobile.yml`, `desktop.yml`, `testflight.yml`, `bench.yml`, `cleanup-tags.yml` | `docs/development.md` § 9 (CI table) + § 10 (Release process) |
| `.claude/settings.json` hooks, `.claude/agents/*.md`, `.claude/commands/*.md` (any slash command behavior) | `docs/development.md` § 4 (Dev loop) — slash command table + hooks list + agents list |
| `rust-toolchain.toml` version bump | `docs/development.md` § 1 (Quick start) + `CONTRIBUTING.md` (Quick start) |
| Required system deps for a crate (Tauri, GTK, Bun, Xcode, hyperfine, etc.) | `docs/development.md` § 1 ("Optional toolchains by area" table) |
| New crate added to `crates/` | `docs/development.md` § 2 (Repository tour table) + root `CLAUDE.md` repo layout + per-crate `CLAUDE.md` |
| New native iOS surface (file added to `crates/outl-mobile/swift/OutlKit/Sources/`, `crates/outl-mobile/src-tauri/gen/apple/Sources/outl-mobile/`, or `main.mm`) | `docs/development.md` § 3 ("Why the mobile crate has native Swift / ObjC code" table) + § 5 (Testing — Swift rows) + § 6 (Cookbook: Touch the iOS native bridge) + `crates/outl-mobile/CLAUDE.md` if the bridge contract changes |
| New entry point pattern (e.g. new MCP tool family, new TUI overlay class, new theme registration path) | `docs/development.md` § 2 ("Entry points by intent" table) + § 6 (Cookbooks) if it's a recurring shape |
| New `Op` variant, sidecar field, op-log format change | `docs/development.md` § 6 (Cookbook: Add a new `Op` variant) + `docs/crdt.md` + `outl-md/CLAUDE.md` |
| `/check` / `/check-invariants` / `/roundtrip` / `/coverage` / `/new-op` / `/init-playground` semantics | `docs/development.md` § 4 (Dev loop slash command table) |
| Benchmark layout (new bench file, new size tier, hyperfine recipe) | `docs/development.md` § 8 (Performance) |
| Version source-of-truth or release tooling (e.g. someone proposes re-adding `version` to `tauri.conf.json`) | `docs/development.md` § 10 (Release process) + `crates/outl-mobile/CLAUDE.md` |
| Conventional Commits enforcement / release-notes pipeline | `docs/development.md` § 10 + root `CLAUDE.md` "Coding conventions" |
| Storage trait surface, `JsonlStorage` / `MemoryStorage` test contract | `docs/development.md` § 5 ("What to mock and what not to") + `docs/storage.md` + `outl-core/CLAUDE.md` |
| New `Action` variant in `outl-shortcuts` / new keybinding / chord rebound | `docs/shortcuts.md` (the row that ships to users) + `outl-shortcuts/src/{action.rs,defaults.rs}` + every client's dispatcher (`outl-tui/src/input/*.rs`, `outl-desktop/src/lib/{shortcuts.ts,action-handlers.ts}`) + `outl-desktop/src/lib/api.ts` (TS mirror of the `Action` union — no codegen) |
| New helper / DTO promoted into `outl-core` / `outl-md` / `outl-actions` (anything that should be reused across clients) | the catalog part that owns the concept — `docs/primitives-core.md`, `docs/primitives-markdown.md` or `docs/primitives-actions.md` (index: `docs/shared-primitives.md`) + mirror at `.github/instructions/shared-primitives.instructions.md` |
| New code-block runtime in `outl-exec`, new fence language, or `OutputFormat`/`auto_run` change | `docs/markdown-format.md` § Query code blocks + `crates/outl-exec/CLAUDE.md` + alias table in `crates/outl-md/src/lang.rs` and TS mirror |

When in doubt: **if a contributor's first 30 minutes with the repo would land them on outdated guidance, update the doc.** That's the bar.

### One owner per fact — link, don't duplicate

**Every user-facing fact lives in exactly one `docs/*.md`.
`CLAUDE.md` files link to it instead of copying.**
A keybinding, a CLI subcommand, a slash command, a CRDT op variant, a config field, a sidecar field — each has one canonical home under `docs/`, and the matching `CLAUDE.md` (root or per-crate) **links** to it.
It does not enumerate the same rows in its own table.

Why this matters: every duplicated table is a future drift incident.
We've already hit it on `docs/shortcuts.md` ↔ per-crate vim tables (the desktop's CLAUDE.md ended up listing chords that drifted from `defaults.rs` within one sprint) and on `outl-tui/CLAUDE.md`'s Navigation / Insert tables (rebound `Ctrl+E` for sidebar, doc kept showing `\`).
When the same row lives in two places, half the time the second copy goes stale silently and the contributor following it walks into a wall.

The discipline:

- **`docs/*.md` is the canonical surface for users + contributors.**
  Tables, full chord lists, every config key, the full subcommand matrix — they live there.
- **`CLAUDE.md` (root or per-crate) carries only what `docs/` cannot:** invariants, "architectural decisions you don't get to revisit", crate-specific contracts (which methods are the entrypoint, what the layering rule is), the reasoning behind a choice.
  Things a contributor needs *before* touching code — not user-facing reference.
- **When the same fact would live in both,** the `CLAUDE.md` writes a one-line link: `> User-facing X lives in [docs/X.md](../docs/X.md) — don't duplicate it here.` and stops.
- **If you find yourself copying a table from `docs/` into a `CLAUDE.md`, stop.** Replace with a link.
- **The other direction is fine.** `docs/*.md` linking *into* `CLAUDE.md` for architectural context is welcome — that's a one-way pointer toward depth, not duplication.

Map of canonical homes (extend as new ones are minted):

| Fact | Lives in | `CLAUDE.md` files link, do not duplicate |
|---|---|---|
| Every keyboard shortcut (TUI + desktop + mobile, side-by-side) | [`docs/shortcuts.md`](shortcuts.md) | `outl-tui/CLAUDE.md`, `outl-desktop/CLAUDE.md`, `outl-shortcuts/CLAUDE.md` |
| `outl` CLI subcommands + JSON envelope | [`docs/cli.md`](cli.md) | `outl-cli/CLAUDE.md` |
| TUI manual (modes, overlays, visual conventions) | [`docs/tui.md`](tui.md) | `outl-tui/CLAUDE.md` |
| Outl markdown dialect + sidecar spec | [`docs/markdown-format.md`](markdown-format.md) | `outl-md/CLAUDE.md` |
| Query code block DSL + syntax | [`docs/markdown-format.md`](markdown-format.md) § Query code blocks | `outl-exec/CLAUDE.md` |
| CRDT algorithm + invariants | [`docs/crdt.md`](crdt.md) | `outl-core/CLAUDE.md` |
| Storage trait + JSONL backend | [`docs/storage.md`](storage.md) | `outl-core/CLAUDE.md` |
| Sync model (iCloud / Syncthing / iroh roadmap) | [`docs/sync.md`](sync.md) | `outl-mobile/CLAUDE.md`, `outl-desktop/CLAUDE.md` |
| iroh transport internals (pinned iroh 1.0.0 API surface, the named regression + chaos test catalog) | [`docs/iroh-internals.md`](iroh-internals.md) | `outl-sync-iroh/CLAUDE.md` |
| iOS platform integration (bundle ids + entitlements, `BGTaskScheduler` wiring, iCloud layout + peer-file materialisation) | [`docs/ios-platform.md`](ios-platform.md) | `outl-mobile/CLAUDE.md` |
| `outl://` deep links (scheme contract + per-client wiring) | [`docs/deep-links.md`](deep-links.md) | `outl-mobile/CLAUDE.md`, `outl-desktop/CLAUDE.md` |
| MCP wiring + recipes | [`docs/mcp.md`](mcp.md) + [`docs/mcp-recipes.md`](mcp-recipes.md) | (no per-crate CLAUDE.md owns this today) |
| Config file (`outl.toml`) | [`docs/config.md`](config.md) | per-crate CLAUDE.md where the field is read |
| Theming palette + presets | [`docs/theming.md`](theming.md) | `outl-tui/CLAUDE.md`, `outl-desktop/CLAUDE.md` |
| Dev loop (clone, build, slash commands, hooks, agents, CI) | [`docs/development.md`](development.md) | every per-crate CLAUDE.md's "When you're done" section links here |
| Shared Rust primitives (catalogue of reusable APIs) | [`docs/shared-primitives.md`](shared-primitives.md) (index) + its parts [`primitives-core.md`](primitives-core.md), [`primitives-markdown.md`](primitives-markdown.md), [`primitives-actions.md`](primitives-actions.md) + mirror at `.github/instructions/shared-primitives.instructions.md` | root `CLAUDE.md` references it |
| Contributing policy (review, invariants enforced at PR time) | [`docs/contributing.md`](contributing.md) | root `CLAUDE.md` references it |
| Automated-reviewer configuration (which bot reads which file) | [`docs/contributing.md`](contributing.md) § The automated reviewers | `.pr_agent.toml` and `.github/copilot-instructions.md` are the configured surfaces, not second owners of the policy |

When you add a brand-new surface (a new CLI subcommand, a new `Op` variant, a new MCP tool, a new theme, a new client), it follows the same rule:

1. Document the surface in the right `docs/*.md` (create a new one if needed).
2. The per-crate `CLAUDE.md` adds a one-line link if it needs to point contributors at it, plus any architectural note that *cannot* live in `docs/` (invariant, contract, "why this decision").
3. Update the map above so the next contributor doesn't have to rediscover where things live.

**`doc-sync-guard.sh` is a backstop, not the discipline.** The hook flags drift after the fact; the discipline is to link in the first place so drift can't happen.

---

## Markdown / documentation style

**Never hard-wrap prose at an arbitrary column.**
We use [semantic line breaks](https://sembr.org/): one sentence per line, breaking after sentence-ending punctuation (`.`, `!`, `?`) and sometimes after `:` when followed by a substantial clause.
Lines stay as long as the sentence is — no 70/80/100-column reflow.

Why: hard-wrapping at ~70 chars breaks lines mid-thought, makes diffs noisier on edits, and renders ugly in editors that already soft-wrap.
Semantic line breaks keep diffs minimal (an edit touches one line, not a paragraph block) and read naturally on every surface (GitHub, mdBook, terminal pagers).

Rules:

- **Prose**: one sentence per line.
  Don't break inside a sentence.
- **Lists**: each list item on its own line; if a single item contains multiple sentences, break those sentences too.
- **Code fences, tables, YAML frontmatter, ASCII tree diagrams**: preserve **exactly**.
  Tables especially must stay one row per line, no matter how wide.
- **Headings, HRs, link references**: one line, as always.
- **Outline / `.md` content** (anything under `note-example/`, real workspace pages, fixtures): **do not touch**.
  That markdown is data, not docs — it represents the outl dialect literally and indentation / line shape is structural.

This rule applies to every `*.md` in the repo except outline content (see exception above): root `CLAUDE.md`, per-crate `CLAUDE.md`, `docs/*.md`, `README.md`, `CHANGELOG.md`, `CONTRIBUTING.md`, `SECURITY.md`, `.github/*.md`, `.claude/agents/*.md`, `.claude/commands/*.md`.

---

## What we will *not* block your PR for

These are noise.
If a reviewer comments on one of these without a behavioural reason, push back politely:

- Style and formatting — `cargo fmt`, `cargo clippy -D warnings`, and rustdoc warnings handle them in CI.
- `if let Some(x) = y` vs `match y { Some(x) => ... }` with no behavioural difference.
- "I would have named this differently" without a concrete clarity win.
- Speculation about a future architecture that nobody asked for.
- Re-litigating settled decisions (ULID, MIT, JSONL, Tauri, etc.).
- Adding TODOs the author already acknowledged in "Out of scope".

---

## What we are not building yet

outl ships continuously; the CRDT core, TUI, CLI, desktop, iOS **and Android** mobile, plugins, and code-block execution are all in.
Reviewers will push back on PRs that try to introduce these without an explicit issue:

- Query DSL (`{{query: ...}}`).
- `ChronDbStorage` backend ([issue #1](https://github.com/avelino/outl/issues/1)).
- Per-page op log shards ([Per-page op log shards](sync.md#per-page-op-log-shards-for-10k-pages) in `docs/sync.md`, only when the workspace hits 10k pages).

**Android used to be on this list and no longer is.**
It shipped — signed APK on every release, hand-written JNI bootstrap, Android branches in the frontend — while this page still told reviewers to reject work on it.
Android PRs are ordinary PRs; what is open there is release plumbing, in [android-platform.md](android-platform.md).

If your PR genuinely belongs to one of these, link the tracking issue.

---

## Disagreeing with a review

We want this project to be inclusive, which means we want to hear when you think a reviewer is wrong.
The expectation is:

- Disagree on the substance, in the thread.
  Cite the invariant or the decision you think the reviewer is misapplying.
- For settled decisions ("ULID is wrong for us"), open an issue with the rationale — those debates belong in design discussions, not in a code-review thread.
- For new opinions ("this trait surface is over-abstracted"), the PR itself is the right place.
  Reviewers will engage.

The job of a reviewer here is to protect the invariants and the user's data, not to enforce taste.
If a comment crosses that line, say so.

---

## The agents that help

We use specialised review agents (configured in `.claude/agents/`):

- **`crdt-invariant-checker`** runs after any change in `outl-core/src/{tree,log,op}.rs`.
  Validates convergence, idempotency, cycle handling, coverage.
- **`paper-verifier`** compares the Rust against the paper line by line after edits to the four critical CRDT functions.
- **`markdown-roundtrip-tester`** runs after `outl-md/` changes.
  Validates roundtrip stability and matching invariants.
- **`refactor-architect`** is invoked when a file crosses the 600-line threshold.
  Proposes a split by responsibility.
- **`doc-keeper`** runs at the end of every feature that changes public API, markdown syntax, TUI shortcut, sidecar format, or user-observable behaviour.

If you're using Claude Code locally, these run automatically.
If not, they run in CI on the PR.

---

## Where to look next

- [Development guide](development.md) — how to clone, build, run, test, and debug. Pairs with this page.
- Root [`CLAUDE.md`](https://github.com/avelino/outl/blob/main/CLAUDE.md) — project-wide invariants and conventions.
- Per-crate `CLAUDE.md` (e.g.
  [`crates/outl-core/CLAUDE.md`](https://github.com/avelino/outl/blob/main/crates/outl-core/CLAUDE.md)) — invariants specific to that crate.
- [`docs/architecture.md`](architecture.md) — design decisions.
- [`docs/crdt.md`](crdt.md) — the algorithm.
- [`docs/markdown-format.md`](markdown-format.md) — the markdown dialect and sidecar spec.
- [`docs/storage.md`](storage.md) — the `Storage` trait.

Welcome aboard.
