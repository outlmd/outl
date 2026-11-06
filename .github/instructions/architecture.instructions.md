---
applyTo: "crates/**"
---

## Reuse-first violations — no parallel implementations

Duplication here is a real hazard: two implementations of the same logic drift apart over time, and the user is the one who hits the divergence.

**Past incidents to anchor severity:**

- `outl_md::index::Backlink` and `outl_actions::Backlink` were two parallel "backlinks" pipelines that started identical and ended up disagreeing on self-references — caught by the user, not the reviewer.
  Collapsed into `outl_actions::backlinks_for_page` in 0.5.3.
- PR #47 (Logseq import) opened with `crates/outl-cli/src/cmd/import/normalize.rs` reimplementing `\r\n` handling, `id::` stripping, and long-form date rewriting — every one of which `outl_actions::paste::normalize_external_syntax` already owned.
  (That directory has since been replaced by the adapter-based `crates/outl-import`; the lesson carries over.)
  Caught in review *after* a Claude-assisted PR shipped without the catalog being visible.
  That is why the shared primitives catalog exists (see `shared-primitives.instructions.md`).

The rule the PR author was expected to follow:

1. **Grep before writing.** `rg "fn foo"` / `rg "struct Foo"` across `crates/`.
   Look in **upstream crates first**, in this order: `outl-core` → `outl-md` → `outl-actions`.
   These are where shared primitives live.
   The catalog above is your starting point.
2. **Prefer evolving the existing API** over duplicating, even if that means a small refactor (rename, generalize a parameter, move into a sibling module).
   One owner per concept; many callers.
3. **Refactor *into* the shared crate, not *around* it.**
   If a TUI helper feels like it could live in `outl-actions`, the PR should move it there *now* — the mobile client will need it soon.
   The `flatten_subtree_paths` migration is the canonical pattern.
4. **Duplication is OK only when the platforms are genuinely different.** `outl-tui::EditBuffer` and the mobile `<textarea>` are both "cursor + text", but one is a terminal widget Rust has to render itself and the other is a browser primitive.
   Same role, different runtime — not duplication.
   **Recalculating** `(line, col)` from `cursor` in both places, though, would be — extract to `outl_md::view::char_to_line_col` and wrap.

When you spot a duplicate, point at the existing function with `file:line` and ask: "can you call this instead, or extend it if it doesn't quite fit?
The fix is to wrap or evolve the upstream API, **never** to write a parallel one.
If the author argues for duplication, they have to fit it into case 4 above — same role, genuinely different runtime.
Anything else is a blocker.
- **Layering violations.**
  UI imports in `outl-core`.
  Client crates building op trees instead of calling `outl-actions`.
  Workspace mutations done outside `Workspace::apply`.
- **New `Op` variant without the full checklist.**
  Adding a variant touches `apply_op`, `undo_op` (the inverse must be exact), the sidecar serializer, the markdown projection, the replay tests, and the per-crate docs.
  Check the diff against `/new-op` expectations and call out anything missing.
- **Trait surface that locks out a future backend.** `Storage` must stay implementable by ChronDB later.
  If a new method assumes file semantics (paths, flock), question it.
- **Sidecar / op-log format changes without a migration story.**
  Existing workspaces on disk must still load.
  Either the change is backward-compatible (new optional field) or there is a versioned migration path described in the PR.
- **File size growth past 600 lines.**
  Note it, suggest a split by responsibility, point at `refactor-architect` agent.
  Past 900 lines, request a refactor before merge.
- **Premature abstraction.**
  A new trait or generic with one impl and no second use case in sight.
  The Rule of Three applies — concrete first, abstract on the third caller.

## Documentation drift — block PRs that change behavior without updating the dev/contrib docs

`docs/development.md` (engineer onramp) and `docs/contributing.md` (review policy) are the two pages a new contributor reads before opening their first PR.
A stale onramp is **worse than no onramp** because it sends contributors confidently into a wall — they follow steps that no longer work and silently distrust the project the rest of the way.

**Use this table to decide when the PR must update docs.**
If you see a diff in the left column and no matching update in the right column, request the doc change before approving.

| If the PR touches... | Require an update to |
|---|---|
| `.github/workflows/ci.yml` (jobs, matrix, excluded crates, `RUSTDOCFLAGS`, paths-ignore) | `docs/ci-and-release.md` § 1 (CI walkthrough) |
| `.github/workflows/release.yml`, `mobile.yml`, `desktop.yml`, `testflight.yml`, `bench.yml`, `cleanup-tags.yml` | `docs/ci-and-release.md` § 1 (CI table) and § 2 (Release process) |
| `.github/actions/*/action.yml` (a step shared by more than one workflow) | `docs/ci-and-release.md` § 1 ("Shared build setup") |
| `.claude/settings.json` hooks, `.claude/agents/*.md`, `.claude/commands/*.md` (any slash command or hook behavior) | `docs/development.md` § 4 (Dev loop) |
| `rust-toolchain.toml` version bump | `docs/development.md` § 1 and root `CONTRIBUTING.md` |
| System deps for a crate (Tauri, GTK, Bun, Xcode, hyperfine, etc.) | `docs/development.md` § 1 ("Optional toolchains by area") |
| New crate added to `crates/` | `docs/development.md` § 2, root `CLAUDE.md` repo layout, per-crate `CLAUDE.md` |
| New native iOS surface (file added to `crates/outl-mobile/swift/OutlKit/Sources/`, `crates/outl-mobile/src-tauri/gen/apple/Sources/outl-mobile/`, or `main.mm`) | `docs/development.md` § 3 (the "Why the mobile crate has native Swift / ObjC code" table — does the new surface fit an existing row or is it a new reason?) + § 6 cookbook + `crates/outl-mobile/CLAUDE.md` |
| New `Op` variant, sidecar field, or op-log format change | `docs/development.md` § 6 cookbook + `docs/crdt.md` + `outl-md/CLAUDE.md` |
| `/check` / `/check-invariants` / `/roundtrip` / `/coverage` / `/new-op` / `/init-playground` semantics | `docs/development.md` § 4 (slash command table) |
| Benchmark layout (new bench file, new size tier, hyperfine recipe) | `docs/development.md` § 8 (Performance) |
| Version source-of-truth or release tooling (e.g. someone re-adds `version` to `tauri.conf.json`) | `docs/ci-and-release.md` § 2 + `crates/outl-mobile/CLAUDE.md` (and reject re-adding the `version` field — it's an invariant) |
| Conventional Commits enforcement or release-notes pipeline | `docs/ci-and-release.md` § 2 + root `CLAUDE.md` "Coding conventions" |
| Storage trait surface, `JsonlStorage` / `MemoryStorage` test contract | `docs/development.md` § 5 + `docs/storage.md` + `outl-core/CLAUDE.md` |
| New `Action` variant in `outl-shortcuts` / new keybinding / chord rebound | `docs/shortcuts.md` (the row that ships to users) + `outl-shortcuts/src/{action.rs,defaults.rs}` + every client's dispatcher (`outl-tui/src/input/*.rs`, `outl-desktop/src/lib/{shortcuts.ts,action-handlers.ts}`) + `outl-desktop/src/lib/api.ts` (TS mirror of the `Action` union — no codegen, drift here is silent until runtime) |
| Public API of a shared primitive listed in the catalog (`docs/primitives-*.md`) | The matching catalog row in the part that owns it |

Phrase the comment so the author knows exactly which file and section to move.
"Doc looks stale" is noise; "section 9 of `docs/development.md` still says `ci.yml` runs on the workspace including `outl-mobile` — this PR removes that exclusion" is review.

---
