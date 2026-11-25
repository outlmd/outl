# Shortcuts

Every keyboard shortcut outl ships with, across the terminal app (TUI), the desktop app and the phone app, in one table per task.

If you are new, learn these ten first.
They work the same way in every client that has a keyboard.

| Do this | TUI | Desktop |
|---|---|---|
| Jump to any page | `Ctrl+P` | `Cmd/Ctrl+P` |
| Open today's journal | `t` | `Cmd/Ctrl+J` |
| Move between blocks | `j` / `k` | `j` / `k` |
| Edit the selected block | `i` | `i` |
| Leave the editor | `Esc` | `Esc` |
| New block below | `o` | `o` |
| Indent / outdent | `Tab` / `Shift+Tab` | `Tab` / `Shift+Tab` |
| Cycle TODO, DOING, DONE | `Ctrl+T` | `Cmd/Ctrl+T` |
| Undo | `u` | `u` or `Cmd/Ctrl+Z` |
| Show every key | `?` | `?` with nothing focused |

The in-app help is the fastest lookup while you work: `?` in the TUI and on the desktop (with nothing focused).
This page exists so you can see everything without launching the app.

**You cannot rebind keys yet.**
The chords below are fixed.
A plugin can add its own chords through `contributes.keybindings` (see [`plugins.md`](plugins.md)), and a plugin chord never replaces a built-in one on the desktop.

**Which client can do what** is answered in [`client-parity.md`](client-parity.md), which is generated from the code.
This page says **which key**; that one says **whether anything happens**.
When a key does nothing in a client, the status line tells you what to do instead, and the same sentence is in that table.

> **Why one catalog owns every chord, and why the desktop has no character cursor:** [RFC 0070](rfcs/0070-keybinding-ownership-and-vim-parity.md).

If a row below disagrees with what you observe in the app, **the code is right and this page is stale**.
File an issue or fix the row.
The tables here are written by hand.
Only [`client-parity.md`](client-parity.md) is generated.

## How to read the tables

- **Chrome / Global** chords fire in every mode.
- **Normal / Insert / Visual / Overlay** mirror the vim modes.
  The desktop dispatcher never reads `editor.vim_mode` (see [`docs/config.md`](config.md)), so every mode is live in every case: with no textarea focused it falls into `Normal`.
- **`Cmd`** is the macOS modifier; **`Ctrl`** is the same chord on Linux / Windows / TUI.
  We list one form per row to keep the table readable.
- A chord in the form `q q` is a vim-style two-key sequence: press the first, then the second.
  On the desktop the pending first key expires after 1 s; the TUI waits for the next key with no timeout.
  In the TUI, a second key that matches nothing is not swallowed: it runs as if you had pressed it alone, so `g t` ends up on today's journal through plain `t`.

---

## Chrome (Global): works in any mode

| Action | TUI | Desktop | Mobile |
|---|---|---|---|
| Quick switcher (fuzzy pages + journals) | `Ctrl+P` | `Cmd/Ctrl+P` _(pages, then blocks, then matching actions; type `>` for actions only)_ | tap toolbar |
| Open today's **j**ournal | `t` / `g j` (chord) | `Cmd/Ctrl+J` | toolbar |
| Cycle task state on focused or selected block, `(none) → TODO → DOING → DONE → (none)`, one stop per press (T for **t**ask) | `Ctrl+T` / `Ctrl+Enter` | `Cmd/Ctrl+T` / `Cmd/Ctrl+Enter` | tap checkbox |
| Mark the focused or selected block **DONE** outright from any state, idempotent when already done (D for **d**one). TUI on a query-result embed row (`!((blk-…))`): resolves the handle and completes the *source* block; desktop refuses the row with a pointer to the source | `g D` (chord) | `g D` (Normal) | _(no chord, tap the checkbox through to DONE)_ |
| Run code block under cursor / selected block (X for e**x**ecute). TUI: when the block isn't code, `g x` instead opens the markdown link `[text](url)` under the cursor, an asset link in the OS default app, anything else in the browser (issue #183) | `g x` chord / `:run` | `Cmd/Ctrl+Shift+X` (inside a textarea the Insert-mode strikethrough wins, commit first or use the Run button; plain `Cmd+X` is the OS cut / block cut) | tap "Run" button |
| Previous journal day | `[` | `Cmd/Ctrl+[` | swipe right |
| Next journal day | `]` | `Cmd/Ctrl+]` | swipe left |
| Open **yesterday's** / **tomorrow's** journal (anchored on today, not on the page you are reading: on a 2020 page `[` means 2020, `g [` still means yesterday) | `g [` / `g ]` | `g [` / `g ]` (Normal) | quick switcher: "Open yesterday's journal" / "Open tomorrow's journal" |
| Export the workspace as plain Markdown (copies it out; the `.md` files on disk are already plain Markdown) | _not in the terminal_; `outl export md <page>` exports one page from a shell (see [`cli.md`](cli.md)) | quick switcher: "Export as Markdown" | Devices sheet, Export card |
| Toggle sidebar | `Ctrl+E` | `Cmd/Ctrl+Shift+E` | _(single pane)_ |
| Toggle focus mode (outline only) | `z f` (chord; `F` is find-backward and a terminal sees `Ctrl+Shift+F` as `Ctrl+F`) | `Cmd/Ctrl+Shift+F` | focus button in the capsule |
| Quick capture to today | _open today and add a block_ | `Cmd/Ctrl+Shift+Space` (from anywhere) | capture button / `outl://capture` |
| Sync panel (pending per device, merged edits, undo) | `:sync` | Settings, Sync | Devices sheet |
| Toggle backlinks panel | `Ctrl+B` | `Cmd/Ctrl+Shift+B` | inline below outline |
| Toggle backlinks order (newest/oldest) | `Ctrl+O` | direction button in the backlinks header | direction button in the backlinks header |
| Open settings | _via `:settings`_ | `Cmd/Ctrl+,` | gear icon |
| Toggle help overlay | `?` | `?` (nothing focused) | help button |
| Quit | `q q` (chord) / `Z Z` (vim alias) / `Ctrl+C` | `Cmd/Ctrl+Q` (OS) | none |

