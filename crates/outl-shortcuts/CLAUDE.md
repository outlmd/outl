# CLAUDE.md — outl-shortcuts

Single source of truth for outl's keyboard bindings.
**Every client that resolves a key → action goes through this catalog** — TUI today, desktop today, any future GUI tomorrow.

> User-facing chord list (TUI + desktop + mobile side-by-side) lives in [`docs/shortcuts.md`](../../docs/shortcuts.md).
> This file carries only architectural rules: how the catalog is structured, how clients consume it, what to do when adding a binding.
> Don't duplicate the chord table here — root `CLAUDE.md` → "One owner per fact" has the policy.

## Why this crate exists

> Full reasoning, rejected alternatives, and the vim-parity gap on the desktop: [RFC 0070](../../docs/rfcs/0070-keybinding-ownership-and-vim-parity.md).

Before this crate, the TUI defined its bindings in `outl-tui/src/input/` and the desktop wired its own `KeyboardEvent` handlers in `lib/shortcuts.ts`.
Result: `Cmd+P` opened the picker on the desktop, `Ctrl+P` did nothing on the TUI for two weeks until somebody noticed.
With both crates pulling from a shared `(chord → action)` table, "the key the user knows" works the same on every surface, and the help overlay on each client can list every binding in one query.

## Hard rule

**No client hardcodes a `(chord, action)` pair.**
Every binding lives in `src/defaults.rs::default_bindings`; clients consume it via `default_bindings()`, `bindings_for_mode(mode)`, or `lookup(mode, chord)`.

If a client needs a new shortcut, it goes here first.
A binding that only the TUI cares about still lives here (with `Mode::Normal` / `Mode::Insert` / `Mode::Visual` / `Mode::Overlay`) — the desktop just won't subscribe to it unless `settings.vim_mode == true`.

> **Pending-input ops.** Three actions — `ReplaceChar` (vim `r{ch}`), `FindCharForward` (`f{ch}`), `FindCharBackward` (`F{ch}`) — are bound to a single key but **read a second character before they apply**.
> The TUI implements this with `state::PendingInputOp` (consumed in the next `handle_normal_key` call); a GUI client without that machinery should either prompt modally or skip the binding outside `vim_mode`.
> The catalog still owns the `(chord, action)` pair so the help overlay stays consistent across clients; the second-char read is a client concern.

## What this crate owns

- **[`Action`]** — every named operation outl performs in response to a key.
  Tagged-union serde (`{"kind": "OpenToday"}`) so the desktop frontend can `switch` on a string instead of an arbitrary integer.
  The user-facing chord list (including `DeletePage` and its `g d` binding) lives in [`docs/shortcuts.md`](../../docs/shortcuts.md); this file carries only architectural rules, never a chord description.
- **[`Chord`] / [`ChordSequence`]** — modifier-prefixed key combos, expressed independently of any input library so `crossterm::KeyEvent` (TUI) and `KeyboardEvent` (browser DOM) can both map into them.
  `Chord::parse` / `ChordSequence::parse` turn a human string (`"Ctrl+Shift+A"`, `"Ctrl+T S"`) into the typed form.
  This is the one owner of that parse, so a plugin's `contributes.keybindings[].key` resolves the same way on every client instead of each reimplementing it.
- **[`Mode`]** — which modal state a binding applies to. `Global` matches everywhere; `Normal` / `Insert` / `Visual` / `Overlay` are the vim modes (desktop subscribes only while `settings.vim_mode == true`).
- **[`Binding`]** — `(mode, chord, action, description)` row.
  The description is what the help overlay displays — keep it short and verb-led ("Open today's journal", not "This shortcut opens today's journal in the current window").
- **[`default_bindings`]** — the canonical table.
  Hand-curated, ordered for help-overlay readability.
- **`support` / `Support` / `Client`** (`src/support.rs`) — **which client performs which action**, and the sentence shown to the user when one cannot.
  One exhaustive `match`, so a new [`Action`] variant does not compile until all three clients have a verdict.
  The reason text lives here, not in the client: a client that writes its own wording is a second copy of the fact, which is how three docs ended up disagreeing about `y r`, `:` and mobile undo.
  Rendered to [`docs/client-parity.md`](../../docs/client-parity.md) and pinned by `the_parity_doc_matches_the_code`.
  `Support::Native` is deliberately its own state — reachable, no handler, no nudge (`Backspace` on an empty textarea) — because a boolean would force that row to lie in one direction or the other.
