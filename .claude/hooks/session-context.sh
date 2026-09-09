#!/usr/bin/env bash
# SessionStart hook: inject critical context every session.
#
# Reminds Claude of the invariants that MUST NOT be violated and the
# current state of the project. Keeps the model from drifting on long
# sessions where the original spec scrolls out of the immediate window.

set -uo pipefail

cat <<'EOF'
# outl session context

You are working on **outl**, a local-first outliner with CRDT-based tree sync.

## CRITICAL invariants (NEVER violate)

1. **Op log is source of truth.** All mutations go through `Op` → `apply_op` → log.
   The `.md` file is a projection. Never edit `.md` directly to "fix" state.

2. **Markdown stays 100% clean.** No `id::`, no UUID, no HTML comments.
   IDs live ONLY in the `.outl` sidecar (JSON dotfile).

3. **The CRDT algorithm follows Kleppmann et al. 2022 literally.**
   `do_op` / `undo_op` / `apply_op` / `creates_cycle` must match the paper.
   100% test coverage on these four functions is non-negotiable.

4. **A move that creates a cycle is a deterministic NO-OP, but the op
   stays in the log.** Removing the op breaks reordering correctness.

5. **Storage is a trait, not a struct.** Never call into `rusqlite`
   from `outl-core`. Everything goes through the `Storage` trait.

6. **A `.md` holding lines the op log never saw is never overwritten.**
   A sidecar hash match proves outl wrote the file last, NOT that the
   ops exist. Ask `outl_actions::content_lines_missing_from` before
   re-projecting. Root `CLAUDE.md` invariant 8 has the incident.

7. **A capability difference between clients is declared, never
   discovered.** `outl_shortcuts::support` / `capability_support` are
   exhaustive `match`es; `outl-tauri-shared/tests/{command_parity,
   wire_types}.rs` are the same rule for commands and wire DTOs.

## Where new code goes (do NOT hand-roll these)

The four things most likely to be written in the wrong place:

- **A new Tauri command** → body in `outl-tauri-shared/src/commands/`,
  one line in `src/wrappers/catalog.rs`, then an `invoke_handler!`
  entry in **both** clients. Never hand-write a `#[tauri::command]`
  wrapper in a client crate — `tests/command_parity.rs` fails if a
  client skips a module or leaves a generated command unregistered.
  Shared bodies take `String`, not `&str`.

- **A page mutation** → `outl_actions::commit_page(ws, hooks, page, f)`.
  It owns the five-step sequence (undo snapshot, mutation, backlink
  invalidation, peer announce, projection). A bare
  `apply_page_md_with_sidecar_guarded` is step 5 alone and is only
  correct with a comment naming which steps you skip and why.

- **A new wire DTO / TS interface** → both sides, plus a pin in
  `outl-tauri-shared/tests/wire_types.rs` (or an `UNPINNED` row with a
  reason). The frontend contract is hand-written on purpose; the test
  is what makes that safe.

- **Anything visual** (a colour, token, spacing value, component, or
  interaction) → read `DESIGN.md` first. Colours come from
  `outl_theme::Palette`; a hex in a client stylesheet, or a
  `--color-outl-*` token with no `Palette` field behind it, is a second
  definition of that colour.

- **A new shared helper** → `outl-actions` (or core/md), listed in
  `docs/primitives-*.md` **and** mirrored in
  `.github/instructions/shared-primitives.instructions.md`. Grep the
  catalog before writing it.

## Reminders

- Read `CLAUDE.md` in the crate you're touching before making changes.
- Per-crate invariants are in `crates/<name>/CLAUDE.md`.
- The paper: <https://martin.kleppmann.com/papers/move-op.pdf>
- Tests in `crates/outl-core/tests/` are spec, not afterthought.

## State

Shipped: TUI, CLI, desktop (Tauri 2), mobile (iOS **and** Android), the
JavaScript plugin system (Boa), code-block execution, the query DSL
(`{{query}}`), and P2P sync over **iroh** — which is the default
transport (`[sync] transport = "iroh"`), with `"file"` (iCloud Drive /
shared FS) as the explicit opt-out. LAN peer discovery over mDNS landed
too.

Not built: ChronDB storage (issue #1), graph view, app-closed reminder
delivery, per-page op-log shards beyond the current layout.

Do not describe iroh, Android or the query DSL as future work — they are
in `main`.
EOF

exit 0