> **Why `Cmd+J` and not `Cmd+T` for today's journal?** Every outliner ecosystem uses `T` for *task* / TODO, TUI's `Ctrl+T`, Logseq's `Cmd+T`, the universal Markdown checkbox shortcut. Re-training that muscle memory would be hostile. `J` for **journal** is unambiguous and lines up with the TUI's `g j` chord.

> **`Ctrl+T` on a query-result row (TUI).** A [query](query.md) result is a block whose whole text is one `!((blk-…))` embed. Cycling it acts on the *source* block, never prefixing the token, which would destroy the reference, then re-runs the page's query so the list updates on the spot.

**Defaults the user often asks about.**
Both clients ship with **sidebar and backlinks panel HIDDEN** (`show_sidebar: false`, `show_backlinks: false`).
Editor-hero on first launch, the user opts the panels in with the chord.
This matches Bear / Ulysses on the desktop and `outl-tui`'s historical behaviour.

### Why the shifted variants for sidebar / backlinks

| Chord | Why _not_ |
|---|---|
| `Cmd+B` | Reserved for **bold** in Insert mode, every popular markdown editor (Notion, Obsidian, Discord, Slack, Typora) treats it that way. Hijacking would be hostile. |
| `Cmd+\` | macOS **1Password** global autofill. Stealing it breaks every 1Password user. |

So `Cmd+Shift+E` (VS Code's "Show Explorer") and `Cmd+Shift+B` are the canonical chrome chords on the desktop.
The TUI mirrors the spirit with `Ctrl+E` / `Ctrl+B` (most terminals collapse `Ctrl+Shift+letter` into `Ctrl+letter`, so both forms work identically).

---

## Inline markdown: Insert mode (textarea focused)

Wrap the current selection (or insert the delimiter pair around the caret).
Mirrors the convention every markdown editor on the planet ships.

| Action | TUI | Desktop | Mobile |
|---|---|---|---|
| Bold (`**…**`) | type `**` | `Cmd/Ctrl+B` | toolbar B |
| Italic (`_…_`) | type `_` | `Cmd/Ctrl+I` | toolbar I |
| Inline code (`` `…` ``) | type `` ` `` | `Cmd/Ctrl+E` | toolbar `<>` |
| Strikethrough (`~~…~~`) | type `~~` | `Cmd/Ctrl+Shift+X` | type `~~` |
| Link (`[label](url)`) | type `[` | `Cmd/Ctrl+K` | type `[` |

> The TUI has no chord for any of these, it renders the markup but never writes it, so the "type it" cell is the whole story there, not a fallback.
> Mobile's keyboard toolbar carries bold, italic and code and stops: `ToolbarAction` in `@outl/shared` is the entire button set, and it has no strikethrough and no link.
> This table claimed "toolbar S" and "toolbar 🔗" for both until the catalog was re-checked against that union.

> outl ships `_…_` as the canonical italic. The parser still accepts `*…*` for compatibility, but `.md` projections emit underscores.

---

## Outline navigation: Normal mode

The desktop dispatcher does not read `editor.vim_mode`: with nothing focused it falls into `Normal` regardless.
The TUI is vim-style by definition.