- **[`bindings_for_mode`] / [`lookup`]** — query helpers.
  `lookup` is `O(n)` over the table; the table is small (under 100 entries today) so we don't bother with a hashmap.
  **`lookup` prefers a mode-specific binding over a `Global` one** for the same chord — it can't rely on table order because the `Global` chrome rows are listed first for help-overlay readability,
  so a naive first-match would resolve every dual-bound chord (e.g. `Cmd+Shift+X` → `WrapStrike` in Insert vs. `RunCodeBlock` in Global) to its `Global` action even inside the overriding mode.

## What this crate does NOT own

- ❌ Handlers.
  Each client maps `Action -> {do_something}` itself.
  The TUI's `App::dispatch` and the desktop's `lib/action-handlers.ts` are the dispatchers; this crate doesn't know how a "commit insert buffer" actually commits.
  > **Handler behaviour contracts** (e.g. `FoldAll` skipping leaves, `UnfoldAll` walking the full tree, `DeleteRange` iterating bottom-up)
  > live in the per-client CLAUDE.md (`outl-tui/CLAUDE.md`, `outl-desktop/CLAUDE.md`) under their vim / mode sections,
  > and in `docs/tui.md` / `docs/shortcuts.md` for user-facing copy.
  > This file owns *which `Action` exists*; the *what the handler does* is a per-client concern documented next to the dispatcher.
  > The `doc-sync-guard` hook flags edits to `action-handlers.ts` here as a heuristic —
  > when the change is purely how the handler dispatches (not a new chord or `Action` variant), the per-client doc is the right place to land the contract, not this file.
- ❌ Input adapters.
  `crossterm::KeyEvent → Chord` lives in `outl-tui`; `KeyboardEvent → Chord` lives in `outl-desktop/src/lib/shortcuts.ts`.
  > **Only the desktop actually resolves through [`lookup`].**
  > The TUI dispatches Normal-mode keys from its own `match` in `input/normal.rs`; `input/chord_adapter.rs` exists to match *plugin* chords and nothing else.
  > So the catalog and the TUI's arms can disagree without failing to compile — `reminder_chord_tests` pins a handful of chords against `lookup`, and the rest are unpinned.
  > `docs/shortcuts.md` claimed both clients called `lookup` for months. If you re-spell a chord in `defaults.rs`, grep `input/normal.rs` yourself; the build will not do it for you.
- ❌ User-level overrides (rebinding `i` to `a`).
  When that ships, it'll go through the same `Vec<Binding>` shape — a user override is just a different source list fed into the same `lookup` algorithm.
- ❌ OS-specific chord rewriting (`Cmd` ↔ `Ctrl`).
  Each client decides which physical key its `Chord::ctrl(c)` corresponds to on the running OS.

## Adding a binding

> **Doc updates are part of the change, not a follow-up.** The
> `doc-sync-guard.sh` hook (`PostToolUse:Edit`) now fires the moment
> `defaults.rs`, `action.rs`, `outl-tui/src/input/*`, or the desktop
> frontend's shortcut wiring (`shortcuts.ts`, `action-handlers.ts`,
> `BlockRow.tsx`) is touched — it requires the matching CLAUDE.md
> tables to update in the same edit. We learned this the hard way on
> the `Cmd+T` → `Cmd+J` swap: the binding moved silently because the
> hook only watched line counts, not the catalog file. Don't disable
> the guard; treat the warning as a checklist item.

1. **Pick the mode honestly.** `Global` only if the chord must fire in every mode — chrome chords (`Cmd+P`, `Cmd+J`) yes; `j` / `k` no (those are `Normal`).
2. Append a `Binding { mode, chord, action, description }` to the relevant section of `default_bindings()`.
3. Run `cargo test -p outl-shortcuts` — `no_duplicate_chord_in_same_mode` catches collisions; `every_binding_has_a_description` catches empty descriptions; `bindings_round_trip_via_serde` catches schema breakage.
4. If the action is **new**, also extend [`Action`] (`src/action.rs`) in the same commit.
   Group it under the right "intent" section (chrome / navigation / editing / visual / code).
