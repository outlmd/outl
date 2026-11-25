# TUI manual

The `outl` terminal UI is a keyboard-driven outliner that lives in your terminal.
It opens on today's journal, edits markdown you can read with `cat`, and syncs with your other devices while it runs.
If you have used vim, Roam or Logseq, the keys will feel familiar.
If you have not, start with [Your first 10 minutes in the TUI](tui-quickstart.md) and come back here when you want a specific answer.

| You want | Go to |
|---|---|
| A guided first session | [TUI quickstart](tui-quickstart.md) |
| Something is stuck, blank or not saving | [TUI troubleshooting](tui-troubleshooting.md) |
| Every chord, side by side with the desktop and phone | [Shortcuts](shortcuts.md) |
| What each client can and cannot do | [Client parity](client-parity.md) |
| How copy and paste convert content | [Paste](paste.md) |

This page is the complete TUI reference.
The cross-client chord catalog is [shortcuts.md](shortcuts.md), but it does not list every TUI chord yet, so the tables below are the full TUI list.

## Start it

```bash
outl                              # opens the TUI on the workspace outl resolves (see below)
outl --workspace ~/notes          # explicit workspace
outl -w ~/notes --theme dracula   # explicit workspace and theme for this run
outl tui ~/notes                  # subcommand form, path as an argument
outl-tui ~/notes                  # the standalone binary: path only, no flags
```