| Action | TUI | Desktop | Mobile |
|---|---|---|---|
| Selection down | `j` / `↓` | `j` / `↓` | tap block |
| Selection up | `k` / `↑` | `k` / `↑` | tap block |
| Enter Insert at end of block | `i` | `i` | tap block |
| Enter Insert at start of block | `I` | `I` | tap at start |
| Enter Insert one char past cursor (vim append) | `a` | `a` *(= `i`, no char cursor)* | none |
| Enter Insert at end of block (vim `A`) | `A` | `A` | none |
| Substitute block (clear + Insert at col 0; `S` / `cc`) | `S` | `S` | none |
| Substitute char under cursor (= `xi`) | `s` | none *(char cursor only)* | none |
| Yank current block to register + OS clipboard (`Y`, alias of `y y`) | `Y` | `Y` | long-press → "Copy text" |
| Paste OS clipboard **with** formatting (outline structure / multi-paragraph split) | `p` | `Cmd/Ctrl+V` | paste |
| Paste OS clipboard **without** formatting (raw text, single block) | `P` | `Cmd/Ctrl+Shift+V` | none |
| Open `[[ref]]` / `#tag` / `((blk-…))` under cursor | `Enter` | `Enter` *(= `i`, no char cursor, follow a ref by clicking the token)* | tap |
| New block below + Insert | `o` | `o` / `Cmd/Ctrl+Shift+Enter` | toolbar `+` |
| New block above + Insert (creates a sibling *before* the selected block) | `O` | `O` | long-press → "New block above" |
| Indent block | `Tab` | `Tab` | drag right |
| Outdent block | `Shift+Tab` | `Shift+Tab` | drag left |
| Move block up among siblings | `K` · `Alt+↑` · `Ctrl+Shift+↑` | `Cmd/Ctrl+Shift+↑` · `Alt+↑` · `Shift+K` | long-press menu |
| Move block down among siblings | `J` · `Alt+↓` · `Ctrl+Shift+↓` | `Cmd/Ctrl+Shift+↓` · `Alt+↓` · `Shift+J` | long-press menu |
| Cut block + subtree (move-by-id; paste keeps `((blk-…))` refs) | none | `Cmd/Ctrl+X` | long-press → "Cut block" *(fresh id on paste, see note below)* |
| Copy block + subtree (paste duplicates with fresh ids) | none | `Cmd/Ctrl+C` | long-press → "Copy block" |
| Paste block after the selection (cut → move, copy → duplicate) | none | `Cmd/Ctrl+V` | long-press → "Paste block" |
| Cancel a pending cut | none | `Esc` | none |
| Delete block (chord) | `d d` | `d d` | swipe left |
| Fold / unfold (toggle collapsed) | `c` | `c` | tap bullet |
| Unfold all on the page (chord) | `z R` | `z R` | tap header "Unfold all" button |
| Fold all on the page (chord) | `z M` | `z M` | tap header "Fold all" button |
| Center viewport on cursor (chord) | `z z` | `z z` | none |
| Zoom in on block (make it the outline root) | `z i` | `z i` / `Cmd/Ctrl+Shift+]` | tap bullet |
| Zoom out (back up one level toward the page) | `z o` | `z o` / `Cmd/Ctrl+Shift+[` | tap breadcrumb / back |
| Last block (jump) | `G` | `G` | none |
| First block (chord) | `g g` | `g g` | none |
| Reselect last Visual range (chord) | `g v` | `g v` | none |
| Search workspace for word / block text, forward | `*` | `*` *(seeds picker)* | page switcher "Blocks" tab *(no hit-stepping)* |
| Search workspace for word / block text, backward | `#` | `#` *(seeds picker)* | page switcher "Blocks" tab *(no hit-stepping)* |
| Undo last committed block mutation | `u` | `u` / `Cmd/Ctrl+Z` | keyboard toolbar |
| Redo | `Ctrl+R` | `Ctrl+R` / `Cmd/Ctrl+Shift+Z` | keyboard toolbar |
| Yank block ref → clipboard | `y r` / `/refer` | right-click the bullet → "Copy block ref" | long-press → "Copy block ref" |
| Yank block **embed** → clipboard | `/refer-embed` _(no chord)_ | right-click the bullet → "Copy block embed" | long-press → "Copy block embed" |
| Enter Visual | `V` _(the catalog spells this `v`; the TUI binds `V` only, [known divergence](../crates/outl-shortcuts/tests/tui_catalog_divergence.rs))_ | `v` | none |
| Open command palette | `:` | `:` / `Cmd/Ctrl+K` _(the quick switcher opened on `>`: actions with their chords)_ | none |
| Open slash menu | `/` _(overlay)_ | none _(the desktop's slash menu is inline, inside a block, see Insert mode below)_ | none _([#parity](client-parity.md): `SlashCommandMenu`)_ |

> **About `a` / `*` / `#` on the desktop.** The desktop's Normal mode has only a selected block id, no character cursor inside the block. So `a` collapses to `i` (the textarea's own caret takes over), and `*` / `#` seed the picker with the first few words of the selected block's text instead of doing a word-under-cursor search. The catalog still ships these chords so muscle memory from the TUI carries over.

> **`Cmd+X` / `Cmd+C` / `Cmd+V` are mode-aware on the desktop.** Inside a block editor (Insert mode, a `<textarea>` is focused) they are the OS-native text cut / copy / paste, the chords aren't in the catalog there, so the keystroke reaches the webview untouched. In **view mode** (Normal, nothing focused) they act on the whole selected block + its subtree: cut marks it to *move by id* (the paste emits a single `Op::Move`, so `((blk-…))` refs and backlinks survive, and the target may live on another page, moving the block across pages), copy snapshots it as markdown (the paste duplicates with fresh ids). This is also why **run code block** moved off `Cmd+X` to `Cmd+Shift+X` (view mode): a text-editing app has to let the OS-wide cut win.

> **Mobile's "Cut block" mints a fresh id, unlike the desktop's `Cmd/Ctrl+X`.**
> The desktop's block clipboard tags a cut with `{kind: "cut", nodeId}` and pastes it as a single `Op::Move`, so the block keeps its identity and every `((blk-…))` ref to it stays valid.
> Mobile's long-press "Cut block" instead renders the subtree to markdown and deletes the source in one round-trip, then pastes it back via the same fresh-id path its "Copy block" already uses, a `((blk-…))` ref pointing at the cut block goes stale across the round-trip.
> Use "Copy block ref" first if something else in the workspace links to the block you're about to cut.