5. **Declare what all three clients do with it** in `src/support.rs`.
   You do not get to skip this: `support()` is an exhaustive `match`, so the crate does not compile until the row exists.
   Then wire the handlers the row promised:
   - TUI: `crates/outl-tui/src/input/normal.rs` (its own `match`, not `lookup` — see the note under "What this crate does NOT own").
   - Desktop: `crates/outl-desktop/src/lib/action-handlers.ts`.

   > **This step used to read:** *"A client that doesn't need the action just doesn't add a handler — `lookup` returns `Some(Action::Foo)` and the dispatcher no-ops with a debug log."*
   >
   > That instruction is the defect. A no-op with a debug log is indistinguishable, from the keyboard, from a broken feature — and it left `y r` and `:` documented as desktop chords with no handler behind either.
   > A client that does not perform the action declares `Missing` / `NotApplicable` **with the sentence the user is shown**, and the dispatcher surfaces it. Silence is not an option the row offers.

6. Regenerate the parity doc and run both gates:

   ```sh
   OUTL_UPDATE_PARITY_DOC=1 cargo test -p outl-shortcuts
   cd crates/outl-desktop && npx vitest run src/lib/shortcuts.support.test.ts
   ```

   The first pins `docs/client-parity.md` to the code; the second pins the desktop handler map to the catalog **in both directions** — a promised handler that is missing, and a shipped handler the catalog still calls absent.
7. **Update both user-visible tables in the same commit:**
   - `crates/outl-desktop/CLAUDE.md` "OS-standard chrome" / "Block-editor chords" / "Inline markdown" — whichever the chord belongs to.
   - `docs/tui.md` if the binding has a TUI counterpart.
   - This `CLAUDE.md`'s mode-semantics example list if the chord is a load-bearing illustration (e.g. the `Cmd+B`-in-Insert vs. `Cmd+B`-in-Global rationale below).

## Mode semantics

