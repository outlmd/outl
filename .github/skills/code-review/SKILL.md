---
name: code-review
description: Verification routine for reviewing an outl pull request. Maps each area a diff touches (op log, sidecar, reconcile, shared primitives, shortcut and capability catalogs, theme tokens, CI, docs) to the specific greps, files and regression tests that decide whether the change is safe. Use when reviewing a pull request, a diff, or a commit in this repository, and before approving any change under crates/, .github/workflows/ or docs/.
license: MIT
metadata:
  repo: outlmd/outl
---

# Reviewing an outl pull request

`.github/copilot-instructions.md` carries the **rules**: the invariants, the merge bar, what not to comment on, how to format the review.
Read it first and defer to it.

This file carries the **routine**: given what the diff touches, the concrete checks that separate a real finding from a guess.
Every check below exists because the corresponding bug shipped at least once.

Work in the order given.
Stop at the first section that does not apply.

## Step 0 — Route by what changed

Run `git diff --name-only origin/main...HEAD` and match paths against this table.
Only the matching rows apply.

| Changed path | Go to |
|---|---|
| `crates/outl-md/src/reconcile.rs`, `matching/`, `sidecar.rs`, `unlogged.rs` | [Reconcile and the sidecar](#reconcile-and-the-sidecar) |
| `crates/outl-core/src/tree/`, `log.rs`, `op.rs` | [The CRDT](#the-crdt) |
| Any new `pub fn` or helper under `crates/` | [New helpers](#new-helpers) |
| `crates/outl-shortcuts/` | [Catalogs with a per-client verdict](#catalogs-with-a-per-client-verdict) |
| `crates/outl-theme/`, any client stylesheet | [Theme tokens](#theme-tokens) |
| `crates/outl-*/src-tauri/`, `outl-tui/src/`, `outl-mobile/src/`, `outl-desktop/src/` | [Client code](#client-code) |
| `.github/workflows/`, `.claude/hooks/`, `scripts/` | [CI and guards](#ci-and-guards) |
| `Cargo.toml`, `package.json`, lockfiles | [Dependencies](#dependencies) |
| Any `.md` | [Docs](#docs) |

## Reconcile and the sidecar

This is the area that already deleted user content, so it gets the most scrutiny.
[RFC 0210](../../../docs/rfcs/0210-md-content-outside-op-log.md) has the full incident: 233 pages, 1,426 lines, made undetectable afterwards by the rebuilt sidecar.

**Check 1: does the diff advance `last_synced_hash`?**

```bash
git diff origin/main...HEAD -- crates/outl-md/ | grep -n 'last_synced_hash'
```

Writing that hash is a claim that the op log holds everything in the file.
If the diff advances it on a path that did not emit an op for every line it read, that is a blocker.
The gate must be `unlogged::content_lines_missing_from(disk, &sidecar.blocks)`, asked **against the sidecar's blocks, never against a fresh render**.
A render answers "do disk and tree disagree", which is also yes for every remote edit and reorder, and that is issue #166 reintroduced with the blame moved.

**Check 2: is the regression net still intact?**

```bash
grep -rn 'if_stale_refuses_when_the_md_carries_content_the_log_lacks\|if_stale_declines_when_the_sidecar_cannot_answer\|recovery_does_not_reproject_over_text_the_log_never_saw\|a_torn_op_log_never_lets_repair_overwrite_a_good_md' crates/
```

Those tests exist to fail if someone re-simplifies the gate back to a hash comparison.
A diff that deletes, renames or relaxes one needs an explicit justification in the PR body.
Silence there is a blocker.

**Check 3: `SIDECAR_VERSION`.**

If the diff bumps it, the change must make an *existing* field mean something different.
A merely additive field rides at the same version with `#[serde(default)]`, detected by presence.
Bumping for an additive field makes every already-shipped binary reject the file, and downstream a rejected sidecar reads as a missing one: fresh ULID per block, every `((blk-…))` handle rotated, duplicates on both sides of the sync.
See `crates/outl-md/CLAUDE.md` → "Sidecar versioning".

## The CRDT

`do_op`, `undo_op`, `apply_op` and `creates_cycle` must match Kleppmann et al. 2022 line for line, at 100% coverage.

Two shapes that look like bugs and are not.
Do **not** report these:

- A move that creates a cycle is a no-op on the materialized tree **and still goes into the log**.
  Removing the log write breaks correctness of future reordering.
- Delete is `Move(node, TRASH_ROOT)`, never a physical removal.

What to actually check: any HLC comparison must include the actor tiebreak.
A comparison on timestamp alone is a convergence bug that property tests may not surface at the default case count.

## New helpers

Before approving any new function, struct or constant under `crates/`, confirm it is not a second implementation of something that exists.

```bash
grep -n '<symbol or intent>' docs/shared-primitives.md docs/primitives-*.md
grep -rn '<symbol>' crates/ --include='*.rs' --include='*.ts'
```

The catalog is one document across four files, so grep them together.
Backlinks, code-block execution and external-markdown normalization were each caught mid-PR as duplicates.
The fix is always to wrap the upstream API, never to write a parallel one.

Two specific homes people miss:

- UI-agnostic workspace mutations belong in `outl-actions`, not in a client.
  If the diff adds tree-walking or op-building inside `outl-tui/`, `outl-mobile/` or `outl-desktop/`, that is the finding.
  `outl-tui/src/outline_ops.rs` is the one deliberate exception, documented in its module doc.
- Pure, stateless TS shared by both GUI clients belongs in `crates/outl-frontend-shared/`, not copied into one.

## Catalogs with a per-client verdict

`outl_shortcuts::support` and `outl_shortcuts::capability_support` are exhaustive `match`es, so a new variant will not compile until every client declares its verdict.
That part is enforced by the compiler.

What is **not** enforced, and what to check:

- The reason text shown to the user must live in the catalog, not written fresh in a client.
  A client wording its own "not available here" message is a fourth copy of the fact.
- `docs/client-parity.md` is generated and pinned by `the_parity_doc_matches_the_code`.
  If the diff edits that doc by hand, that is the finding.
- `Support::Native` (reachable, no handler, no nudge) is a real state, distinct from missing.
  `Backspace` on an empty textarea works on desktop with no handler because the platform does it.
  Do not report a `Native` row as an unimplemented action.

## Theme tokens

Colors come from `outl_theme::Palette` and reach clients through `applyPaletteToRoot()`.

```bash
git diff origin/main...HEAD | grep -nE '^\+.*#[0-9a-fA-F]{3,8}\b'
```

A hex literal added to a client stylesheet is a second definition of a color.
The one legitimate exception is a client's `@theme` boot block, which exists so the first painted frame is branded before the palette arrives over the wire.
A token declared there under a name that is not a real `Palette` field leaves that property unset for the first frame, which `the_theme_tokens_match_the_palette` catches.

## Client code

- No `.unwrap()` in non-test code.
  Use `expect("explicit reason")` or propagate.
- A `PageMarkdownAheadOfLog` refusal must reach the user.
  A client that swallows it into a log line ships a page that silently stopped syncing.
  See `docs/clients.md` → "Surfacing a page that stopped syncing".
- Writes must not block the UI.
  This project's clients never `await` a write on the interaction path, they commit in the background.
- Reminder schedule math has one owner, `outl_actions::reminders::next_fire_at`.
  A second opinion in TypeScript or Swift reaches the user at 3am on one device before it reaches a test.

## CI and guards

**A guard that can pass without checking anything is worse than no guard.**
For any new or changed check, find its failure path and confirm it fails closed:

- Does it exit non-zero when its input is empty or missing?
- Does a `2>/dev/null`, a missing `set -e`, or an unchecked `||` swallow the error that should fail it?
- For a `Stop` hook, does it avoid looping when the condition cannot be cleared (`stop_hook_active`)?

**Hooks only run for Claude Code.**
`.claude/hooks/` are `PostToolUse` on `Edit|Write`, which does **not** include the Bash tool.
A file written through `python3`, `sed -i` or `cat >` reaches disk having passed no guard.
A rule that must hold for humans, Copilot PRs and dependabot needs a CI counterpart, so if a PR adds a rule as a hook only, say so.

**Workflow `paths:` filters.**
When a PR adds a test that pins one crate against another, check that both crates appear in the `paths:` of the workflow running that test.
A parity test behind a filter that omits the crate it pins does not run on the change that breaks it.

## Dependencies

Direct dependencies track latest, including majors.
Two hard constraints:

- **No SQLite, rusqlite, or any binary log format.**
  Cross-device sync depends on per-actor append-only files.
- **No bincode.**
  Every version carries RUSTSEC-2025-0141 and `outl-core` is published for embedding, so a downstream `cargo deny` fails on the advisory.
  `postcard` is the choice.

`crates/outl-mobile/src-tauri/tauri.conf.json` must **not** carry a `version` field.
CI reads the version from `Cargo.toml` and injects it, and Tauri's iOS path silently defaults to `1.0.0` otherwise.

## Docs

**Same-PR rule.**
A change to a workflow, slash command, hook, public API, sidecar, op-log format, shortcut or user-observable behavior updates its doc in the same PR.
`docs/contributing.md` → "Keep docs in sync" has the "if you changed X, update Y" map.

**One owner per fact.**
Before accepting a doc paragraph, ask whether that fact already lives somewhere else.
If it does, the new copy should be a link.
Every doc contradiction closed in issue #212 was the same fact written in two files, each stale in a different direction.

**A doc that names a symbol must name one that exists.**
This is mechanical and worth doing:

```bash
# for each identifier the diff added to a doc
grep -rn '<identifier>' crates/ --include='*.rs' --include='*.ts'
```

Four docs described `icloud_path.rs`, `icloud_workspace_root` and `ICLOUD_CONTAINER_ID` for months after the code was deleted.
A grep settles it in seconds.

**Semantic line breaks:** one sentence per line, no column reflow.
Do not report a long single-sentence line as needing a wrap.

## Verifying

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

bun run test        # vitest, every package
bun run typecheck   # tsc --noEmit, every package
```

Roughly a third of each GUI client is TypeScript, and invariants 12 and 13 are enforced only by TS parity tests.
A Rust-only run does not clear the bar.

## Before posting

State the concrete consequence, not the rule.
"This advances `last_synced_hash` over a block that produced no op, so the next page open renders over the user's text" earns a fix.
"This violates invariant 8" earns an argument.

If a finding cannot be tied to a real-world consequence, drop it.
One sharp comment builds more trust than five plausible ones.