> **`Cmd/Ctrl+Shift+Enter` works without vim mode.**
> With no textarea focused the desktop falls into Normal dispatch, and the dispatcher never reads `vim_mode`, so every user can append a block from view mode.
> Inside a block editor the same chord commits the current edit first (Insert-mode `CommitAndContinue`).
> Both `Cmd+Shift+Enter` (macOS) and `Ctrl+Shift+Enter` (Windows / Linux) are bound in each mode.

### Cursor inside a block (Normal)

These rely on a **character cursor inside the selected block**.
The TUI ships it natively; the desktop has only a selected block id, so the char-cursor ops surface a status-line nudge instead of firing.

| Action | TUI | Desktop |
|---|---|---|
| Char left / right | `h` / `l` (or arrows) | `h` / `l` |
| Word right / left | `w` / `b` | `w` / `b` |
| Word end forward (vim `e`) | `e` | none |
| Start / end of block text | `0` / `$` (or Home/End) | `0` / `$` |
| Find char forward / backward (next typed char) | `f{ch}` / `F{ch}` | none |
| Delete char under / before cursor | `x` / `X` | none |
| Delete to end of block (`D`) / change to end (`C`) | `D` / `C` | none |
| Replace char under cursor with next typed char | `r{ch}` | none |
| Toggle case of char under cursor; advance | `~` | none |

### On a backlink row

`j` / `k` cross the separator from the outline's last block into the "Linked from" rows, and a row there is **not read-only**.
Every chord below acts on the block the focused row draws, the citing block or one of its children, which the panel lists below it, on **its own** page, not on the page you are reading.

The chords are the ones the tables above already carry, this says what each of them *means* here, and nothing more.

| Operation | On a backlink row |
|---|---|
| Enter Insert (`i` / `I` / `a` / `Enter`) | edit the cited block in place; `Esc` commits and returns to the row |
| Cycle task state | flips the cited block's state, on its page |
| Mark DONE (`g D`) | completes the cited block, on its page |
| Delete block (`d d`) | deletes it, and its subtree (its children are drawn below the citing block, down to four levels) |
| Indent / outdent | reparents it among its own page's blocks |
| Move block up / down | reorders it among its own page's siblings |
| New block below / above (`o` / `O`) | creates the sibling **on the cited block's page** |
| Open the source page | nothing in the panel does it, `Ctrl+P`, then the page name from the row's header |
| Undo (`u`) | nothing here, the entry belongs to the page that changed, so open that page first |

