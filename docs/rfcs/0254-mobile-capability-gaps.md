# RFC 0254 — Most of mobile's missing features are not missing features

| | |
|---|---|
| **Status** | Accepted |
| **Issue** | [#254](https://github.com/outlmd/outl/issues/254) (+ [#14](https://github.com/outlmd/outl/issues/14), [#18](https://github.com/outlmd/outl/issues/18), [#19](https://github.com/outlmd/outl/issues/19)) |
| **PR** | none yet |
| **Date** | 2026-09-02 |
| **Reference doc** | [`docs/client-parity.md`](../client-parity.md) |
| **Invariant** | root `CLAUDE.md` invariant 12 |
| **Guarded by** | `undo_immediately_after_an_edit_does_not_duplicate_the_block`, `redo_immediately_after_undo_does_not_duplicate_the_block`, `a_new_edit_after_undo_branches_history_and_clears_redo` (`crates/outl-tauri-shared/tests/history_command.rs`); `undo_page_restores_the_previous_snapshot`, `a_new_edit_after_undo_clears_the_redo_stack`, `history_is_keyed_per_page_not_shared_globally` (`crates/outl-desktop/src-tauri/src/commands/history.rs`); `the_parity_doc_matches_the_code` (`crates/outl-shortcuts/src/support.rs`); `block-selection.test.ts`, `PageSwitcher.test.tsx`, `Journal.buildContextActions.test.ts` (`crates/outl-mobile/src/`). |

Sub-project **C** of the client convergence effort (A → B → C → D).
A unified the design tokens ([RFC 0022](0022-unified-design-tokens.md)).
B made feature parity a compile error ([RFC 0253](0253-client-capability-catalog.md)).
C closes the gaps those two enumerated.

## Why

[`docs/client-parity.md`](../client-parity.md) records **27** cells where the mobile client declares `❌` — "not implemented here, and it should be".

Read as a list, that is a demoralising number and a shapeless one.
Read against the code, it is neither.
Two facts reshape it.

### The 27 cells are far fewer than 27 problems

Ten of them are one feature:

`EnterVisual`, `ReselectLastVisual`, `SelectRangeDown`, `SelectRangeUp`, `IndentVisualRange`, `OutdentVisualRange`, `MoveVisualRangeUp`, `MoveVisualRangeDown`, `YankRange`, `DeleteRange`.

Every one of them is blocked on the same missing thing: **mobile cannot select a range of blocks.** Build that once and ten cells change together. Four more (`CutBlock`, `CopyBlock`, `PasteBlock`, `YankCurrentBlock`) are one block-clipboard feature.

### And most of the rest are not missing *features* at all

The backends were surveyed, and they fall into three groups with nothing in common but the `❌`:

| Group | Examples | What is actually absent |
|---|---|---|
| **A — shared backend already exists** | `copy_block`, `paste_block`, `search_blocks` | Only the mobile UI. The command is in `outl-tauri-shared` and mobile can call it today. |
| **B — backend exists but is trapped** | `undo_page`, `redo_page` | The command lives in `crates/outl-desktop/src-tauri/src/commands/history.rs`, so mobile cannot reach it. |
| **C — no backend anywhere** | `copy_block_ref`, `toggle_pin`, `cut_block`, `fold_all` | Everything. |

**Group B is the same defect [RFC 0022](0022-unified-design-tokens.md) fixed.**
`get_theme` and `list_themes` lived in the desktop's own crate, which is precisely why mobile had no themes for three releases; moving the body into `outl-tauri-shared` was the entire fix.
`undo_page` / `redo_page` are in that state right now, and [#14](https://github.com/outlmd/outl/issues/14) — mobile undo — has been open long enough to be quoted in `Journal.tsx`'s comments as a known gap.

Nobody was wrong to file #14 as a mobile feature request. But it is not one: the undo logic is already shared, in `outl-actions/src/history.rs`, and the desktop reaches it through a thin command that only the desktop registers.

## What we chose

**Order the work by what unblocks the most, and fix the trapped commands before building anything new.**

Four phases. Each lands independently and each flips its own parity cells, so `docs/client-parity.md` is regenerated at the end of every one and never overstates what shipped.

**Phase 1 — free the trapped commands.**
Move `undo_page` / `redo_page` from `crates/outl-desktop/src-tauri/src/commands/history.rs` into `outl-tauri-shared`, following the house pattern the theme commands now demonstrate: shared body, thin `#[tauri::command]` wrapper per client. Register on mobile, add the UI affordance. Closes #14 and two cells for a fraction of what a from-scratch implementation would have cost.

**Phase 2 — UI over backends that already exist.**
`copy_block` / `paste_block` (`outl-tauri-shared/src/commands/block.rs`) and `search_blocks` (`.../page.rs`) are callable from mobile today. This phase is Solid components and gestures, no Rust. Closes #19 and the clipboard cells that do not need range selection.

**Phase 3 — range selection.**
The one genuinely new interaction, and the highest leverage: ten cells. Mobile has no modal Visual state and should not grow one — the parity table already says so for the vim-mode rows, and that reasoning stands. What it needs is a touch-native multi-select (long-press to start, drag or tap to extend), with the existing range actions dispatched from it.

**Phase 4 — the individually small ones.**
`FoldAll` / `UnfoldAll`, `NewBlockAbove`, `DeleteEmptyBlock`, `CopyBlockRef` (#18), `TogglePin`, `InsertRemindNag`. Each needs a backend or a UI affordance of its own; none unblocks another.

**`OpenCommandPalette` and `PageHistory` are deliberately not in any phase** — see Scope.

## Why not the alternatives

**Work the parity table top to bottom.**
The obvious reading, and it is the worst order: it starts with `OpenCommandPalette` and reaches range selection — ten cells — last, having spent the budget on singletons.

**Build range selection first, since it unblocks the most.**
Tempting, but it is the only phase that invents a new interaction, so it carries the most design risk and the least certainty. Phases 1 and 2 are mechanical, close #14 and #19, and produce a shorter table before the risky work starts.

**Implement mobile undo natively rather than moving the command.**
This is what #14 implicitly proposed, and it would have been a second implementation of logic `outl-actions` already owns — the exact drift `docs/contributing.md` § Reuse-first forbids. The survey is what caught it.

**Give mobile a modal Visual state so the vim range actions work unchanged.**
Rejected on the same grounds the parity table already states for `EnterInsertAfter` and the char-cursor ops: mobile edits on tap and has no modes. A hidden modal state on a touch surface is a worse answer than a touch-native selection that dispatches the same actions.

## The opposite direction

**Required section — what this makes worse.**

**Every closed cell is a nudge string deleted, and nudges were load-bearing.**
Each `❌` carries text telling the user what to do instead. Turning a cell `Full` removes it. If a phase ships the capability but misses an edge case, the user now gets silence where they used to get a sentence pointing at the desktop.

**Mobile's surface grows fastest where it is least tested.**
The mobile client has 67 tests against the desktop's 96 and the shared lib's 333. Phase 3 in particular adds a stateful gesture interaction, which is the hardest kind to test and the easiest to regress.

**Phase 1 moves code the desktop depends on.**
`undo_page` / `redo_page` are wired into the desktop's `state.rs`, not just its command module. Moving the body must not change desktop behaviour, and the desktop's undo has no test naming it today.

**Nothing here touches reconciliation, sync, projection or the op log.**
Undo already goes through `Op` and the log; this RFC moves where a command is registered, never what it does. No `.md` is written, no sidecar read. A missing mobile button cannot cost a user content — which is exactly why this is sub-project C and not sub-project A.

## Who does not have this

Per [invariant 12](../../CLAUDE.md).

**The TUI keeps its own `❌` cells and this RFC does not touch them** — `CutBlock`, `CopyBlock` and `PasteBlock` are `Missing` there, and the block clipboard work in Phase 2 is mobile-only by scope. If Phase 2's shared work makes the TUI's gap cheap to close, that is a follow-up, recorded rather than silently bundled.

## How it cannot regress

**The rule.** Invariant 12 already binds: every cell this RFC flips must be flipped in `outl_shortcuts::support` (or `capability_support`), not just in the client. A client that gains a feature without updating its declaration re-creates the drift RFC 0253 removed.

**The tests**, per phase:
1. `undo_page` / `redo_page` have **no test naming them today**. Phase 1 adds coverage of the shared body before moving it, so the move is provably behaviour-preserving rather than merely compiling.
2. `the_parity_doc_matches_the_code` already fails if a client's declaration and the doc disagree — every phase ends by regenerating it.
3. Each phase adds mobile-side tests for what it built, and `capability_parity.rs`'s mobile assertions extend to any capability that changes verdict.

## Scope

Not covered here:

- **`OpenCommandPalette` on mobile.** The desktop does not have it either — both declare `Missing`. Making it a mobile-only feature would invert the parity it is meant to fix; it belongs in its own issue covering both clients.
- **`PageHistory` on mobile.** Tracked as a `Capability` by [RFC 0253](0253-client-capability-catalog.md), not an `Action`, and it needs a timeline UI rather than a keybinding. Separate work.
- **The CLI and MCP vocabulary** — sub-project D ([#255](https://github.com/outlmd/outl/issues/255)).
- **Closing the TUI's own clipboard gaps** — noted above, deliberately not bundled.