| Mode | Meaning | Subscribed by |
|---|---|---|
| `Global` | Always active. Chrome chords (`Cmd+P`, `Cmd+J`, `Cmd+T`, `Cmd+,`). | Every client. |
| `Normal` | Vim Normal mode — outline navigation (`j`, `k`, `i`, `o`, `dd`). | TUI always; desktop when `vim_mode == true`. |
| `Insert` | Inside a textarea / EditBuffer. Movement (`Up`, `Down`, `Left`, `Right`), commit (`Esc`), inline-markdown wrappers (`Cmd+B`, `Cmd+I`). | TUI + desktop. |
| `Visual` | Range selection (vim Visual). | TUI; desktop via `V` (vim) **or** `Shift+↑/↓` (`SelectRange*`, any mode — issue #23). |
| `Overlay` | Picker / settings modal / help is open. | TUI + desktop — used to give the overlay its own escape chord without it colliding with `Normal` mode bindings. |

**`SelectRangeDown` / `SelectRangeUp` are the "enter a mode from another mode" example** (issue #23).
Bound in **both** `Normal` and `Visual`, they start a contiguous selection from Normal — the desktop's non-vim multi-select entry, reached via its DOM "nothing focused" → Normal fallback — and keep extending it once Visual is active.
`MoveVisualRange{Up,Down}` are Visual-only, mirroring the single-block `MoveBlock{Up,Down}` chords one mode over.

**`Cmd+B` is the canonical "context-dependent chord" example:**

- `Global` mode → no binding (we want bold-in-insert to take priority).
- `Insert` mode → `WrapBold`.
- `Normal` mode → no binding either (the desktop frontend used to wire it to "toggle sidebar"; that's `Cmd+Shift+E` now to avoid hijacking markdown's universal bold chord — see `crates/outl-desktop/CLAUDE.md` for the rationale).

If you find yourself wanting two different actions on the same chord across modes, the catalog already supports it — just add two `Binding` rows with different `mode` fields and the `no_duplicate_chord_in_same_mode` test will let them through.
`Cmd+Shift+X` ships exactly that split today: `WrapStrike` in Insert (textarea focused) and `RunCodeBlock` in Global — inside a textarea the mode-specific row wins, everywhere else the Global one fires.
`Cmd+Shift+Enter` is a second example (issue #92): `CommitAndContinue` in Insert (commit the edit, start the next block) vs. `NewBlockBelow` in Normal (view mode — create a sibling below without a `vim_mode` requirement).
It is also dual-spelled per OS — `Cmd+Shift+Enter` (META) and `Ctrl+Shift+Enter` (CTRL) — bound twice in each mode because the desktop adapter never rewrites `Cmd`↔`Ctrl`, the same two-row pattern `Cmd/Ctrl+P` uses.
Both splits are pinned by tests (`cmd_shift_x_splits_between_insert_and_global`, `cmd_shift_enter_splits_between_insert_and_normal`) so a future reorder of `default_bindings()` can't silently collapse them.

**`Cmd+Z` / `Cmd+X` are the canonical "don't shadow the OS" examples:**

- Plain `Cmd+X` carries **no binding at all** — it used to be `RunCodeBlock` (Global), which `preventDefault`ed the OS-universal cut inside every desktop textarea (issue #80).
  Run-code lives on `Cmd+Shift+X` now.
- `Cmd/Ctrl+Z` (Undo) and `Cmd/Ctrl+Shift+Z` (Redo) are bound in **Normal**, not Global, so a focused textarea keeps the chord for its own native undo instead of having the dispatcher swallow it.
  They sit next to the vim spellings (`u` / `Ctrl+R`) in the catalog.

**`Cmd+X` / `Cmd+C` / `Cmd+V` are the "OS-native vs. structural" example:**

- `Insert` mode → **no binding**.
  Inside a block editor the webview's native text cut / copy / paste must win, so the catalog stays silent and the dispatcher lets the keystroke through.
  A text-editing app that swallowed `Cmd+X` would be hostile, revisiting the old "X for execute" decision.
- `Normal` (view) mode → `CutBlock` / `CopyBlock` / `PasteBlock` — act on the whole selected block + subtree.
  These are **`Normal`, not `Global`**: a `Global` binding would shadow the native text cut inside a textarea (the desktop's Insert→Global dispatch fallback).
  Keep them out of `Global`.
- The desktop reaches `Normal` via its DOM-detected "nothing focused" state, so these fire in view mode **whether or not `vim_mode` is on** — they're view-mode gestures, not vim gestures.
- Each is **dual-spelled per OS**: `Cmd+X/C/V` (META) and `Ctrl+X/C/V` (CTRL), plus `Cmd/Ctrl+Shift+↑/↓` for the reorder.
  Both spellings are bound in `Normal` because the desktop adapter never rewrites `Cmd`↔`Ctrl`.
  Drop the CTRL row and the chord dead-keys on Linux / Windows, the same two-row pattern `Cmd/Ctrl+Z` uses.

`RunCodeBlock` stays **`Global` `Cmd+Shift+X`**, off plain `Cmd+X` (now cut).
It vacated `Cmd+X` so the native/structural cut owns it.
On `Cmd+Shift+X` the `Insert` `WrapStrike` row wins inside a textarea and the `Global` row fires everywhere else, the `Cmd+Shift+X` split described above.

## Wire format (Tauri / JSON)

The desktop ships the whole binding table to the frontend on boot via the `list_shortcut_bindings` Tauri command (`crates/outl-desktop/src-tauri/src/commands/shortcuts.rs`).
Serde format is stable and load-bearing:

```json
{
  "mode": "Global",
  "chord": { "chords": [{ "modifiers": "META", "key": { "Char": "p" } }] },
  "action": { "kind": "OpenPicker" },
  "description": "Quick switcher"
}
```

- `Action` uses `#[serde(tag = "kind")]` — the desktop's `switch` over the string discriminant compiles to a fast dispatch in JS.
- `ChordSequence` is `{ "chords": [Chord, …] }` even for single-chord bindings (so the JS side has one shape to handle).
- `Modifiers` ships as a string of `|`-joined flag names (`"META"`, `"META|SHIFT"`).

**Don't change these names** — the desktop frontend lives in pure TypeScript without a `bindgen` step.
A field rename here is a silent frontend break.

## Verify before "done"

```bash
cargo fmt --all
cargo clippy -p outl-shortcuts --all-targets -- -D warnings
cargo test -p outl-shortcuts
```

If you added a new `Action` or changed the chord for an existing one:

```bash
OUTL_UPDATE_PARITY_DOC=1 cargo test -p outl-shortcuts   # regenerate docs/client-parity.md
cargo test -p outl-tui                                  # input/* tests + dispatch coverage
cd crates/outl-desktop && npx vitest run                # includes shortcuts.support.test.ts
```

And smoke-test the TUI + desktop manually — the help overlay should list the new entry, and the chord should either fire or **tell the user why it didn't**.

> A resolved action in the debug log used to be the pass criterion here.
> It isn't: a chord that resolves and no-ops looks identical to a broken one from the keyboard, which is what `support.rs` exists to end.
> If the client can't perform the action, what you should see is the catalog's sentence in the status line — not a clean console.