Three of those have no visible result **on the citing block**, deliberately: indent, outdent and move reparent or reorder it among siblings the panel does not draw.
On a child row they do show, because its parent and siblings are on screen.
`o` / `O` on the citing block does not put the cursor in the new block either (on a child row it does, since the sibling lands inside the drawn children), because a brand-new sibling cites nothing and so is a backlink of no page, except when the page cites *itself*, where the block really is in the outline on screen.
The full reasoning, and why a cross-page edit **refuses** where an ordinary one re-targets: [`client-backlinks.md` → Editing a backlink where it is cited](client-backlinks.md#editing-a-backlink-where-it-is-cited).

On mobile there is no chord surface at all, so these arrive as a tap plus a long-press sheet: [`mobile-ux.md`](mobile-ux.md#backlink-rows--a-place-you-can-work).
**Mark DONE is the one row that does not cross over**, because `Action::MarkDone` is `Missing` on that client everywhere and not just here, the sheet toggles task state through to DONE instead, which is what its nudge in the catalog says.
Nothing on the phone reads that matrix yet, so the sentence is written and the user does not see it.
`Capability::BacklinkInlineEdit` therefore covers the other nine operations and leaves mark done to the action row that owns it; what its own TUI `Partial` records is the missing way to the source page, the row the table above just gained.

---

## Insert mode (text editing)

| Action | TUI | Desktop | Mobile |
|---|---|---|---|
| Commit + exit Insert | `Esc` | `Esc` / blur | blur |
| Newline inside the block (multi-line text) | `Shift+Enter` | `Shift+Enter` | `Enter` |
| Split block at cursor into two (caret at end → empty sibling below; caret at start → empty block above; caret mid-text → head stays, tail becomes the new sibling, caret lands at its start) | `Enter` | `Enter` | `+` in the toolbar |
| Commit + new block, caret-aware (caret at col 0 → *before* the block / vim `O`; past col 0 → *below*) | none | `Cmd/Ctrl+Shift+Enter` | none |
| Indent (stay in Insert) | `Tab` | `Tab` | drag |
| Outdent (stay in Insert) | `Shift+Tab` | `Shift+Tab` | drag |
| Move block up (stay in Insert, same block, caret kept) | `Alt+↑` · `Ctrl+Shift+↑` | `Cmd/Ctrl+Shift+↑` · `Alt+↑` | none |
| Move block down (stay in Insert, same block, caret kept) | `Alt+↓` · `Ctrl+Shift+↓` | `Cmd/Ctrl+Shift+↓` · `Alt+↓` | none |
| Delete block on empty | `Backspace` on empty | `Backspace` on empty | none |
| Caret to start / end of the current line | `Home` / `End` | native | native |
| Walk into the **previous** block, caret at its end | `←` at offset 0 | `←` at offset 0 | `←` at offset 0 _(hardware keyboard)_ |
| Walk into the **next** block, caret at its start | `→` at the end | `→` at the end | `→` at the end _(hardware keyboard)_ |
| Walk into the previous / next block keeping the column | `↑` / `↓` on the block's first / last line | same | same _(hardware keyboard)_ |
| Auto-pair `(` `[` `{` `[[` `((` | yes | yes | yes |
| Ref autocomplete | `[[` triggers picker | `[[` triggers picker | `[[` triggers picker |
| Tag autocomplete | `#` triggers picker | `#` triggers picker | `#` triggers chip strip |
| Block ref autocomplete | `((` triggers picker | `((` triggers picker | none |
| Slash command autocomplete | `/` _(word-initial)_ | `/` _(word-initial)_ | none _([#parity](client-parity.md): `SlashCommandMenu`)_ |
| Cycle task state on current (`TODO` → `DOING` → `DONE` → none) | `Ctrl+T` / `Ctrl+Enter` | `Cmd/Ctrl+T` / `Cmd/Ctrl+Enter` | tap checkbox / long-press menu |
| Cut / copy / paste **text** (native) | none | `Cmd/Ctrl+X` / `Cmd/Ctrl+C` / `Cmd/Ctrl+V` | native |
| Run code block | `g x` chord | _(commit with `Esc`, then `Cmd/Ctrl+Shift+X`)_ | tap "Run" |

> **The outline reads as one continuous document in Insert mode.**
> Holding `←` walks back through the page and `↑` at the top of a block lands inside the one above, without ever leaving Insert, the four rows above are one behaviour seen from four keys.
> `@outl/shared/caret-nav` is the single owner of the decision on the GUI clients and mirrors `outl_tui::input`'s `cross_block_nav_eligible` / `cross_block_step`, so the three clients agree on when a keystroke leaves a block and where the caret lands in the next one.
>
> Two limits are deliberate and shared.
> A **line** here is a logical line (a `\n` inside the block, as in a fenced code block), never a soft-wrapped visual one, so `↑` from the middle of a long wrapped block steps to the previous block rather than to the row above, a textarea exposes no per-visual-line caret API, and both ways to work around that are worse (that module's doc carries the reasoning).
> And the walk **stops at the outline's edges** instead of continuing into the backlinks section: that is another page, and opening it for editing from an arrow key aimed at this one is not what the user pressed.
> On mobile this only ever fires from a hardware keyboard, since a soft keyboard has no arrow keys.

---

## Visual mode (range)

TUI + desktop; mobile has no Visual equivalent yet.

On the desktop, `Shift+↑` / `Shift+↓` start (and keep growing) a contiguous selection **without** vim mode, the non-vim multi-select entry, painted at 18% accent opacity.
It flips the client into Visual and pops a floating **batch toolbar** (`N selected` + Indent / Outdent / Move up / Move down / Delete / Done) so the range ops are reachable by mouse; the toolbar fires the same actions the chords do.
Only the toolbar's **Delete** confirms before erasing a range that contains nested children; the keyboard delete (`d` / `x` / `Delete` / `Backspace`) and the TUI delete without a prompt, matching vim.

| Action | TUI | Desktop |
|---|---|---|
| Start / extend selection down | `j` / `↓` | `Shift+↓` (any mode) · `j` / `↓` (vim) |
| Start / extend selection up | `k` / `↑` | `Shift+↑` (any mode) · `k` / `↑` (vim) |
| Yank range | `y` | `y` |
| Delete range (toolbar **Delete** confirms if a block has children; the keys don't) | `d` / `x` | `d` / `x` · `Delete` / `Backspace` · toolbar **Delete** |
| Indent range (vim `>`) | `Tab` / `>` | `>` · toolbar **Indent** |
| Outdent range (vim `<`) | `Shift+Tab` / `<` | `<` · toolbar **Outdent** |
| Move range up among siblings | `K` · `Alt+↑` · `Ctrl+Shift+↑` | `Cmd/Ctrl+Shift+↑` · `Alt+↑` · `Shift+K` · toolbar **↑** |
| Move range down among siblings | `J` · `Alt+↓` · `Ctrl+Shift+↓` | `Cmd/Ctrl+Shift+↓` · `Alt+↓` · `Shift+J` · toolbar **↓** |
| Leave Visual (captures range so a follow-up `g v` restores it) | `Esc` | `Esc` · toolbar **Done** |

---

## Page operations

`g d` (Normal mode, "go delete") is the canonical chord for page deletion.
It lives in the shared `outl-shortcuts` catalog, same `g<action>` family as `g j` (today) / `g x` (execute) / `g p` (properties).
The chord deletes the focused page (sidebar-highlighted row when the sidebar has focus on the TUI, otherwise the current page).
Each client confirms before invoking `outl_actions::page::delete`.
Journals are refused everywhere.
Clients also expose the action through their native page-list affordance.

| Action | TUI | Desktop | Mobile |
|---|---|---|---|
| Delete the focused page (with confirmation) | `g d` (Normal mode) deletes the current page; sidebar `d` deletes the focused sidebar row | `g d` (Normal mode) + hover `×` button on the sidebar row | long-press the row in the page switcher |
| Pin / unpin the current page (`pinned:: true`; pinned pages sit in the sidebar) | `g P` or `/pin` | _not on the desktop yet: pin from the TUI_ | pin toggle on the page's row in the page switcher |

The desktop routes `g d` through the same `DeletePage` handler in `action-handlers.ts` (same `window.confirm` + `deletePage(slug)` flow as the `×` button).
Mobile has no keyboard surface.
Long-press in the page switcher remains the only trigger on touch devices.

---

## Reminders (`remind::`)

Authoring a reminder and inspecting the schedule are chorded on every client; **delivering** it is per-OS and never a chord.
Every client delivers, the TUI included (an OSC 9 notification plus a toast, on its event-loop tick).
What the TUI can't do is deliver while it's closed, since a terminal session has no background presence, see [reminders.md](reminders.md).

| Action | TUI | Desktop | Mobile |
|---|---|---|---|
| Add a reminder to the selected block | `g r` (writes `remind:: 9am`; reports the existing rule instead of overwriting) | `Cmd+R`, or `g r` in Normal | long-press the block → **Remind me…** |
| "Nag me" preset (`remind:: now every 1h until DONE`) | `g R` (overwrites, escalating is explicit) | `g R` in Normal | none |
| Open the reminders list | `g n` | `Cmd+Shift+R` / `Ctrl+Shift+R` | Reminders tab in the bottom bar |
| Open the task list (open `TODO` / `DOING`, by page, the first 200, with the uncapped count shown) | none (`query` block with `status: open`; `g t` lands on today's journal) | `g t`, or `Cmd+Shift+L` / `Ctrl+Shift+L` | Tasks tab |
| In the list: open the reminder's page | `Enter` | `Enter` / click the row | tap the row |
| Snooze the selected block 1h, without opening the list | `g s` | `g s` in Normal | the chips in the sheet (no chord surface) |
| In the list: snooze 1h | `s` | the `1h` / `tomorrow` / `next week` buttons | the `1h` / `Tomorrow` / `Next week` chips |
| In the list: close | `Esc` / `q` | `Esc` | drag down / tap outside |

> **Why `g n` in the TUI and not `Ctrl+R`?**
> `Ctrl+R` is already Redo in Normal mode, and a terminal cannot distinguish `Ctrl+R` from `Ctrl+Shift+R`.
> The `g` family (`g j`, `g x`, `g d`) is the honest home for it there; only the desktop takes the `Cmd/Ctrl+Shift+R` spelling.
> `Ctrl+R` on the desktop likewise stays Redo on Linux / Windows, which is why authoring is `Cmd+R` only.

Snoozing writes `Op::SnoozeRemind`, so it converges: silencing a nag on the phone silences the same block on the laptop.

---

## Properties (`key:: value`)

`remind::` above is one property with its own chords; this is the generic door.
Creating the **first** property on a block used to be impossible in either GUI client, you could edit a chip that already existed and nothing more (issue #13).

| Action | TUI | Desktop | Mobile |
|---|---|---|---|
| Open the property editor for the selected block | `g p` | `g p` in Normal, the chip row is always visible, so this opens the blank pair (same as `Cmd+Shift+P`) | long-press the block → **Properties…** |
| Add a property to the selected block | `o` inside `g p`, or `/prop <key> <value>` | `Cmd+Shift+P` / `Ctrl+Shift+P` in Normal, or the `+ prop` button at the end of the chip row (visible on block hover) | `+ Add property` at the top of the sheet |
| Edit a property's value | `Enter` inside `g p`, or `/prop <key> <value>` | click the chip | tap the chip |
| Delete a property | `d d` inside `g p`, or `/prop <key>` (empty value) | the `×` on the chip | swipe the row left, or Delete inside the editor |
| Add / edit a **page** property | `p` inside `g p` flips to page scope, or `/prop-page <key> <value>` | the chip row under the page title (always visible) | **long-press the title**, the Block ⇄ Page switch in the sheet, or the Properties button on an empty page |
| Complete the key from the workspace catalogue | `Tab` in the key field | typeahead on the key field | the most-used keys as tappable chips, plus **Other…** for the keyboard |
| Pick a `[[page]]` as the value | `[[` / `#` in the value field | `[[` in the value field opens the page list (↑/↓, Enter or click) | the `[[` picker |
| Close the editor | `Esc` / `q` | `Esc` | swipe the sheet down, or ✕ |

The key field completes from the workspace's own key catalogue (`outl_actions::known_keys`), most-used first, skipping keys the block already carries.
Deleting and "empty the value" are the same backend call, the `×` exists because a gesture nobody can see is not an affordance.

**The TUI gets an overlay rather than a form.** It is modal and vim, so tabbing between a key field and a value field fights the mode: `Tab` there means *complete*, and `Enter` is what advances key → value → save. `/prop` and `/prop-page` stay for muscle memory; the overlay is for when you don't remember the key.

> **`g p` used to mean "pin".** The `pinned::` toggle moved to **`g P`** (`/pin` is unchanged).
> Pinning is a once-per-page act with a second door already; editing properties is a daily one, and `pinned::` is itself one of the page properties `g p` now edits.

---

## Overlays (picker, palette, settings, help)

| Action | TUI | Desktop |
|---|---|---|
| Highlight next | `↓` / `Tab` / `Ctrl+J` | `↓` |
| Highlight previous | `↑` / `Shift+Tab` / `Ctrl+K` | `↑` |
| Confirm | `Enter` | `Enter` |
| Close overlay | `Esc` | `Esc` |

The picker (`Cmd+P` / `Ctrl+P`) fuzzy-matches pages and journals together; type a date in ISO (`2026-06-04`) or natural (`today`, `yesterday`) to jump.
On the desktop it also lists tasks (blocks with `TODO`, `DOING`, `DONE` or a checkbox) in their own section, and actions with no chord, such as "Export as Markdown".

Quick capture (`Cmd+Shift+Space` / `Ctrl+Shift+Space`) adds one line to the end of today's journal without leaving the page you're on. On the desktop the chord also works with outl in the background. `outl://capture` opens the same box on desktop and mobile.

---

## Keys only the TUI has

These chords work in the terminal app and are not in the shared catalog, so the desktop help overlay does not list them.
All are Normal mode unless noted.

| Keys | What it does |
|---|---|
| `h` / `l` (or `←` / `→`) | Cursor left / right inside the block |
| `w` / `b` | Cursor one word right / left |
| `0` / `$` (or `End`) | Cursor to the start / end of the line |
| `g g` / `G` | First / last block |
| `Ctrl+D` / `Ctrl+U` | Half page down / up |
| `PageDown` / `PageUp` | Page down / up |
| `y y` | Yank the current block |
| `p` / `P` | Paste the OS clipboard with / without formatting |
| `/` | Open the slash menu |
| `n` / `N` | Next / previous search hit after `*` or `#` |
| `V` | Enter Visual mode |
| `Ctrl+B` | Toggle the inline backlinks panel |
| `Ctrl+E` | Toggle the sidebar |
| `Ctrl+O` | Flip the backlinks order (newest / oldest) |
| `Ctrl+S` | Force a save |
| `Ctrl+L` | Reload the workspace from disk (in Insert, `Esc` first) |

---

## Where each chord lives in the code

| Layer | File | What it owns |
|---|---|---|
| Canonical catalog | `crates/outl-shortcuts/src/defaults.rs` | Every `(mode, chord, action, description)` row. |
| `Action` enum | `crates/outl-shortcuts/src/action.rs` | The named operation each chord resolves to. |
| TUI key dispatch | `crates/outl-tui/src/input/normal.rs` | Its own `match` on `crossterm::KeyEvent`. Does **not** call `lookup`. |
| TUI chord adapter | `crates/outl-tui/src/input/chord_adapter.rs` | `crossterm::KeyEvent → Chord`, for plugin keybindings only. |
| Desktop input adapter | `crates/outl-desktop/src/lib/shortcuts.ts` | `KeyboardEvent → Chord`. |
| Desktop dispatcher | `crates/outl-desktop/src/lib/action-handlers.ts` | `Action → Tauri command`. |
| Mobile toolbar / gestures | `crates/outl-mobile/src/components/` | Per-component on-screen handlers. |
| Per-client support | `crates/outl-shortcuts/src/support.rs` | Whether each client performs each action, and the sentence shown when it can't. Rendered in [`client-parity.md`](client-parity.md). |

A chord change is a single line in `defaults.rs` plus, if the action is new, a row in `action.rs` and a handler in each client.
See [`crates/outl-shortcuts/CLAUDE.md`](../crates/outl-shortcuts/CLAUDE.md) for the full add-a-binding checklist.

### Catalog versus the TUI's own keys

For contributors.
The desktop resolves every keystroke through `outl_shortcuts::lookup(mode, chord)`, so a change in `defaults.rs` reaches it on the next build.
**The TUI does not.**
It dispatches Normal-mode keys from its own `match` in `input/normal.rs`, and `input/chord_adapter.rs` exists only to match plugin chords.
The catalog and those arms can disagree without anything failing to compile.
Treat a TUI cell in the tables above as documentation of the `match`, not of the catalog.

They disagree on 20 chords, 6 of them destructive.
`Ctrl+X` is `CutBlock` in the catalog and deletes a character in the TUI.
`Ctrl+Shift+R` is `OpenReminders` in the catalog and redoes in the TUI.
`Ctrl+Z` is `Undo` in the catalog and arms the `z` fold chord in the TUI, two keystrokes from a silent content edit.
`crates/outl-shortcuts/tests/tui_catalog_divergence.rs` pins both halves: the catalog rows through `lookup`, the TUI arms by reading `input/normal.rs`.
Which side is right is a product decision nobody has made, so the list is pinned rather than resolved: a new divergence, or a change to a listed one, fails the test.

Another 27 TUI chords are bound in `input/normal.rs` and absent from the catalog.
They are the table in [Keys only the TUI has](#keys-only-the-tui-has), and `crates/outl-shortcuts/tests/tui_only_chords.rs` declares them.
That test fails if one lands in the catalog without this page changing.

### Desktop internals: the range and fold handlers

**This one is for contributors, not for looking up a key.**
It sits under the table above because it is the next question after it: having found the handler, what does the handler have to do?
Six decisions behind individual desktop handlers, each a rule rather than a preference, and each with an obvious alternative that is wrong in a way that is quiet.
The architectural half (the three categories of vim op, why the char-cursor ops have no handler, how Visual mode is reachable without vim) is in [`docs/desktop-keybindings.md`](desktop-keybindings.md) → "Vim parity".

**This section is a declared home for desktop handler behaviour contracts, alongside the per-client CLAUDE.md, and that is a widening, not what the catalog crate already said.**
[`crates/outl-shortcuts/CLAUDE.md`](../crates/outl-shortcuts/CLAUDE.md) → "What this crate does NOT own" sends those contracts to the per-client doc and gives *this* page a different job: "`docs/tui.md` / `docs/shortcuts.md` for user-facing copy".
It states outright that "the per-client doc is the right place to land the contract", so it is the counterpart to this paragraph, not its corroboration.
It illustrates the contracts with `FoldAll` (`z M`), `UnfoldAll` (`z R`) and `DeleteRange`, three of the rules below, which is why the address matters here rather than in the abstract.
The widening stands on its own: the crate CLAUDE.md sits under a 40k context guard, so a contract that is pure iteration order lands here and the rule it serves stays there.
Written down so the next contributor inherits the decision instead of relitigating it from the same evidence.

- **Range ops walk bottom-up or top-down, and tolerate an id that is already gone.**
  `applyVisualBlockOp` now has two callers, `IndentVisualRange` and `OutdentVisualRange`, which walk `[lo → hi]` over the raw slice: a child of a selected parent is meant to be re-levelled.
  The two range moves do not walk the raw slice.
  They walk `rangeMoveTargets` (`@outl/shared/outline`), the **roots** of the selection, so a parent selected with its children moves as one unit.
  Top-down for up, bottom-up for down, because a descending move has to clear the block below the range first.
  A range that is not contiguous siblings of one parent, or already sits at the edge, moves nothing.
  `DeleteRange` is **not** one of them, it owns its own `[...rangeIds].reverse()` loop, bottom-up for a different reason: children before parents, so a parent's move-to-trash cannot pull a still-targeted descendant out from under the walk.
  NodeIds are stable, `deleteBlock` is `Move(node, TRASH)` and moves preserve identity, so the highlight follows the re-render, and `safeCall` swallows a per-id failure instead of aborting the range.
- **`z R` / `z M` walk `flattenAll` / `flattenParents`, never `flattenVisible`.**
  `z R` must expand subtrees hidden under a collapsed parent, and a visible-only walk no-ops on every descendant of a folded node.
  `z M` uses `flattenParents` because `set_block_collapsed` always writes `Op::SetCollapsed` (the CRDT needs every flip to converge), so folding a leaf would make its future children appear collapsed.
  `z R` can use `flattenAll`, since unfolding a leaf has no future effect.
  This mirrors `outl-tui`'s `collect_collapse_candidates`, so the op count matches across clients.
- **`A` routes through a caret intent, not a post-focus fixup.**
  The handler sets `caretIntent: "end"` *before* `editingBlockId`; the row's effect reads it on mount, applies `setSelectionRange`, then clears it.
- **The Visual highlight is a memoised `Set<id>` at the parent**, built once per change as `visualSet` and answered per row in O(1).
  A per-row predicate is O(N²) per keystroke; `isInVisualRange` survives in `@outl/shared/outline` for tests only.
  Mobile's `Journal.tsx` mirrors `visualSet` by that name, so renaming it is a two-client change.
- **`O` creates with `beforeId`.**
  `createBlock({ beforeId: anchor })` → `create_before` (a floor-slot swap in core).
  Never reintroduce the old create-at-tail plus `moveBlockDown` loop.
  `Cmd/Ctrl+Shift+Enter` is caret-aware inside the editor (col 0 → *before*, past col 0 → *below*), and `stopImmediatePropagation` is what preempts the catalog's create-below binding.
- **The block clipboard is one tagged value, and cut preserves identity.**
  `{ kind: "cut", nodeId } | { kind: "copy", markdown }`, with the backend resolving the page via `enclosing_page_id`.
  Cut emits a single `Op::Move` (cross-page allowed, a self-subtree target rejected); copy duplicates through `paste_block_after` with fresh ids.
  `Y` and Visual `y` additionally write the OS clipboard via `copy_markdown`; paste-in `p` / `P` is deferred.

---

## Help overlay vs. this page

In the TUI, `?` in Normal mode opens a tabbed help popup.
On the desktop, `?` with nothing focused opens the help overlay.
If you need to look something up while you type, the overlay is faster than this page.

The TUI popup is written by hand in `view/overlays.rs` (`help_tab_body`), not generated from the catalog, so it can lag this page.
When they disagree, the code in `input/normal.rs` is right.

