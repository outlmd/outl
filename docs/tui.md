# TUI Manual

The `outl` terminal UI is the primary way to interact with an outl workspace.
It's journal-first, modal (Normal / Insert / Visual), and designed to feel familiar if you've used vim or any keyboard-driven outliner (Roam, Logseq, Obsidian).

## Running

```bash
outl --workspace ~/notes          # opens the TUI on ~/notes
outl --workspace ~/notes --theme dracula
outl tui ~/notes             # explicit subcommand form
cd ~/notes && outl           # no args: opens TUI in cwd
```

The TUI requires a real interactive terminal.
If stdout isn't a TTY (e.g. CI), it exits with a clear error instead of hanging.

## Copy to clipboard

Every yank (`yy` / `Y` in Normal, `y` in Visual) writes clean canonical outl markdown to the OS clipboard.
Two paths are tried in order:

1. **`arboard`** — direct X11 / Wayland / macOS clipboard API.
2. **OSC 52** — a terminal escape sequence (`\x1b]52;c;<base64>\x07`) that reaches the clipboard over SSH, inside tmux, and in Chrome OS Crostini where `arboard` has no display server.

The status line reads `yanked N block(s) → clipboard` on success and `yanked N block(s) (clipboard unavailable)` when both paths fail.

Pasting the yanked markdown in another app reconstructs the same bullet structure.
Pasting back into outl uses `p` (with formatting) or `P` (without formatting) — see [Paste from clipboard](#paste-from-clipboard) below.

## Paste from clipboard

`p` and `P` both read the OS clipboard via `arboard` (`actions/paste.rs`).

`p` — **with formatting.**
Routes the clipboard text through `outl_actions::paste_markdown` when it looks like a bullet outline or spans two or more non-blank lines.
Multi-line plain text is split into one block per non-blank line (blank lines are ignored).
Single-line plain text falls through to a native splice at the cursor.

`P` — **without formatting.**
Calls `outl_actions::paste_plain` directly.
The raw clipboard text is inserted as a single block with no normalisation, outline parsing, or paragraph splitting.
Use `P` when the clipboard contains identifiers with underscores, brackets, or other characters that `paste_markdown` would misread as markdown syntax.

### Drag-and-drop file import (Insert mode)

Dragging a file into the terminal window (Finder, Files app over SSH, a file manager) delivers its path as pasted text — terminals have no separate "file drop" event.
In Insert mode, outl checks whether the pasted payload is exactly the path of one or more existing files before falling back to a normal paste.
When it is, each file is imported into `<root>/assets/<hash>.<ext>` (content-addressed, size-capped by `[assets] max_bytes`) and its markdown link is spliced at the cursor, same as typing it.
A pasted path that isn't an existing file (or spans more than one line without every line being a file) is left alone and pastes as plain text.
See [`markdown-format.md` → Asset links](markdown-format.md#asset-links-nameassetshashext) for the link format.

## Mouse capture (opt-in)

By default the TUI does not capture mouse events, preserving the terminal's native text-selection (Shift-drag to copy a URL, etc.).
Set `[tui] mouse_capture = true` in `~/.config/outl/config.toml` to enable mouse support:

| Gesture | Action |
|---|---|
| Scroll wheel | Move the outline selection up / down |
| Click | Select the block under the pointer |
| Drag + release | Select a range and copy it as clean outl markdown to the OS clipboard |

The drag-copy uses the same arboard + OSC 52 dual path as the keyboard yank.

## Config

The TUI reads two layers of TOML before launching:

1. **Global** — `~/.config/outl/config.toml` (the [`outl-config`](../crates/outl-config/) crate; XDG-style on every OS).
   Same file the desktop app's Settings modal writes to, so changing your theme in the desktop reflects on the next TUI launch.
   ```toml
   [theme]
   preset = "dracula"

   [editor]
   vim_mode = true
   font_size = 15
   ```
2. **Per-workspace** — `<workspace>/.outl/config.toml`.
   Carries the legacy `[workspace] actor_id` plus the `actor_claimed_by` marker.
   The actor this device actually writes under lives outside the workspace, in the device store — see [config.md](config.md#where-the-actor-id-actually-lives).
   A `[theme] preset` here overrides the global setting for this workspace only.

Theme precedence at startup (first hit wins): `--theme` CLI flag → per-workspace `[theme] preset` → global `[theme] preset` → built-in default (`outl`).

## Modes

### Normal

The default.
Move between blocks, open references, run commands.
No characters insert themselves — every key is a command.

| Key | Action |
|-----|--------|
| `i` | Edit current block (Insert mode) |
| `I` | Edit, cursor at start of block |
| `a` | Edit, cursor one char to the right (vim "append"). Clamps at end of block so `a` at end-of-line behaves like `i`. |
| `A` | Edit, cursor at end of block (vim "append at end") |
| `x` | Delete char under cursor (Normal mode) |
| `X` | Delete char before cursor (Normal-mode Backspace) |
| `D` | Delete from cursor to end of block (vim `d$`) |
| `C` | Delete to end of block + enter Insert (vim `c$`) |
| `S` | Clear block + enter Insert at column 0 (vim "substitute line") |
| `s` | Delete char under cursor + enter Insert (vim "substitute char") |
| `r{ch}` | Replace char under cursor with next typed char, stays in Normal (vim `r`) |
| `f{ch}` / `F{ch}` | Find next / previous occurrence of the next typed char on the current block |
| `~` | Toggle case of char under cursor; cursor advances one position |
| `Y` | Yank current block (alias of `y y`) — also writes clean outl markdown to the OS clipboard (arboard + OSC 52 fallback for SSH / tmux). Status line: `yanked N block(s) → clipboard` or `(clipboard unavailable)`. |
| `e` | Cursor to the end of the current / next word (vim `e`; pairs with `w`) |
| `*` / `#` | Search workspace for the word under cursor (forward / backward). Walk results with `n` / `N`. |
| `z R` / `z M` | Unfold all / fold all blocks on the current page (chord). `z M` skips leaf blocks — only blocks that already have children get folded; folding a leaf today is invisible, but would silently surface as "children appear collapsed" once the user added any underneath. |
| `z z` | Center viewport vertically on the cursor (chord) |
| `z i` / `z o` | Zoom in on the selected block / zoom out one level (chord). `z i` makes the block the outline root — only its subtree renders, with a breadcrumb of ancestors in the status chrome; `z o` pops one level back toward the full page (no-op at the page root). Pure local view state — never written to the op log, so it's per-device and never syncs. |
| `g v` | Re-enter Visual mode at the last captured range (chord) |
| `o` / `O` | New block below / above |
| `Enter` | Open `[[ref]]` / `#tag` / journal / block ref (`((blk-X))` / `!((blk-X))`) under cursor (otherwise edit). On a block ref it opens the source page and lands the cursor on the referenced block; orphan handles surface a status message and stay put. |
| `j` / `k` / `↑` / `↓` | Move between blocks |
| `h` / `l` / `←` / `→` | Move cursor inside the current block |
| `w` / `b` | Cursor to next / previous word |
| `0` / `$` | Cursor to start / end of block |
| `Tab` / `Shift-Tab` | Indent / outdent the current block |
| `K` / `J` (or `Alt+↑/↓`) | Move block up / down |
| `dd` | Delete the current block (chord) |
| `c` | Fold / unfold the current block. The bullet row shows `▼ ` (expanded) or `▶ ` (collapsed) when the block has children, two spaces otherwise. Children are hidden from the outline while collapsed and `j` / `k` skip past them, but the underlying tree is untouched. State is persisted as an `Op::SetCollapsed` in the op log — every device replays the same sequence, so the fold layout converges across iCloud / Syncthing peers without relying on file-level last-write-wins (which would lose concurrent flips). No-op on a block whose sidecar entry hasn't been written yet (save first). |
| `y r` | Yank the current block's ref handle (`((blk-XXXXXX))`) to the OS clipboard + `last_yanked_ref` (chord). On headless / no-clipboard environments it falls back to the status line only. |
| `Ctrl+Enter` / `Ctrl+T` | Cycle the block's task prefix — TODO → DOING → DONE → none (`Ctrl+T` is the portable fallback for tmux / Terminal.app, which collapse `Ctrl+Enter` into plain `Enter`) |
| `u` / `Ctrl+R` | Undo / redo |
| `V` | Enter Visual mode (multi-block select) |
| `t` / `Home` | Today's journal |
| `[` / `]` | Previous / next journal |
| `g j` | Jump to today (chord) |
| `Ctrl+P` | Quick switcher (fuzzy page/journal pick) |
| `/` | Slash command menu (Notion-style, fuzzy filter) |
| `:` | Command palette (vim-style) |
| `B` | Toggle the inline backlinks section below the outline |
| `Ctrl+O` | Flip the backlinks sort direction (newest ⇄ oldest); persists to `config.toml` |
| `?` | Toggle this help popup |
| `q q` / `Ctrl+C` | Quit — `q` alone arms a chord, second `q` confirms |
| `Z Z` | Quit (vim's "save and quit"). outl already commits Insert on every Normal boundary, so this is equivalent to `q q`; kept distinct so vim muscle memory works. |

### Insert

Text input goes into the buffer.
Esc commits (writes back to the `.md`), Enter commits + creates a new block.

| Key | Action |
|-----|--------|
| `Esc` | Commit and return to Normal |
| `Enter` | Commit + new block below (soft newline inside open code fence — see below) |
| `Alt+Enter` / `Ctrl+J` | Soft newline (stays in same block) — portable across terminals |
| `Shift+Enter` | Soft newline — only on terminals that speak the kitty keyboard protocol |
| `Ctrl+Enter` / `Ctrl+T` | Cycle the block's task prefix — TODO → DOING → DONE → none (stays in Insert, caret follows the text; `Ctrl+T` works on terminals that collapse `Ctrl+Enter`) |
| `Tab` / `Shift-Tab` | Indent / outdent (stays in Insert) |
| `Backspace` on empty | Delete block, move to previous |
| `Left` at column 0 | Spill into the previous block (cursor at end) |
| `Right` at end of block | Spill into the next block (cursor at start) |
| `(`, `[`, `{` | Auto-pair with closing |
| `[[` | Page reference autocomplete (titles indexed across workspace) |
| `#` | Tag autocomplete |
| `@` | Mention autocomplete — word-initial only (preceded by start-of-line or whitespace, so `a@b.com` doesn't fire). Lists pages where `type:: person` is set, fuzzy-matched against the typed name. Composite names allow spaces (`@Thiago Avelino` is one query, not a tag terminating at the space). When no existing person matches, the typed text is offered as a "create new" candidate; accepting it materialises the page with `type:: person` set. Insert produces `[[@name]]`. |
| `((` | Block reference autocomplete — fuzzy-match on block text, inserts `((blk-XXXXXX))`. Empty query lists newest-first (NodeId descending = ULID time order) so the popup is deterministic and the same eight rows show on every keystroke. |
| `:` | Emoji shortcode autocomplete — word-initial only (preceded by start-of-line or whitespace, so `14:00`, `key::value`, and `https://` don't fire). Backed by `outl_md::emoji::search` (GitHub gemoji catalog, ~1800 shortcodes). Popup row shows `glyph  :shortcode:`; accept inserts the canonical `:shortcode:` form into the buffer (the `.md` always stores the shortcode literal, never the codepoint — see `docs/markdown-format.md` § "Emoji shortcodes"). The pretty render translates `:shortcode:` to the unicode glyph; the editing row stays raw so cursor columns match source bytes 1:1. |
| `↑` / `↓` in popup | Navigate completion |
| `Enter` / `Tab` in popup | Accept completion |
| `Esc` in popup | Cancel completion |

### Multi-line blocks and fenced code

A single block can hold multiple lines (`Alt+Enter` / `Ctrl+J` / `Shift+Enter` on kitty terminals).
Used for paragraphs of prose inside one bullet and — most importantly — for fenced code blocks:

```
- ```lisp
  (+ 1 2)
  ```
```

**Auto-fence**: while typing inside an *open* code fence (the opener
` ``` ` is above the cursor but no closer has been typed yet), plain
`Enter` is treated as a soft newline.
This lets you type a fenced block naturally without remembering the soft-newline combo:

```
- ```lisp        ← typed `- `` ```lisp `, then Enter (+ 1 2)        ← typed body, Enter
  ```            ← typed closer, Enter
- next bullet    ← Enter here is a sibling again
```

The on-disk format is plain CommonMark — see [`docs/markdown-format.md`](markdown-format.md#multi-line-block-text-continuation-lines).

### Visual

A range of blocks is highlighted.
`j` / `k` extends the range; the common Normal-mode keys for editing aren't available — Visual is for batch operations.

| Key | Action |
|-----|--------|
| `Esc` / `v` / `V` | Cancel, back to Normal |
| `j` / `k` / `↑` / `↓` | Extend the range |
| `d` / `x` | Delete the selected range |
| `y` | Yank the selected range to the register and to the OS clipboard as clean outl markdown (arboard + OSC 52 fallback) |
| `Tab` / `>` | Batch indent the selected range |
| `Shift-Tab` / `<` | Batch outdent the selected range |

## Overlays

Three modal popups can appear over the main panes.
They steal the keystream while open; `Esc` always closes them.

### Quick Switcher (`Ctrl+P`)

Fuzzy search across page titles, slugs, and journal dates.
Today's date is always present even if the journal file doesn't exist yet.

### Slash menu (`/`) and Command palette (`:`)

Two surfaces over the **same** command registry — pick whichever matches your muscle memory:

- **`/`** (Normal mode) opens a Notion-style filterable list.
  Each entry shows its name + description.
  Inside Insert mode, typing `/` triggers inline autocomplete with the same list — pick a command with `Tab`/`Enter` without leaving the buffer.
- **`:`** is the vim command line.
  Same registry, same args, same aliases — `/q` and `:q` are interchangeable.

Unknown commands surface in the status line as `unknown command: <name>`.

#### Workspace / navigation

| Command | Aliases | Action |
|---------|---------|--------|
| `open <name>` | `o`, `new`, `n` | Open (or create) page by name |
| `today` | — | Jump to today's journal |
| `search` | `s`, `find` | Workspace-wide block search |
| `quit` | `q`, `exit` | Close the TUI |
| `write` | `w`, `save` | Force-save current page |
| `refresh` | `r`, `reload` | Re-read workspace from disk |
| `theme <preset>` | — | Swap the active theme |
| `help` | `h` | Toggle help popup |

#### Properties

| Command | Aliases | Action |
|---------|---------|--------|
| `prop-block <key> <value>` | `prop` | Set property on current block (empty value deletes) |
| `prop-page <key> <value>` | — | Set page-level property (`title::`, `icon::`, …) |

#### Block references

| Command | Aliases | Action |
|---------|---------|--------|
| `refer` | — | Copy `((blk-XXXXXX))` of the current block to the OS clipboard + `last_yanked_ref`. Same as the `y r` chord. |
| `refer-embed` | — | Copy the embed form `!((blk-XXXXXX))` of the current block to the OS clipboard + `last_yanked_ref`. |

> **Clipboard fallback**: `y r` / `/refer` / `/refer-embed` use [`arboard`](https://crates.io/crates/arboard) to talk to the OS clipboard.
> The status line reads `copied … to clipboard` on success and `yanked … (clipboard unavailable)` on terminals / SSH sessions without a clipboard backend — the token still lives in `last_yanked_ref` so the in-app paste path keeps working.

#### Files

| Command | Aliases | Action |
|---------|---------|--------|
| `upload` | `attach` | Attach a file: pick it, copy it into `<workspace>/assets/<hash>.<ext>`, and insert its `[name](assets/…)` link at the cursor |

> **File picker**: on macOS / Windows `/upload` opens the native "open file" dialog.
> On Linux (and any build without the dialog) it takes a typed path — `/upload` hands off to the `:` palette pre-filled `upload `, so `upload /path/to/file.pdf` works everywhere, including over SSH.
> Dragging a file into the terminal also works: in Insert mode a pasted path to an existing file is imported and linked automatically (see [Drag-and-drop file import](#drag-and-drop-file-import-insert-mode)).
> outl never renders the file — clicking the link (`g x`) opens it in the OS default app.

#### Code execution

| Command | Aliases | Action |
|---------|---------|--------|
| `run` | `x`, `execute` | Run the code block under the cursor |

#### Date & time inserters

These write text **at the cursor** (Insert mode only).
They skip the auto-commit step the other commands do, so your in-flight edit stays alive while the text lands.

| Command | Aliases | Inserts |
|---------|---------|---------|
| `date-today` | `dt` | `[[YYYY-MM-DD]]` (today) |
| `date-tomorrow` | `dtm` | `[[YYYY-MM-DD]]` (today + 1) |
| `date-yesterday` | `dy` | `[[YYYY-MM-DD]]` (today − 1) |
| `date-next-week` | `dnw` | `[[YYYY-MM-DD]]` (today + 7) |
| `date-last-week` | `dlw` | `[[YYYY-MM-DD]]` (today − 7) |
| `date-next-monday` | `dnmon` | next Monday's journal ref |
| `date-next-tuesday` | `dntue` | next Tuesday's journal ref |
| `date-next-wednesday` | `dnwed` | next Wednesday's journal ref |
| `date-next-thursday` | `dnthu` | next Thursday's journal ref |
| `date-next-friday` | `dnfri` | next Friday's journal ref |
| `date-next-saturday` | `dnsat` | next Saturday's journal ref |
| `date-next-sunday` | `dnsun` | next Sunday's journal ref |
| `date <arg>` | — | flexible — see below |
| `iso-date-today` | `isod` | `YYYY-MM-DD` (no brackets, for `due::` etc) |
| `iso-date-tomorrow` | `isodtm` | `YYYY-MM-DD` |
| `iso-date-yesterday` | `isody` | `YYYY-MM-DD` |
| `time-now` | `now`, `tn` | `HH:MM` (no brackets, plain time) |
| `datetime-now` | `dtn`, `stamp` | `[[YYYY-MM-DD]] HH:MM` (journal ref + time) |
| `week-num` | `wn`, `week` | `#YYYY-Www` (ISO week as a tag) |

##### `/date <arg>`

| Input | Resolves to |
|-------|-------------|
| `/date +3d` | today + 3 days |
| `/date -2w` | today − 2 weeks |
| `/date +1m` | today + 1 month (Jan 31 + 1m → Feb 28/29 — clamped to last day of month) |
| `/date 5d` | bare `Nd`/`Nw`/`Nm` is treated as positive |
| `/date 2026-06-15` | absolute ISO date |
| `/date April 22nd, 2026` | any absolute spelling the shared date parser accepts (`2026/04/22`, `22/04/2026`, `Sept 3rd, 2025`, `22 April 2026`, …) |

Garbage input (`/date nope`, `/date +3x`, invalid date) shows `usage: date +Nd | -Nw | +Nm | YYYY-MM-DD` on the status line.

> **Weekday math:** `date-next-<weekday>` always jumps to the **next** occurrence of that weekday, strictly in the future.
> Running it on the same weekday adds 7 days, not 0 — `date-next-monday` on a Monday means "next Monday.
> **ISO week year:** `week-num` uses `%G-W%V` (ISO 8601), not `%Y-W%V`.
> The ISO year can differ from the calendar year on a few days around year boundaries — e.g. 2025-12-31 (Wednesday) belongs to ISO week `2026-W01`, not `2025-W01`.

#### Plugin commands

JavaScript plugins installed under `<workspace>/.outl/plugins/` are loaded at startup.
Each command a plugin contributes (its `contributes.commands`) appears in the **same** `/` slash menu, listed by its title with a `plugin · <plugin-id>` description so you can tell it apart from a built-in.
Selecting one runs the plugin's handler; any `notify` message or error it produces shows as a toast.
If the plugin mutates the workspace, the affected `.md` files are re-rendered and the current page reloads so the change is visible immediately.

Plugins are **best-effort**: a plugin that fails to load (bad manifest, tampered bundle, mismatched API version) is skipped with a toast, and the TUI runs normally.
A workspace with no plugins behaves exactly as before.

Plugins can also register `onOp` hooks that fire after every workspace mutation (your edits, or ops arriving from a peer).
Hooks are dispatched once per mutation; a hook that itself mutates the workspace never re-triggers itself (loop-safe).

> Keybinding contribution (`contributes.keybindings`) and the install / permission-approval flow are not implemented yet; today the TUI surfaces plugin **slash commands** and runs **op hooks**.

## Panels

```
┌─outl · default-dark ───────────────────────────────────────────────────┐
│ Page · Avelino                                                         │
├────────────────────────────────────────────────────────────────────────┤
│ - I am the author                                                      │
│ - some other note                                                      │
│ ───────────────────────────────────────────────────────────────────────│
│  Backlinks · 2 ref(s)                                                  │
│                                                                        │
│ 📄  Project X                                                          │
│ - led by [[Avelino]]                                                   │
│   - milestone A                                                        │
│   - milestone B                                                        │
│                                                                        │
│ 📅  2026-05-24                                                         │
│ - meeting with [[Avelino]] about Q4                                    │
├──┌NORMAL─┐ i edit  o new  K/J move …  ⇇ 2 backlinks ───────────────────┤
│  └───────┘                                                             │
└────────────────────────────────────────────────────────────────────────┘
```

- **Outline** — the current view (journal or named page).
  Markdown renders inline (bold/italic/code/strike); the selected/editing block is shown raw so cursor columns align with source bytes.
  Block references (`((blk-XXXXXX))`) resolve to the source block's text plus its page icon; orphaned handles render dimmed.
  Embeds (`!((blk-XXXXXX))`) — when the block contains a single embed token (whitespace OK) — render the source block **and its children** expanded read-only below the carrying block.
  Every embed row carries a `↳ ` prefix (root + descendants), so the expansion reads as one cohesive block.
  Descendants are indented by `2 * (depth + 1)` spaces before their `↳ ` so children align under the source's *text*, not under the parent's `↳ `.
  Task checkboxes (`☐` TODO, `◐` DOING, `☑` DONE), page refs, and tags render with their normal styling inside the expansion.
  Recursion is capped at depth 4 to break embed cycles.
  The cursor-bearing block always keeps the raw `((…))` / `!((…))` literal on its first row so column counting stays exact.
- **Backlinks (inline)** — rendered below the outline, separated by a full-width `─` rule.
  Every block in any other page that contains `[[this]]` or `#this` shows up with its children, grouped by source page.
  Pages are ordered by how recently they referenced this page (default: most recent on top); `Ctrl+O` flips the direction, shown in the section header (`↓ newest (^O)` / `↑ oldest (^O)`).
  Within a page, blocks keep document order.
  When a citing block is nested inside an outline (not at page root level), its ancestor chain shows as a dimmed breadcrumb above it, root-first (e.g. `Planejamento Q3 › Objetivos`); a root-level block has no breadcrumb.
  Consecutive references from the same branch collapse the trail — it renders once, then stays implicit for the following reference(s) until the branch changes.
  `j`/`k` navigation crosses the separator transparently: from the last outline block, `j` lands you on the first backlink; `k` from the first backlink walks back into the outline.
  Toggle the section with `B`.
  Self-references are excluded.
  Press `i` / `Enter` on a backlink to jump to its source page positioned on the referencing block (in-place editing lands in a follow-up).
- **Status / hint** — mode badge, contextual key reminder, backlink count, status messages.

### Pages sidebar

Toggled by `Ctrl+E` (the desktop-standard "toggle sidebar" chord; `\` was dropped to avoid clashing with it).
Shows Today / pinned / recent pages and a mini-calendar of journals.
`j` / `k` move the selection.
`Tab` cycles the section (Today / Pinned / Recent / Calendar).
`Enter` opens the focused page.
`d` on a regular page arms a `delete page '<title>'? y/n` confirmation in the status line.
`y` confirms, any other key cancels (and is swallowed).
The `g d` chord (Normal mode) routes through the same confirmation flow: with the sidebar focused it deletes the highlighted row, with the outline focused it deletes the current page.
Journals are refused in both paths (calendar rows are a no-op; pinned/recent journals are silently skipped).
On confirm, the page root moves to `NodeId::trash()` via `outl_actions::page::delete`, the `.md` + `.outl` projections are removed, the index is rebuilt, and if the deleted page was the current view the TUI lands on today's journal.
For fuzzy title jumps without the pane, `Ctrl+P` (quick switcher) still works with the sidebar closed.

## Parser-warning banner

When you open a `.md` that the outl parser had to recover from (a leading `# heading`, a free paragraph between bullets, imported markdown that doesn't fit the dialect), the TUI shows a yellow banner above the outline:

```
┌─ ⚠ 3 line(s) outside outl dialect — preserved as blocks ─┐
│ line 1: # 2026-06-08 (+2 more)                           │
└──────────────────────────────────────────────────────────┘
```

- Every offending line is preserved as a regular block — nothing is dropped on parse, and the next save normalises the file to the dialect (`- <raw>`).
- The status line also carries a short `⚠ N line(s) outside outl dialect — preserved` hint for terminals where the banner is not visible.
- On a clean file the banner collapses to zero height; the layout looks identical to before the feature landed.
- Source of truth: `ParsedPage.warnings` (`outl_md::ParseWarning`).
  Mobile + desktop render the same data via `<ParseWarningsBanner>` from `@outl/shared`, and `outl doctor` lists every page with active warnings.
- The behaviour is intentionally non-blocking: you can keep editing, save, navigate away.
  Use the warning as a hint to clean the file at your pace — outl never deletes content on your behalf.

## Behavior worth knowing

- **Autosave, coalesced**: every commit (Esc from Insert, structural ops, history navigation) marks the page dirty and repaints on the same frame — no disk I/O yet.
  The actual `.md` write + op-log reconcile drains the moment you pause typing, or is forced after 600ms if you keep a burst going.
  It always flushes before quitting, `Ctrl+S`, cross-page navigation, or a `call:` re-run, so an edit is never lost — it just isn't on disk the instant you press `Esc`.
  Concurrent `outl serve` is safe — both routes go through `outl_md::reconcile_md`.
- **An unreadable page is never overwritten.**
  Causes: a permissions change, a raw I/O error, invalid UTF-8, or an iCloud file whose contents haven't downloaded to this device yet.
  The TUI shows `cannot read <path> … editing disabled` and **refuses to save that page** until a load succeeds.
  A failed read parses as an empty document, and without this guard the next commit would render that emptiness back over your page and reconcile it, sending every block to the trash on every device.
  A page that simply doesn't exist yet is a different case and still opens as a normal empty page.
- **No IDs on disk**: every block has a stable ULID, but it lives in the `.outl` sidecar file, not in your markdown.
  `outl serve` / `outl-tui` rebuild that sidecar after every change.
- **External edits hot-reload**: when another editor writes the currently-open `.md`, the TUI picks it up automatically within about a second.
  If you're in Insert mode, it refuses to clobber your in-flight edit and writes a warning on the status line — finish typing, press `Esc` to commit, then `Ctrl+L` to reload.
- **Undo bounded**: 200 most recent snapshots.
  Older edits drop off the front.
  Each snapshot remembers selection + cursor so undo lands you where you were.
- **Empty pages keep a bullet**: deleting the last block silently re-adds an empty `- ` so your cursor always has somewhere to go.
- **Slugified filenames**: `[[Avelino]]` lives in `pages/avelino.md` with `title:: Avelino` set automatically on first open.

## Code-block execution

Fenced code blocks can be run in place.
The result lands as a `> **result:**` subblock right below the source, and re-running updates the same subblock idempotently.

| Key | Action |
|-----|--------|
| `g x` | Run the code block under the cursor — or, when the block isn't code, open the markdown link `[text](url)` under the cursor (issue #183) |
| `:run` (also `:x`) | Same, via the command palette |

`g x` prioritizes code: a fenced block always runs.
Only when the current block is **not** a code block does `g x` look for a markdown link under the cursor and open it — the cursor may sit on the link's text or its URL.
An asset link (`[name](assets/<hash>.<ext>)`) opens the file in the OS default app; outl never renders it.
Any other link opens in the system browser, and only `http` / `https` / `mailto` are allowed — anything else is refused.
Following a `[[page]]` / `#tag` / `((block ref))` is still `Enter`, unchanged.

```
- ```lisp
  (map (lambda (x) (* x x)) (list 1 2 3 4))
  ```
  - > **result:** `(1 4 9 16)`
```

Built-in languages (each behind a Cargo feature, so you can strip
what you don't need):

| Tag | Engine | Notes |
|-----|--------|-------|
| ` ```lisp ` | [Steel](https://github.com/mattwparas/steel) | Scheme R5RS-ish |
| ` ```js ` | [Boa](https://boajs.dev) | ES2015+, `console.log` captured |
| ` ```python ` | [RustPython](https://rustpython.github.io) | Py3 subset, no native ext |
| ` ```lua ` | [mlua](https://github.com/mlua-rs/mlua) | Lua 5.4 vendored |
| ` ```echo ` | builtin | Returns source verbatim — debug only |

Adding another language is one file under `crates/outl-exec/src/runtimes/` plus a feature flag.
See [`docs/exec.md`](exec.md) (forthcoming) for the contract and `outl-exec/src/runtimes/lisp.rs` as the canonical template.

## Theming

See [`docs/theming.md`](theming.md) for the palette spec, preset list,
and how to set a theme via config or CLI.

## What's NOT in the TUI yet

The core editor and most-used surfaces ship today.
Some things are explicitly deferred:

- **`{{query: ...}}`** — inline saved queries; not yet implemented.
- **Visual mode batch indent / yank / paste** — only delete is wired
  today.
- **Graph view** — the desktop has it; the TUI may grow one but
  not a priority.
- **P2P sync via iroh** — works today (default transport, single-user
  multi-device over QUIC); the `file` transport (iCloud Drive) is opt-in.
- **Live multi-user collaboration** — multiple people editing the same
  workspace concurrently is still out of scope.
