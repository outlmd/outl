# RFC 0070 — One catalog owns every chord, and the desktop has no character cursor

| | |
|---|---|
| **Status** | Shipped (char-cursor parity deliberately unbuilt — see Scope) |
| **Issue** | [#70](https://github.com/outlmd/outl/issues/70), [#80](https://github.com/outlmd/outl/issues/80); supporting: #92, #23, #184, #119, #41, #183 (all linked in [Why](#why)) |
| **PR** | — |
| **Date** | 2026-08-06 |
| **Reference doc** | [shortcuts.md](../shortcuts.md) |
| **Invariant** | root `CLAUDE.md` → "`outl-shortcuts` is the single (chord → action) catalog" and "What you're NOT building yet" → character cursor in desktop Normal mode |
| **Guarded by** | `no_duplicate_chord_in_same_mode`, `cmd_shift_x_splits_between_insert_and_global`, `cmd_shift_enter_splits_between_insert_and_normal`, `nudges_are_written_for_the_user_not_the_developer`, `the_parity_doc_matches_the_code` (`crates/outl-shortcuts/src/`), `"wires no handler for an action the catalog calls absent"`, `"wires a handler for every action the catalog says it supports"` (`crates/outl-desktop/src/lib/shortcuts.support.test.ts`) |

## Why

Eight issues over roughly a year, filed by users who had no idea they were reporting the same thing.

The oldest one is the reason the catalog exists at all: the TUI defined its bindings inside `outl-tui/src/input/`, the desktop wired its own `KeyboardEvent` handlers, and `Cmd+P` and `Ctrl+P` **drifted within a sprint**.
Nobody chose that divergence — it is just what happens when a chord table has two homes.

What the users actually reported:

- **A block containing a `[[ref]]` could not be edited from the keyboard at all** ([#70](https://github.com/outlmd/outl/issues/70)).
  `Enter` in Normal mode fires `OpenRefUnderCursor`, and the desktop, having no character cursor, approximated "the ref under the cursor" as "the *first* ref token in the block".
  So every ref-carrying block navigated away instead of entering Insert.
- **`Cmd+X` never cut text** ([#80](https://github.com/outlmd/outl/issues/80)).
  The catalog bound plain `Cmd+X` to `RunCodeBlock` in Global mode.
  The desktop dispatcher matches Global bindings even with focus inside a block's `<textarea>`, calls `preventDefault()`, and the OS-universal cut is swallowed — by design, from the catalog's point of view.
  `Cmd+Z` was equally dead, and `Cmd+C`/`Cmd+V` working made it read as a bug rather than a decision.
- **A non-vim user had no keyboard route to a new block in view mode** ([#92](https://github.com/outlmd/outl/issues/92)).
  The only path was vim's `o`, gated behind a setting most users never enable.
- **Pressing `Enter` mid-text did not split the block** ([#184](https://github.com/outlmd/outl/issues/184), TUI) — "There seems to be no way to split a block".
  The user tried `r`+`Enter`, `I`+`Enter` and `s`+`Enter` before filing.
- **Pressing `Enter` in the Mac app inserted a newline instead of creating a block** ([#119](https://github.com/outlmd/outl/issues/119)).
- **No arrow-key navigation between blocks on the desktop** ([#41](https://github.com/outlmd/outl/issues/41)) — you typed a block, then reached for the mouse.
- **No way to open a `[text](url)` markdown link from the TUI** ([#183](https://github.com/outlmd/outl/issues/183)), with a specific ask: `gx`, and keep code-block execution winning when a fence is under the cursor.
- **No batch operations on touch** ([#23](https://github.com/outlmd/outl/issues/23)) — the TUI had Visual mode, mobile had per-block taps only.

Read individually, these are eight small bugs.
Read together they are one fact with a corollary.
The fact: a chord's meaning has to be decided in exactly one place, and that place has to know what *mode* the user is in.
The corollary: the desktop's Normal mode holds only a block id, so any chord whose meaning depends on a character position cannot mean the same thing there as in the TUI.

The two most expensive issues, #70 and #80, are both instances of the corollary being papered over — once by guessing a cursor position, once by letting a Global chord shadow a native text-editing key.

## What we chose

**`outl-shortcuts` is the single (chord → action) catalog, and mode is part of the key.**
`crates/outl-shortcuts/src/defaults.rs` is the only file where a chord is bound.
Clients fetch it — the desktop over the `list_shortcut_bindings` Tauri command at boot — and wire each `Action` to a handler.
`lookup(mode, chord)` prefers the mode-specific row over the `Global` one, which is what makes "same chord, two meanings" expressible instead of a collision.

Three splits carry the weight, and each was a bug before it was a design:

| Chord | Insert (textarea focused) | Normal / Global (no textarea) |
|---|---|---|
| `Cmd+Shift+X` | `WrapStrike` | `RunCodeBlock` (Global) |
| `Cmd+Shift+Enter` | `CommitAndContinue` | `NewBlockBelow` (Normal, no `vim_mode` needed) |
| `Cmd+X` / `Cmd+C` / `Cmd+V` | *unbound* — falls through to native text editing | block cut / copy / paste (structural) |

Plain `Cmd+X` carries **no binding at all**, and `Cmd/Ctrl+Z` / `Cmd/Ctrl+Shift+Z` are bound in **Normal**, not Global, so a focused textarea keeps them for its own native behaviour instead of having the dispatcher swallow them.
"Don't bind it" is a decision the catalog has to be able to express, and #80 is what taught us that.

**The desktop's missing character cursor is a stated capability gap, not a bug backlog.**
Vim ops are categorised by what they need from the cursor model.
Block-level ops (`a`, `A`, `S`, `Y`, `*`, `#`, `z R`, `z M`, `z z`, `V`, `g v`, Visual `>` `<` `y` `d`) work on `selectedBlockId` and are implemented.
Char-cursor ops (`x`, `X`, `D`, `C`, `s`, `r`, `f`, `F`, `~`, `e`) route to **one** message, `why::NO_CHAR_CURSOR` in `crates/outl-shortcuts/src/support.rs` that puts a status-line message pointing at `i` plus textarea edits.
Their catalog entries stay so the help overlay still lists them.
One nudge function rather than ten messages is the point: there is a single place that says "this needs a cursor we don't have".

**#70's fix is behavioural, not a better guess.**
`Enter` on a selected block enters edit mode, refs or not, matching every GUI outliner.
Following a ref on the desktop is the click affordance (`onRefClick`), and the read-only backlink row keeps its open-the-source behaviour.
The TUI keeps the cursor-sensitive version, because it has a cursor: `try_open_under_cursor` follows a ref only when `ref_at_cursor` says the character cursor is on one, and falls back to Insert otherwise.
Same action name, honest per-client semantics, one catalog row.

**Semantics that are not chord lookups live in `outl-actions` or `outl-md`, once.**
`outl_actions::split_block` (`crates/outl-actions/src/block/split.rs`) is the single owner of "press Enter in the middle of a block" for #184, exposed to both GUI clients through the shared `splitBlock` wrapper in `@outl/shared`.
`outl_md::outline_ops::insert_sibling_after_with_text` is the TUI's in-flight-AST equivalent, kept in `outl-md` because it operates on an AST that has not been parsed back into a `Workspace` yet.
The TUI's `gx` for #183 goes through the pure `decide_gx`, which encodes the requested precedence: a fence under the cursor runs, otherwise a markdown link opens.
Its `http`/`https`/`mailto` scheme guard mirrors the desktop's `openExternalUrl`.
`SelectRangeDown` / `SelectRangeUp` for #23 are bound in **both** Normal and Visual, so one machinery serves vim users and the desktop's non-vim multi-select.

**One owner per fact for the tables themselves.**
Every chord lives in [`docs/shortcuts.md`](../shortcuts.md).
The five `CLAUDE.md` files that touch keybindings link to it and carry only the architectural notes a contributor needs before editing that crate.

## Why not the alternatives

**Give the desktop a character cursor and get full vim parity.**
The complete fix, and it is the one thing this RFC deliberately refuses.
It costs a second cursor model: the desktop's Normal mode would need to track a character offset inside a block whose text is currently owned by a controlled `<textarea>` that resets its own state on every keystroke.
That is a real feature with real design work, and doing it under the pressure of ten stubbed vim ops is how you get a cursor that disagrees with the textarea's.
A nudge is honest about a gap; a half-tracked cursor is #70 again with more surface.

**Let each client bind its own keys and just be careful.**
This is what the project actually did, and `Cmd+P` versus `Ctrl+P` drifted inside one sprint.
"Be careful" has no failing test, and the divergence is invisible until a user files a bug about one platform.

**Approximate the missing cursor — "the first ref in the block" for #70.**
Ships fast, no new state, and it turned "open the ref under the cursor" into "no block containing a ref can ever be edited from the keyboard".
Approximating a cursor position produces confident wrong answers, which is worse than a nudge that says nothing happened.

**Keep `Cmd+X` on `RunCodeBlock` and document it.**
It had a mnemonic ("X for execute") and it was documented as deliberate.
In an app that is half text editor, shadowing the OS-universal cut costs more than any mnemonic buys, and the cost lands on users who never read the shortcut doc.
Run-code moved to `Cmd+Shift+X`; strikethrough did **not** move, because it follows the Slack/Discord convention users already have in their fingers.

**Reuse `CommitAndContinue` for view-mode new-block (#92).**
Fewer actions, and the wording lies: in view mode there is no in-flight edit to commit.
A distinct `NewBlockBelow` in Normal keeps the two rows readable in the help overlay and lets the desktop handler skip the commit path entirely.

**Bind `Enter` in Normal to `NewBlockBelow` and be done (#184).**
Tempting, because "create a sibling below" is what the code already did.
It costs the actual request: a user who pasted a paragraph needs to *split* it, and there was no other route.
`split_block` with the caret at the end degenerates to exactly the old behaviour, so splitting is a superset rather than a replacement.

**Handle `Shift+Enter` in the catalog (#119).**
The soft-break case lives in `BlockRow`'s `handleKeydown` rather than in `defaults.rs`, with a code comment saying so.
It is a textarea-local text insertion, not an application action, and putting it in the catalog would mean the dispatcher `preventDefault`s a keystroke whose only job is to type a `\n`.

**Bind `Enter` to open markdown links in the TUI (#183's own alternative).**
The reporter suggested it and then argued against it themselves: `gx` is what vim uses in `.md` files, including from the text half of a link.
`Enter` is already ref-following, and overloading it would put link-opening in the same chord as page navigation with no way for the user to predict which fires.

## The opposite direction

**Refusing to guess the cursor means ten chords do nothing but talk.**
`x`, `X`, `D`, `C`, `s`, `r`, `f`, `F`, `~` and `e` are listed in the desktop help overlay and produce a status-line message.
A vim user who knows those keys gets told to press `i` instead, every time, forever.
That is a permanent papercut traded against #70's class of bug, and it is worse for the user who *only* uses the desktop — the TUI user never notices.

**The mirrored case of "Enter edits instead of following the ref":** the user who wanted to follow the ref from the keyboard now cannot.
On the desktop the only route is the mouse (`onRefClick`), and the status line does not say so.
The TUI keeps cursor-sensitive following, so the two clients now *deliberately* disagree about what `Enter` does on a ref-carrying block, and the help overlay shows one description for both.
That asymmetry is the price of #70 and it is not signposted to the user anywhere.

**The mirrored case of "`Enter` splits at the caret":** `Enter` now means different things on two clients for the same catalog row.
The desktop splits at the caret; the TUI splits at the cursor; and a client without a caret would have to pick one.
Worse, the split is only as good as the caret offset the client sends — `charOffset` is a codepoint offset and a textarea reports UTF-16, so a client that forgets `utf16OffsetToCharOffset` splits emoji and CJK text in the wrong place.
`split_respects_utf8_char_boundaries` pins the backend; nothing pins the conversion at each call site.

**Freeing `Cmd+X` moved a collision rather than removing one.**
`Cmd+Shift+X` is now `WrapStrike` in Insert and `RunCodeBlock` in Global, resolved by mode precedence.
The consequence: you cannot run a code block you are currently editing — you commit with `Esc` first.
Nothing tells you that; the strikethrough just applies.

**Binding undo in Normal rather than Global means undo silently does nothing while you type.**
That is correct (the draft is the textarea's own undo domain) and it is also #80's unfixed half: the controlled `value={draft()}` binding resets the native undo stack on every keystroke, so *neither* undo works inside an in-flight draft.
The user experiences one dead key and two different reasons for it.

**A catalog that can express "two meanings, one chord" can also express an unresolvable one.**
`no_duplicate_chord_in_same_mode` only catches duplicates *within* a mode.
Two rows in different modes always pass, so a genuinely ambiguous pair is a design error the test cannot see — which is why the two intentional splits have their own dedicated tests rather than relying on the collision check.

## How it cannot regress

1. **Invariants.**
   The root `CLAUDE.md` "Decisions you don't get to revisit" table states that `outl-shortcuts` is the single catalog, and carries the *why* in the row itself.
   Two parallel implementations is the bug that was paid to remove, `Cmd+P` and `Ctrl+P` drifted within a sprint, and adding a key on any client without going through `defaults.rs` puts that drift back.
   The root "What you're NOT building yet" list states the desktop char-cursor gap and names the ten chords that nudge, so a contributor cannot read the stubs as unfinished work to helpfully finish.
   `outl-shortcuts/CLAUDE.md` carries the mode-semantics table and uses `Cmd+B`, `Cmd+Shift+X`, `Cmd+Shift+Enter`, `Cmd+Z` and `Cmd+X` as the worked examples.
   `outl-desktop/CLAUDE.md` carries the three-category vim breakdown and the deliberate Normal-not-Global reasoning for undo.
   `outl-tui/CLAUDE.md` carries the `Enter`-splits contract including both degenerate cases.
   `outl-md/CLAUDE.md` records why `outline_ops` sits in `outl-md` rather than `outl-actions`.

2. **Tests.**
   - `no_duplicate_chord_in_same_mode` (`crates/outl-shortcuts/src/lib.rs`) is the anti-drift floor for the catalog.
   - `cmd_shift_x_splits_between_insert_and_global` and `cmd_shift_enter_splits_between_insert_and_normal` (same file) pin the two intentional splits, so a future reorder of `default_bindings()` cannot silently collapse them into one row.
   - `global_chrome_shortcuts_resolve_in_every_mode` (same file) pins the `Cmd+P` case that started all of this — the chrome chord has to resolve in Normal, Insert, Visual and Overlay.
   - `every_binding_has_a_description` and `bindings_round_trip_via_serde` (same file) keep the catalog usable by a client that only sees it over the wire, which is how the desktop consumes it.
   - `crates/outl-desktop/src/lib/action-handlers.test.ts` pins #70 by name — `Enter` enters Insert on a ref-carrying block, on a ref-free block, does nothing with no selection, and opens the source page from a backlink row.
     The same file pins the #80 block clipboard: cut only arms, paste routes cut to `moveBlockAfter` and copy to `pasteBlockAfter`, pasting a cut onto itself is a no-op.
     It also pins #23's range selection and the Visual batch ops.
   - `crates/outl-desktop/src/components/BlockRow.test.tsx` pins #119 and #184 together.
     Plain `Enter` fires `onEnter` and prevents the newline, plain `Enter` mid-text passes the caret offset so the backend splits there, and `Shift+Enter` does **not** fire `onEnter`.
   - Seven tests in `crates/outl-actions/src/block/split.rs` pin `split_block` — middle, end, past-end clamp, start, children staying with the head, UTF-8 boundaries, and a node not in the tree.
   - Five tests around `decide_gx` (`crates/outl-tui/src/actions/exec.rs`) pin #183's precedence.
     A code block runs even with a link in it, a non-code block opens the link under the cursor, a cursor off the link does nothing, and the scheme guard allows only web and mail.

   **Two gaps, named.**
   `charCursorNudge` **no longer exists**, and the gap it left is closed by what replaced it.
   Its message moved into `outl_shortcuts::support`, where the ten char-cursor actions are `Missing(why::NO_CHAR_CURSOR)` on the desktop, and `shortcuts.support.test.ts` fails if any of them acquires a handler — which is the "wired to a block-level approximation" case this line used to say nothing would catch.
   The wording is pinned too: `nudges_are_written_for_the_user_not_the_developer` rejects developer vocabulary, because the state before all this was a `console.warn` in a comment that called DevTools output something "the user sees".
   Arrow-key block navigation from #41 has **no test — none found, gap** — the edge-aware first-line/last-line crossing is unpinned.

## Scope

**Not covered — a character cursor in desktop Normal mode.**
Listed in the root `CLAUDE.md` under "What you're NOT building yet".
Until it exists, the ten char-cursor vim ops nudge.

**Not covered — pending-input chords.**
`r{ch}`, `f{ch}` and `F{ch}` need the dispatcher to read a second character before applying.
No machinery for that exists on the desktop; they are categorised with the char-cursor ops because they are blocked either way.

**Not covered — per-keystroke undo inside an in-flight draft.**
The controlled `value={draft()}` binding invalidates the textarea's native undo stack, and auto-pair, suggestion acceptance and `markdown-wrap.ts` all write `ta.value` directly.
[#80](https://github.com/outlmd/outl/issues/80) delivered block-level undo and explicitly deferred this.

**Not covered — the chord tables themselves.**
[`docs/shortcuts.md`](../shortcuts.md) is the canonical home for every chord.
This RFC deliberately lists none of them beyond the three splits it exists to explain.

**Not covered — mobile gesture design.**
[#23](https://github.com/outlmd/outl/issues/23)'s long-press entry, haptics and batch toolbar are client chrome.
Only the shared `SelectRange*` actions and the batch mutations they drive are in scope here.