**Which workspace opens.**
First hit wins: the path argument of `outl tui`, then `--workspace`, then `[workspace] last` from `~/.config/outl/config.toml`, then the current directory.
The full chain is in [config.md → Workspace path](config.md#workspace-path).
`cd ~/notes && outl` opens the current directory only when `last` is unset or no longer exists on disk.

**A folder that is not a workspace yet.**
In an interactive terminal outl asks `Initialize a new workspace here? [y/N]`.
`y` runs `outl init` for you.
Anything else aborts and changes nothing.
With no terminal on stdin or stderr (a script, a pipe) it exits with `no outl workspace at <path> — run outl init <path> first` instead of asking.

**A real terminal is required.**
If stdout is not a TTY (CI, a pipe) the TUI exits with `outl-tui requires an interactive terminal (stdout is not a TTY)`.

**What you see while it opens.**
A one-line frame with `Opening <path>` appears first, before the workspace is read.
On a large workspace that frame can stay up for seconds while the op log replays.
It is the app working, not a hang.

**Several outl processes on one workspace is a supported setup.**
A TUI, the desktop app, `outl serve` and an MCP server can all be open at once.
Only one process writes under the device's own actor id at a time.
The others write under a temporary actor, so nothing races on the same op file.

**Where problems are logged.**
Dependency warnings never print on the screen.
They go to `<workspace>/.outl/tui.log`.
Run `RUST_LOG=debug outl` for more detail when you report a bug, then `tail -f` that file.

### Terminal notes

- **`Shift+Enter` and `Ctrl+Enter`** need a terminal that speaks the kitty keyboard protocol (kitty, WezTerm, Alacritty, Ghostty, foot, recent iTerm2).
  The TUI asks the terminal once, right after the first frame.
  A terminal that never answers can leave the keyboard dead for about two seconds at launch.
  Wait it out, the keys come back.
- **Portable fallbacks that always work:** `Alt+Enter` or `Ctrl+J` for a soft newline, `Ctrl+T` for the task cycle.
  `Alt+Enter` needs "Option as Meta" on macOS terminals.
- **Icons.**
  Chrome uses ordinary emoji and symbols, so it works without a special font.
  Set `[tui] icons = "nerd-font"` for compact Nerd Font glyphs on status chips, fold markers, property and command glyphs.
  Task checkboxes (`☐` `◐` `☑`), calendar day dots and scrollbars stay Unicode in both styles.
- **Theme.**
  `--theme <preset>` beats the per-workspace and global config.
  `:theme <preset>` swaps the theme for the current session only.
  Presets and your own `~/.config/outl/themes/*.toml` are in [theming.md](theming.md), and `outl theme list` names the files it ignored and why.

## The screen

```
┌─outl · default-dark ───────────────────────────────────────────────────┐
│ outl ▸ notes ▸ Journal · Wednesday, 2026-11-25         ☑ 1/3  4s ago  │
├────────────────────────────────────────────────────────────────────────┤
│ - ☐ call the dentist                                                   │
│ - ▼ project x                                                          │
│   - led by [[Avelino]]                                                 │
│ ───────────────────────────────────────────────────────────────────────│
│  Backlinks · 2 ref(s)                                  ↓ newest (^O)   │
│                                                                        │
│ Project X                                                              │
│ - meeting with [[Avelino]] about Q4                                    │
├────────────────────────────────────────────────────────────────────────┤
│ NORMAL  notes  2 backlinks  i edit  o new  c fold …    14:02  saved ? help │
└────────────────────────────────────────────────────────────────────────┘
```

**Header.**
Workspace name, the page (or `Journal · <weekday>, <date>`), and the zoom trail when you have zoomed into a block.
On the right, chips appear only when they apply: a spinner while the index builds (`indexing`), `done/total` for the tasks on the page, `editing` while you are in Insert, and how long ago the page was saved.

**Outline.**
Markdown renders inline: bold, italic, code, strikethrough, `[[page]]` links, `#tags`, task checkboxes, `> ` quotes and tables.
The block under the cursor (or being edited) is shown as raw source so the caret columns match the bytes in the file.
Everything else is drawn pretty.

**Footer.**
Left to right: mode badge (`NORMAL`, `INSERT`, `VISUAL`), workspace name, backlink count, then the status message or a key hint for the current mode.
Right side: clock, a `saved` indicator and `? help`.
A status message is elided with `…` when the terminal is narrow, and the hint comes back when the message clears.

**Toasts.**
Short confirmations (`saved`, `refreshed`, a due reminder, a failed save) stack bottom right and expire on their own.
A toast grows to four rows to fit a long error, so a path is not cut off at the border.

**Help.**
`?` opens a tabbed help popup: Normal, Insert, Visual, Sidebar, Overlays, Dates.
`h` / `l` or `Tab` switch tabs, `j` / `k` scroll, `PgUp` / `PgDn` page, `g` / `G` jump to top and end, `?` `Esc` or `q` close.
Every other key is swallowed while the popup is open, so the outline behind it never moves.
The help rows are hand-written, so a rare chord can be missing from them.
This page is the complete list.

### Sections under the outline

Both sections sit below the outline, each behind a full-width `─` rule.

**Backlinks ("Linked from").**
Every block that contains `[[this page]]` or `#this-page`, grouped by the page it lives on.
It is on by default in the TUI.
`Ctrl+B` hides and shows it.
Each citing block is one row followed by its children (up to four levels, always expanded).
Pages are ordered by how recently they cited this one, newest first.
`Ctrl+O` flips the direction and saves the choice to `[display] backlinks_order` in `config.toml`.
A citing block nested inside an outline shows its ancestors as a dim breadcrumb above it (`Planejamento Q3 › Objetivos`).
Consecutive references from one branch share a single breadcrumb.
A page that cites itself lists that row too.
On a cold start the header reads `Backlinks · indexing…` so a page that is still loading is not mistaken for a page with no backlinks.

`j` / `k` walks across the rule: from the last outline block `j` lands on the first backlink, and `k` from the first backlink goes back up.
Inside the panel `j` / `k` step through each citing block, then its children, then the next citing block.

**Editing a backlink where it is cited.**
Press `i` or `Enter` on a backlink row (or one of its children) to edit that block in place, without leaving the page you are reading.
The edit lands as one op on that block, and the source page's `.md` is rewritten from the op log.
`Ctrl+T`, `g D`, `d d`, `Tab`, `Shift+Tab` and the move chords act on that same block.
The per-chord meaning is in [shortcuts.md → On a backlink row](shortcuts.md#on-a-backlink-row).
A row whose block was deleted on another device refuses before the edit starts: `That block was deleted, so nothing changed.`
`Enter` while editing a backlink commits and leaves Insert, instead of splitting the block, because there is nowhere on this page for a new sibling's caret to go.
`o` and `O` on a child row create a sibling and open Insert on it.
On the citing block itself they create the sibling on the source page, say `not editable from the panel`, and keep you where you were.
A page that cites itself is the exception, since the new block is in the outline you are already reading.

What the TUI cannot do here is open the page a backlink lives on from the row.
Use `Ctrl+P` and the page name.
See [client-backlinks.md](client-backlinks.md) for the rule across clients.

**Nested pages.**
A page named `os` lists `os/linux` and `os/linux/debian` under a `Nested pages · N` header.
It is independent of the backlinks toggle.
The list is read-only: `j` / `k` stop at the backlinks section.
To open one, press `Ctrl+P` and type its name.

## Modes

outl is modal like vim.
The footer badge says which one you are in.

| Mode | What keys do | Enter it | Leave it |
|---|---|---|---|
| Normal | Every key is a command, nothing types itself | start-up, or `Esc` from the others | (you are in it) |
| Insert | Keys type into one block | `i` `I` `a` `A` `o` `O` `Enter` `s` `C` `S` | `Esc` |
| Visual | `j` / `k` grows a range of blocks, one key acts on all of them | `V` | `Esc`, `v` or `V` |

An open overlay (quick switcher, slash menu, search and the rest, below) takes every keystroke until `Esc` closes it.
`Ctrl+C` quits from anywhere and commits an edit in progress first.
`Ctrl+S` saves now, `Ctrl+L` re-reads the workspace from disk, in any mode.

### Normal mode

**Start typing.**

| Key | What it does |
|---|---|
| `i` | Edit the current block, caret at the cursor |
| `I` | Edit, caret at the start of the block |
| `a` | Edit, caret one character right of the cursor (at end of line it behaves like `i`) |
| `A` | Edit, caret at the end of the block |
| `o` / `O` | New block below / above, and start typing |
| `Enter` | If the cursor is on a `[[ref]]`, `#tag`, journal date or `((block ref))`, open it. Otherwise edit the block |
| `s` | Delete the character under the cursor and start typing |
| `S` | Clear the block and start typing at column 0 |
| `C` | Delete from the cursor to the end of the block and start typing |

**Move around.**

| Key | What it does |
|---|---|
| `j` / `k` or `↓` / `↑` | Next / previous block |
| `g g` / `G` | First / last block |
| `PgDn` / `PgUp` | One screen down / up |
| `Ctrl+D` / `Ctrl+U` | Half a screen down / up |
| `h` / `l` or `←` / `→` | Cursor left / right inside the block |
| `w` / `b` / `e` | Next word / previous word / end of word |
| `0` / `$` or `Home` / `End` | Start / end of the block |
| `f{ch}` / `F{ch}` | Jump to the next / previous occurrence of the next character you type |
| `z z` | Center the screen on the cursor |
| `t` | Today's journal |
| `[` / `]` | Previous / next day from the journal you are on |
| `g [` / `g ]` | Yesterday / tomorrow relative to today, whatever page you are on |
| `g j` | Today (same as `t`) |

**Change text without leaving Normal.**

| Key | What it does |
|---|---|
| `x` / `X` | Delete the character under / before the cursor |
| `D` | Delete from the cursor to the end of the block |
| `r{ch}` | Replace the character under the cursor with the next one you type |
| `~` | Toggle the case of the character under the cursor and move right |
| `Ctrl+T` or `Ctrl+Enter` | Cycle the task prefix: none, TODO, DOING, DONE, none. `Ctrl+T` is the portable one, because tmux and Terminal.app turn `Ctrl+Enter` into `Enter` |
| `g D` | Mark the block DONE from any state |
| `u` / `Ctrl+R` | Undo / redo (see [Saving, undo and safety](#saving-undo-and-safety)) |

`r`, `f` and `F` wait for one character.
Any other key cancels the wait and clears the prompt from the footer.

**Structure.**

| Key | What it does |
|---|---|
| `Tab` / `Shift+Tab` | Indent / outdent the block |
| `K` / `J` or `Alt+↑` / `Alt+↓` or `Ctrl+Shift+↑` / `Ctrl+Shift+↓` | Move the block up / down among its siblings. The cursor stays on it, and `u` undoes the whole move. A terminal that does not send `Ctrl+Shift+↑` still has `Alt+↑` |
| `d d` | Delete the block and its children (it goes to the trash, see [TUI overlays → Trash](tui-overlays.md#trash-trash-trash)) |
| `c` | Fold / unfold the block. `▼` means expanded, `▶` collapsed, blank means no children |
| `z M` / `z R` | Fold every block that has children / unfold everything on this page |
| `z i` / `z o` | Zoom into the block / zoom out one level |
| `z f` | Focus mode: hide header, sidebar, status bar and the warnings banner so only the outline is left. Press again to bring them back |
| `V` | Start Visual mode |
| `g v` | Re-select the last Visual range |

Folds are stored in the op log, so a fold you make here shows up on your other devices too.
`c` does nothing on a block that has not been saved yet.
Wait for the footer `saved` and press it again.

**Zoom.**
`z i` makes the selected block the root of the view.
Only its subtree renders, and the header shows the trail of ancestors.
The page's backlinks and nested pages go off screen while you are zoomed, and `j` stops at the edge of the subtree.
`z o` goes back up one level and does nothing at the page root.
Zoom is a local view setting.
It is never written to the op log and it is not synced.

**Copy and paste.**

| Key | What it does |
|---|---|
| `y y` or `Y` | Copy the block and its children as clean outl markdown to the OS clipboard |
| `y r` | Copy the block's reference `((blk-XXXXXX))` |
| `p` | Paste the clipboard after the selected block, converting outlines and paragraphs |
| `P` | Paste the clipboard after the selected block as one block of raw text |

Details, including SSH and tmux behaviour, are in [Clipboard](#clipboard).

**Pages and panels.**

| Key | What it does |
|---|---|
| `Ctrl+P` | Quick switcher: fuzzy-pick a page or journal |
| `/` | Slash command menu |
| `:` | Command line |
| `g p` | Property editor for the block |
| `g P` | Pin or unpin the page (`pinned::`) |
| `g d` | Delete the page, after a `y/n` |
| `g x` | Run the code block under the cursor, or open the link under it |
| `Ctrl+E` | Show or hide the sidebar |
| `Ctrl+B` | Show or hide the backlinks section |
| `Ctrl+O` | Flip the backlinks order |
| `g r` / `g R` | Add `remind:: 9am` to the block / the "nag me" preset |
| `g s` | Snooze this block's reminder one hour |
| `g n` | Open the reminders list |
| `*` / `#` | Search the workspace for the word under the cursor (forward / backward) |
| `n` / `N` | Next / previous hit of the last search |
| `?` | Help popup |
| `q q` or `Z Z` or `Ctrl+C` | Quit. A single `q` only arms the chord, so a stray key never quits |

`Z Z` is vim's "save and quit".
outl commits on every move back to Normal, so it does the same thing as `q q`.
A chord that is half pressed (`d`, `g`, `y`, `z`, `q`) waits for its second key.
Any key that does not complete it is handled as itself.

### Insert mode

Typing goes into one block.
`Esc` commits the block back to the page and returns to Normal.

| Key | What it does |
|---|---|
| `Esc` | Commit and return to Normal |
| `Enter` | Commit and start a new block below. Inside an open code fence it adds a line instead |
| `Alt+Enter` or `Ctrl+J` | New line inside the same block, works in every terminal |
| `Shift+Enter` | New line inside the same block, kitty-protocol terminals only |
| `Ctrl+T` or `Ctrl+Enter` | Cycle the task prefix without leaving Insert, caret follows the text |
| `Tab` / `Shift+Tab` | Indent / outdent and stay in Insert |
| `Alt+↑` / `Alt+↓` or `Ctrl+Shift+↑` / `Ctrl+Shift+↓` | Move the block and stay in Insert on it. Unsaved text and caret are kept. Closes an open popup |
| `↑` / `↓` | Move between lines of a multi-line block, then continue into the previous / next block |
| `←` at column 0 / `→` at the end | Continue into the previous block (caret at its end) / next block (caret at its start) |
| `Home` / `End` | Start / end of the line |
| `Delete` | Delete the character after the caret |
| `Backspace` | Delete before the caret. On an empty block it deletes the block and moves to the previous one. Between an empty `[[]]` or `(())` it removes the pair |
| `(` `[` `{` | Insert the closing character too. Typing the closing character yourself adds a second one, so leave the one outl added |

Other `Ctrl` and `Alt` chords do nothing in Insert and are never typed as text.
Readline editing (`Ctrl+A`, `Ctrl+W` and friends) is not implemented.
`↓` on the last block of the page does not continue into the backlinks section.
Press `Esc`, move there with `j`, and edit from Normal mode.

**Completion popups.**
They open as you type, and close when the text stops looking like a trigger.
`↑` / `↓` choose, `Enter` or `Tab` accept, `Esc` cancels.

| You type | You get | Notes |
|---|---|---|
| `[[` | Page names | Searches titles across the workspace. The page is created on first open if it does not exist |
| `#` | Tags | Word-initial only. Letters, digits, `-`, `_` and `/` |
| `@` | People | Word-initial only, so `a@b.com` does nothing. Lists pages with `type:: person`. Names may contain spaces (`@Thiago Avelino`). With no match, the typed name is offered as a new person. Inserts `[[@name]]` |
| `((` | Blocks | Fuzzy-matches block text and inserts `((blk-XXXXXX))`. With an empty query it lists the newest blocks first |
| `:` | Emoji | Word-initial, needs a lowercase letter first, so `14:00`, `key::value` and `https://` stay quiet. Inserts the `:shortcode:` text, and the screen shows the glyph |
| `/` | Commands | Word-initial, lowercase name characters only, so URLs and paths stay quiet. Accepting erases the `/query` and runs the command |

**Multi-line blocks and code.**
A block can hold several lines.
While you are inside an open fence (` ``` ` typed, no closing fence yet), plain `Enter` adds a line, so you can type a code block without remembering the soft-newline chord:

````
- ```lisp        type this, then Enter
  (+ 1 2)        type the body, Enter
  ```            type the closer, Enter: now Enter starts a sibling block again
````

The on-disk format is plain CommonMark: [markdown-format.md](markdown-format.md#multi-line-block-text-continuation-lines).

### Visual mode

`V` selects the current block.
`j` / `k` grow the range in either direction, and then one key acts on every block in it.

| Key | What it does |
|---|---|
| `j` / `k` or `↓` / `↑` | Extend the range |
| `d` or `x` | Delete the range |
| `y` | Copy the range as markdown to the OS clipboard |
| `Tab` or `>` | Indent every block in the range |
| `Shift+Tab` or `<` | Outdent every block in the range |
| `K` / `J` or `Alt+↑` / `Alt+↓` | Move the whole range up / down among its siblings. A parent selected with its children moves as one unit |
| `Esc`, `v` or `V` | Leave Visual |

A range whose blocks have different parents, or one that is already at the edge, refuses the move instead of scrambling.
Copy takes only the top-level blocks of the range, so a parent and a selected child are not duplicated.
`g v` in Normal brings the last range back.
Pasting and task cycling are not Visual operations.

## Finding and jumping

**Quick switcher (`Ctrl+P`).**
Type to fuzzy-match page titles, slugs and journal dates.
`↑` / `↓` choose, `Enter` opens, `Esc` closes.
The right side previews the highlighted page.
Today's date is always in the list, even when today's file does not exist yet.
Only the arrow keys move the selection here, because `j` and `k` are letters in the query.

**Search (`/` then `search`, or `:search`).**
Fuzzy-matches the text of every block in the workspace and shows the best 30.
It reads an in-memory index that builds in the background, so typing never re-reads your files.
Right after launch there is a short window where the header shows the indexing spinner and results may be partial.
`Enter` jumps to the block.
Afterwards `n` and `N` walk the same result list, with `hit 3/12` in the footer, wrapping at both ends.
`*` and `#` skip the overlay: they search for the word under the cursor, jump to the first hit and leave the rest for `n` / `N`.

**Sidebar (`Ctrl+E`).**
A 24-column pane with three sections:

| Section | What it holds |
|---|---|
| Calendar | The month you are viewing. `◉` today, `●` a day with a journal, `·` an empty day. Starts on Monday, or Sunday with `[calendar] week_start = "sunday"` |
| Pinned | Pages with `pinned:: true` |
| Recent | Pages you opened in this session, newest first, capped at 20 |

Opening it moves focus to the first non-empty section (Pinned, then Recent, then Calendar).
`j` / `k` move, `g` / `G` jump to first and last, `Tab` / `Shift+Tab` cycle sections, `Enter` opens the highlighted page, `Esc` hands the keyboard back to the outline and leaves the sidebar open, `Ctrl+E` closes it.
`Enter` on a calendar day does nothing yet.
Step through days with `[` and `]`, or type the date in `Ctrl+P`.

**Deleting a page.**
`d` on a Pinned or Recent row, or `g d` anywhere, shows `delete page '<title>'? y/n` in the footer.
`y` confirms, any other key cancels.
The page goes to the trash and its files are removed from disk.
If it was the page on screen you land on today's journal.
Journals cannot be deleted this way.

## Commands

Two doors, one list.
`/` opens a filterable menu with a description next to every command.
`:` is the vim-style command line.
Both run the same registry, so `/quit` and `:q` are the same thing.
Inside a block, typing `/` at the start of a word opens the same list inline.
A name that matches nothing says `unknown command: <name>` in the footer.

The most used ones:

| Command | What it does |
|---|---|
| `open <name>` | Open a page, creating it when it does not exist |
| `today` | Jump to today's journal |
| `search` | Fuzzy search over every block |
| `rename <name>` and `merge <page>` | Rename the open page and respell every link, or fold it into another page |
| `prop <key> <value>` | Set a `key:: value` property on the block |
| `date-today`, `date <arg>`, `time-now` and friends | Insert dates and times while typing |
| `upload` | Attach a file and insert its link |
| `template` | Insert or run a template |
| `trash` and `sync` | Open the trash or the sync panel |

The complete list, with aliases, arguments, date inserters, plugin commands and plugin settings, is in [TUI commands](tui-commands.md).

## Overlays

An overlay takes the keyboard until you close it.
`Esc` always closes it and `Ctrl+C` still quits.
Beyond the quick switcher, search and slash menu described above, there are four more:

| Overlay | Open it | What it is for |
|---|---|---|
| Property editor | `g p` | Browse and edit the block's `key:: value` pairs, or the page's |
| Reminders | `g n` | Every `remind::` rule, with snooze |
| Sync panel | `:sync` | What each paired device is waiting for, and merged edits you can undo |
| Trash | `:trash` | Everything you deleted, and the way back |

Keys and behaviour of each: [TUI overlays](tui-overlays.md).

## Clipboard

**Copy.**
`y y`, `Y`, and `y` in Visual write clean outl markdown to the OS clipboard.
Two routes are tried together:

1. `arboard`, the system clipboard on macOS, Windows and X11 (through XWayland on a Wayland session).
2. OSC 52, a terminal escape sequence that reaches your clipboard over SSH, inside tmux and in Chrome OS Crostini.

The footer says `yanked 2 blocks → clipboard`, or `yanked 2 blocks (clipboard unavailable)` when both failed.
The copy stays available to `p` inside the same session either way.
Pasting the markdown in another app rebuilds the same bullet structure.
tmux needs `set -g set-clipboard on` for OSC 52 to pass through.

On a bare X11 session (no desktop environment, no clipboard manager) outl itself serves the clipboard, so a copy can be pasted elsewhere only while the TUI runs.
With a clipboard manager, a normal quit hands the last copy over.
A crash, a kill or closing the terminal window skips that handover.
macOS keeps the text after quit.

**Paste.**
`p` pastes the system clipboard after the selected block.
Outlines, tables and paragraphs become structure.
`P` pastes it as one block, as typed.
Your terminal's own paste (`Cmd+V`, `Ctrl+Shift+V`) reaches outl as a bracketed paste, which behaves like `p` from Normal mode and as described in [paste.md](paste.md#in-the-tui) from Insert mode.
Conversion rules, tables, and what each client does differently: [paste.md](paste.md).

**Drag a file into the terminal (Insert mode).**
Terminals deliver a dropped file as its path pasted as text.
In Insert mode, when the pasted text is exactly the path of one or more existing files, each is imported into `<root>/assets/<hash>.<ext>` and its link is inserted at the caret.
Several files are inserted separated by spaces.
A path that is not an existing file pastes as plain text.
A file over `[assets] max_bytes` is refused with `import failed: …` and nothing is inserted.

## Mouse (opt-in)

Off by default, so your terminal's own text selection keeps working (Shift-drag to copy a URL).
Turn it on in `~/.config/outl/config.toml`:

```toml
[tui]
mouse_capture = true
```

| Gesture | Action |
|---|---|
| Scroll wheel | Move the selection one block |
| Click | Select the block under the pointer |
| Drag, then release | Select a range, then copy it as clean markdown to the clipboard (the same two routes as `y`) |

Clicks outside the outline (header, sidebar, the backlinks section below it) do nothing.
While capture is on, the terminal's native selection is off.
Hold the modifier your terminal documents (often Shift) to select text natively.
Keyboard copy works with or without it.

## Code blocks and queries

A fenced code block runs in place with `g x` or `:run`.
The result lands as a `> **result:**` child right below, and running again updates that same child.

```
- ```lua
  local s = 0
  for i = 1, 4 do s = s + i * i end
  print(s)
  ```
  - > **result:** `30`
```

| Fence | Engine | Notes |
|---|---|---|
| `js` | Boa | ES2015+, `console.log` is captured |
| `python` | RustPython | Python 3 subset, no native extensions |
| `lua` | Lua 5.4 | Sandboxed host surface |
| `lisp` | Steel | Scheme. Off by default: a Lisp fence cannot be sandboxed, so it is as trusted as a shell. Needs a build with the `lang-lisp` feature |
| `echo` | built in | Returns the source unchanged, for debugging |
| `query` | built in | Runs on every page load, no `g x` needed. See [query.md](query.md) |
| `call:<template>` | template | Runs a callable template. See [templates.md](templates.md) |

`g x` runs a code fence first.
Only when the block is not code does it look for a markdown link under the cursor (on its text or its URL) and open it.
An asset link opens in the OS default app.
Any other link opens in the browser, and only `http`, `https` and `mailto` are allowed.
Anything else is refused.
`Enter` still follows `[[page]]`, `#tag` and `((block))` as before.
`g x` on a `query` block only tells you that query blocks run by themselves.

A query result is a block whose whole text is one `!((blk-…))`.
`Ctrl+T` on it cycles the task state of the source block and re-runs the query, rather than prefixing the token.
`g D` on it completes the source block.

A `mermaid` fence stays as source in the terminal.
Open the page in the desktop or mobile app to see the diagram.

## What the page looks like

- **Block references** `((blk-XXXXXX))` show the source block's text and its page icon.
  An orphaned handle (the source is gone) is dimmed.
  `Enter` on one opens the source page and lands on the block.
  `ref ((x)) does not resolve` appears when the index cannot find it.
- **Embeds** `!((blk-XXXXXX))`, on a block that holds only that token, show the source block and all its children read-only underneath, each row prefixed with `↳ `.
  Nesting stops at four levels, which also breaks any embed loop.
  `![[page]]` is the page-level version: a chip with the page's icon and title, and the page's top-level blocks expanded below when the block holds only the token.
  The block under the cursor always shows the raw token.
- **Tasks** `TODO`, `DOING`, `DONE` render as `☐`, `◐`, `☑`, and the page header counts them.
- **Tables** render as a column-aligned grid when the cursor is elsewhere, and as raw pipes while you edit them.
  A table wider than the terminal wraps instead of scrolling, so widen the window.
- **Quotes**: a leading `> ` renders as a quote.
- **Empty page**: a page whose only block is empty says `press i to write · ? for keys`.
  The line goes away as soon as there is content.
  Deleting the last block keeps an empty `- ` so the cursor always has a home.
- **File names**: `[[Avelino]]` lives in `pages/avelino.md`, with `title:: Avelino` set on first open.

### Banners above the outline

**Outside the dialect** (yellow).
Opening a `.md` that has a leading `# heading`, a free paragraph between bullets, or imported markdown that does not fit shows:

```
┌─ ⚠ 3 line(s) outside outl dialect — preserved as blocks ─┐
│ line 1: # 2026-06-08 (+2 more)                           │
└──────────────────────────────────────────────────────────┘
```

Nothing is dropped.
Each such line becomes a normal block, and the next save rewrites the file in the dialect (`- <raw>`).
The footer repeats a short `⚠ N line(s) outside outl dialect — preserved` for terminals where the banner is hidden.
You can keep editing, save and move on.
`outl doctor` lists every page with active warnings ([doctor.md](doctor.md)).

**This page isn't syncing** (red, cannot be dismissed).
The file on disk holds lines outl never recorded, so the page is frozen in both directions until you fix it.
Edits from your other devices will not appear here, and yours will not leave.
The banner gives the exact command to run in the workspace folder, `outl reconcile --ahead-of-log`, and a sample of the unrecorded line.
It stays visible in focus mode on purpose.
Cause and recovery: [doctor.md](doctor.md).

## Saving, undo and safety

**Autosave, coalesced.**
Every commit (`Esc` from Insert, a structural change, undo) marks the page dirty and repaints in the same frame, with no disk write yet.
The `.md` write and op-log reconcile happen the moment you pause typing, and are forced after 600 ms if you keep typing.
They always flush before quitting, `Ctrl+S`, moving to another page, and a template or `call:` re-run.
The footer `saved` and the `4s ago` chip in the header tell you it is on disk.
`Ctrl+S` and `:write` flush at once and show a `saved` toast.

**An unreadable page is never overwritten.**
A permissions change, an I/O error, invalid UTF-8 or an iCloud file that has not finished downloading shows `cannot read <path> … editing disabled`, and saving that page is refused until a read works.
Without this guard, a failed read would parse as an empty page and the next save would write that emptiness over your notes and send every block to the trash on every device.
A page that does not exist yet is different: it opens as a normal empty page.

**Edits from other programs.**
When another editor saves the open `.md`, the TUI picks it up within about a second.
If you are in Insert at that moment it refuses to overwrite your edit and warns in the footer.
Finish, press `Esc`, then `Ctrl+L`.
Files that appear in the workspace without going through outl (an import, a `vim` save of a new page) are folded in by a background scan.

**Undo.**
`u` undoes and `Ctrl+R` redoes, over the last 200 snapshots.
Each snapshot remembers where your cursor was.
Undo is per page: a snapshot only applies to the page it was taken on, so `u` on a different page says `no undo for this page` and keeps the history.
Go back to that page and the edits are still there.
An edit made on a backlink row is undone on the page it changed, not on the page you were reading.
A move with nowhere to go (the first sibling up) records nothing.
Merges, renames and page deletes are not undone by `u`.
Rename back, or restore from the trash.

**Trash.**
`d d` and a page delete are moves.
`/trash` brings them back from any day and any device (see [TUI overlays → Trash](tui-overlays.md#trash-trash-trash)).

**Backups.**
While the TUI is open, a background thread keeps local snapshots of the workspace (`[backup]`, on by default), in a git repository outside your notes folder.
`outl backup list` and `outl backup restore` use it.
See [config.md → `[backup]`](config.md#backup).

**Sync.**
In a workspace that syncs, the TUI also starts the sync transport.
The footer says `iroh sync enabled`, or `iroh sync off here: <reason>` when another outl process on this device already owns the connection (that is a working state, not a failure).
When P2P cannot start, a toast says `iroh sync unavailable, using filesystem sync` and file-based sync keeps working.
Peer changes arrive while you are in Normal mode.
While you type in Insert, incoming changes wait and are applied the moment you leave Insert, so your text is never swapped out from under you.
Pairing a device is not done in the TUI: `outl peer pair` or `outl peer qr` ([sync.md](sync.md)).

**Captures.**
`outl capture "your line"` drops a line into the capture inbox from any terminal, even with the TUI closed.
The TUI lands what is waiting when it starts and says `added 1 capture to your journal`.
Each line becomes a block at the end of the journal for the day it was captured, not the day it was landed.
A capture made at 23:58 and landed at 00:10 belongs to the 23:58 day.
It is one undo step.
Nothing watches the folder while the TUI runs, so a capture made meanwhile waits for the next start, which is the difference recorded in [client-parity.md](client-parity.md).
A file that cannot be read stays in the inbox and is named, with its folder, in the footer.
`outl capture --list` shows what is waiting.
More: [client-outside-the-app.md](client-outside-the-app.md#capturing-from-outside-the-app) and [cli.md → Capture](cli.md#capture).

## Configuration

Two layers of TOML are read at launch.

1. **Global**: `~/.config/outl/config.toml`, shared with the desktop app.

   ```toml
   [theme]
   preset = "dracula"

   [tui]
   icons = "nerd-font"     # "emoji" (default) or "nerd-font"
   mouse_capture = false   # true turns on wheel, click and drag

   [display]
   backlinks_order = "newest"   # or "oldest", also flipped by Ctrl+O

   [calendar]
   week_start = "monday"        # or "sunday", for the sidebar calendar
   ```

2. **Per workspace**: `<workspace>/.outl/config.toml`.
   A `[theme] preset` here overrides the global one for this workspace only.
   The actor id this device writes under lives in the device store, not here ([config.md](config.md#where-the-actor-id-actually-lives)).

Theme order: `--theme` flag, workspace `[theme]`, global `[theme]`, then the built-in `outl` default.
A name that does not exist falls through silently to the next level.
`[editor] vim_mode` and `font_size` are desktop settings and the TUI ignores them.
A `config.toml` that fails to parse shows its notice in the footer on the first frame, and every setting falls back to its default until you fix it.
Every key, with defaults: [config.md](config.md).

## What the TUI does not do

Recorded per feature in [client-parity.md](client-parity.md), which is generated from the code.
In plain words:

- **No inline formatting chords.**
  Type the markers: `**bold**`, `_italic_`, `` `code` ``, `~~strike~~`, `[label](url)`.
- **No block cut or block copy-with-subtree-paste.**
  Use `d d` then paste, and `y y` for a block with its children.
- **No page history view.**
  Open the page in the desktop app.
- **No one-key quick capture.**
  `t` to go to today, or `outl capture` from another terminal.
- **No task list view.**
  Add a `query` block with `status: open` on any page ([query.md](query.md)).
- **No calendar navigation, no nested-page navigation, no backlink-source jump.**
  `Ctrl+P` and the name reach all three.
- **No plugin marketplace, no peer pairing, no export.**
  Use `outl plugin install`, `outl peer pair` and the export commands in [cli.md](cli.md).
- **No diagram rendering and no on-screen settings.**
  Edit `config.toml`.
- **No graph view.**
  No client has one.
- **No live multi-user editing.**
  Two people editing the same workspace at the same moment is out of scope.
  One person on several devices is the supported case.
- **Reminders fire only while the TUI is open.**

## Related

- [TUI quickstart](tui-quickstart.md) and [TUI troubleshooting](tui-troubleshooting.md)
- [Shortcuts](shortcuts.md) and [Client parity](client-parity.md)
- [Paste](paste.md), [Markdown format](markdown-format.md), [Queries](query.md), [Templates](templates.md), [Reminders](reminders.md)
- [Theming](theming.md), [Config](config.md), [Plugins](plugins.md), [Sync](sync.md)
