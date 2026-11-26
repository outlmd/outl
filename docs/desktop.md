# Using outl on the desktop

The desktop app is outl in a window: a journal, an outline editor, a sidebar and a handful of panels.
It runs on macOS, Windows and Linux, and it reads and writes the same workspace folder as the TUI and the CLI.

This page is the tour.
Each section says what you can do, how, and where the desktop differs from the other clients.
Look-ups live elsewhere: every chord is in [Shortcuts](shortcuts.md), every setting is in [Desktop settings](desktop-settings.md), and something that misbehaves is in [Desktop troubleshooting](desktop-troubleshooting.md).

If you have never run outl, start with [Getting started](getting-started.md) for the install, then come back.

## First launch

1. **Pick a folder.**
   The first screen asks for a workspace folder.
   Choose an empty one, or one that already holds an outl workspace.
   A folder inside iCloud Drive, Dropbox or Syncthing works if you want a file-synced copy, but you do not need one: outl can also sync device to device.
2. **Decide on sync.**
   The second step offers to pair another device.
   Skip it if you only use one.
   A single device is a complete setup, and you can pair later from Settings.

Your journal for today opens.
Type.

A launch with a large workspace shows today's journal read-only for a moment while the rest loads.
Why, and what the budgets are: [Boot performance](boot-performance.md).

Launching outl a second time does not open a second window.
It brings the running one to the front.

## The window

```text
┌───────────────────────────────────────────────────────────────┐
│ [◫]                       [⌄ ⌃ … 🗑 🔔 ▦ ● ◎ ⚙]                │  two floating capsules
├───────────┬───────────────────────────────────────────────────┤
│ Search    │  page title                              ⏱  📎    │
│ Today     │  key:: value                                      │
│ calendar  │  • block                                          │
│ Pinned    │    • child                                        │
│ Recent    │  • block                                          │
│           │  Linked from  (backlinks)                         │
├───────────┴───────────────────────────────────────────────────┤
│ NORMAL · hints for the mode you are in                        │
└───────────────────────────────────────────────────────────────┘
```

The window opens at 1280 by 800 and cannot go below 800 by 600.

**Top-left capsule.**
One button, the sidebar toggle (`Cmd/Ctrl+Shift+E`).
The sidebar starts hidden: the first thing you see is the editor, and you open the sidebar when you want it.

**Top-right capsule**, left to right:

| Button | What it does |
|---|---|
| Fold all, Unfold all | Collapse or expand every block on the page (`z M`, `z R`) |
| Plugin buttons | One per plugin that contributes a toolbar button, if you have any |
| Trash | Opens the [trash](#getting-a-deleted-block-back) |
| Reminders | Opens the [reminders list](#reminders) (`Cmd/Ctrl+Shift+R`) |
| Plugins | Opens the [plugin browser](#plugins) |
| Sync dot | Shows whether a paired device is reachable, and syncs now when clicked |
| Focus mode | Hides the chrome so only the outline is left (`Cmd/Ctrl+Shift+F`) |
| Settings | Opens [settings](desktop-settings.md) (`Cmd/Ctrl+,`) |

Every button is also reachable from the keyboard: `F6` and `Shift+F6` walk between the regions of the window (the capsules, the sidebar, the outline), and the arrow keys move inside one.

**Sidebar.**
Top to bottom: Search (opens the palette), Today, a month calendar, Pinned pages, and Recent pages.
A day in the calendar with a journal shows a dot, today has a ring, and clicking a day opens or creates that journal.
Pinned is every page with `pinned:: true`.
Recent is the pages you opened lately **on this device**, and it is not synced: what you opened on your laptop is no business of your phone.
Hover a regular page row to get **Rename**, **Merge into...** and **Delete**.

**Page header.**
The title, the page's own `key:: value` properties, and two icons: attach a file, and page history.

**Backlinks.**
Under the outline, "Linked from" lists every block that cites this page.
It is on by default and you can edit a cited block in place: see [Backlinks](#backlinks).

**Status bar.**
A mode badge (NORMAL, INSERT, VISUAL, OVERLAY) and the keys that matter in that mode.
Errors and notices appear here.

## Find and open things

`Cmd/Ctrl+P` opens the palette.
It lists, in this order, pages and journals, tasks, blocks whose text matches, and actions.

- Type part of a name to jump to a page.
  If nothing matches, `Enter` creates a page with that name.
- Type a date (`2026-06-04`, `today`, `yesterday`) to open that day's journal.
- Start with `>` to see actions only, each with its chord.
  `Cmd/Ctrl+K`, or `:` with nothing focused, opens the palette already on `>`.
- Block results need a few characters before they appear, and they arrive a moment after the pages do.

Other ways around:

| To | Press |
|---|---|
| Open today's journal | `Cmd/Ctrl+J` |
| Previous or next journal day | `Cmd/Ctrl+[` and `Cmd/Ctrl+]` |
| Follow a `[[link]]` or `#tag` | Click it |
| Search for the words of the selected block | `*` with the block selected |
| See every shortcut | `?` with nothing focused |

## Write

A block is a bullet.
Click one, or select it with `j` and `k` and press `i`, and you are editing it.
`Esc` saves and leaves.
Every change is written the moment it commits: there is no Save command and nothing to lose by closing the window.
A small dot fades in beside a block once its edit is on disk.

**The modes.**
outl borrows vim's idea but does not require vim.
With nothing focused you are in **Normal** mode, where single keys act on the selected block (`o` new block, `dd` delete, `c` fold).
Inside a block you are in **Insert** mode, where keys type.
`Shift+Up` and `Shift+Down` start **Visual** mode, a range of blocks.
The `Vim mode` checkbox in Settings does not turn any of this on or off on the desktop: see [Desktop settings](desktop-settings.md#vim-mode).

Inside a block:

| To | Press |
|---|---|
| Start a new block | `Enter` (splits the block at the caret) |
| Break the line inside the block | `Shift+Enter` |
| Indent, outdent | `Tab`, `Shift+Tab` |
| Move the block up or down | `Alt+Up`, `Alt+Down` (or `Cmd/Ctrl+Shift+Up/Down`) |
| Bold, italic, code, strikethrough, link | `Cmd/Ctrl+B`, `I`, `E`, `Shift+X`, `K` |
| Walk into the neighbouring block | `Left` at the start, `Right` at the end, `Up` and `Down` on the first and last line |

Typing a trigger opens a popup.
`Up` and `Down` choose, `Enter` or `Tab` accepts, `Esc` closes.

| Type | You get |
|---|---|
| `[[` | Pages to link to |
| `((` | Blocks to reference |
| `#` | Pages to use as a tag |
| `:` and a letter | Emoji shortcodes |
| `/` at the start of a word | The slash menu: actions, date inserters, `/upload`, your templates, plugin commands |

`/date-today` writes a date into the block.
`/today` opens the journal.
`/quote` turns the block into a blockquote.
The same names work in the TUI.
Templates are covered in [Templates](templates.md).

Rules you will meet:

- **A `[[link]]` to a page that does not exist creates it** when you follow it.
- **A tag is a page.**
  `#project` and `[[project]]` open the same thing.
- **Pasting** keeps structure when what you paste is an outline, and splits plain text into one block per line.
  `Cmd/Ctrl+Shift+V` pastes as plain text.
  Details: [Paste](paste.md).
- **A fenced code block can run** (JavaScript, Python, Lua and Rust on the desktop).
  Commit the block with `Esc`, then press `Cmd/Ctrl+Shift+X`, or use the Run button on the block.
  The output lands in a `> **result:**` block under it.
  Details: [Running code blocks](exec.md).
- **A fence marked `query`** shows live results from your workspace: [Query code blocks](query.md).
- **A `mermaid` fence** renders as a diagram unless you turn diagrams off in Settings.
- **Embeds.**
  A block whose whole text is `!((blk-…))` shows that block and its children below it, read-only.
  A block whose whole text is `![[page]]` shows the whole page the same way.
  An embed counts as a reference, so the embedded page lists it in its backlinks: [Markdown dialect](markdown-format.md#page-embeds-page).

### Tasks

`Cmd/Ctrl+T` cycles the selected or edited block through `TODO`, `DOING`, `DONE`, and back to plain.
`g D` marks it `DONE` outright.
Clicking the checkbox glyph of a task does the same cycle.

`g t`, or `Cmd/Ctrl+Shift+L`, opens the task list: every open `TODO` and `DOING` in the workspace, grouped by page.
Selecting a row opens the page and selects the block.
The list shows the first 200 and prints the real total in its header, as in `200 of 1431 open`.
Past 200, use a [query block](query.md) with `status: open`.

### Reminders

Write `remind:: 3pm` on a block's second line, or select the block and press `Cmd+R` (or `g r` in Normal mode).
`Cmd/Ctrl+Shift+R` opens the list, grouped Today, Tomorrow, This week, Later, Done.
Each row can be snoozed (`1h`, `tomorrow`, `next week`), resumed or marked done.
The syntax, `every` and `until`: [Reminders](reminders.md).

Notifications are on by default and need the OS to allow them the first time one fires.
They fire only while outl is running.
Settings has a switch and a quiet-hours window.

### Properties

Any line of the form `key:: value` under a block is a property, drawn as a chip.
`+ prop` at the end of the chip row (visible on hover), or `Cmd/Ctrl+Shift+P`, adds one.
The key field completes from the keys your workspace already uses.
`×` on a chip removes it.
Page properties are the chip row under the page title.

## Organise

**Fold and zoom.**
`c` folds the selected block.
`z i` (or `Cmd/Ctrl+Shift+]`) zooms into it, so the block becomes the page title and its children the outline.
The path above the title takes you back up, one crumb at a time, and `z o` goes up a level.
While zoomed, the page's properties, backlinks and nested-page list are hidden, because they belong to the page and not to the block.
Opening any page, including the one already open, leaves the zoom.

**Move and select several.**
`Shift+Up` and `Shift+Down` grow a selection.
A floating bar offers Indent, Outdent, Move up, Move down, Delete and Done.
`y` (or `Cmd/Ctrl+C`) copies the range as outl markdown, `d` deletes it.

**Cut, copy, paste a block.**
With a block selected and nothing focused, `Cmd/Ctrl+X` moves it with its children, and every `((reference))` to it keeps working after the paste.
`y y`, `Y` or `Cmd/Ctrl+C` copies it, with its children, to the system clipboard as outl markdown, so you can paste it into any other app too.
`p` pastes the system clipboard as blocks after the selection, and `Cmd/Ctrl+V` does the same unless a cut is waiting, in which case it moves the cut block.
The selection lands on the last pasted block.
Copying cancels a pending cut.
Where a paste lands and what it converts: [Paste](paste.md#paste-after-the-selected-block-desktop-normal-mode).

**Right-click a bullet** for **Copy block ref** and **Copy block embed**.
A ref, `((blk-…))`, links to the block.
An embed, `!((blk-…))`, shows the block's content wherever you paste it.
A block you created a moment ago may not have a handle yet: wait a second and retry.

**Attach a file.**
Drag a file from the desktop onto a block, use the paperclip in the page header, or type `/upload`.
The file is copied into the workspace's `assets/` folder and linked.
Clicking the link opens the file in its default app.
There is a size cap, `[assets] max_bytes` in the [config](config.md).

### Pages

| To | Do |
|---|---|
| Rename | Click the page title, type, `Enter`. `Esc` cancels |
| Rename or merge from the list | Hover the sidebar row, then **Rename page...** or **Merge into...** |
| Delete | Hover the sidebar row and click `×`, or `g d` on the page |
| Pin | Add `pinned:: true` as a page property, or press `g P` |

Journals cannot be renamed or deleted: their name is their date.
The sidebar's Today and the page title's "Today" label follow the real date, so a window left open overnight relabels itself at midnight and stays on the page you were looking at.
When you come back to the window after a day change, or after it was hidden for 30 minutes or more, outl moves to today's journal, unless you are in the middle of editing a block.

Renaming rewrites every link to the page in the same step.
If the new name is taken, outl asks whether to merge into the page that holds it, and writes nothing until you say yes.
**A merge has no undo.**
The blocks stay under the target, the links stay rewritten, and the old name becomes an alias.
After either, outl opens the resulting page and a message says how many blocks moved and links changed, and names any property both pages held with different values (the target's wins).

Deleting a page asks first.
How a page comes back: [A page over its life](client-page-lifecycle.md).

### Getting a deleted block back

Deleting never destroys anything.
The trash button lists every block deleted in this workspace, from any of your devices, with how long ago and how many blocks come back with it.
**Restore** puts it back under the block it was deleted from and opens that page.
A row that cannot come back says why in place: for example, its page is gone, or its parent was deleted later.
A deleted page is handled on the page's own terms, in [A page over its life](client-page-lifecycle.md#reopening-a-deleted-page).

### What happened to this page

The clock icon in the page header lists what changed on the page, newest first, from every device.
Edits show as a word-level diff.
An edit has a **Restore** button that puts the previous text back.
A deletion has one that brings the lost text back as a new block at the end of the page.
Moves and property changes are listed without a button.

`u` or `Cmd/Ctrl+Z` undoes the last block change on the page you are on, and `Ctrl+R` or `Cmd/Ctrl+Shift+Z` redoes it.
Folding is not undoable.
If another device changes a page, that page's undo history resets, so undo never reverts someone else's edit.

## Backlinks

"Linked from" under the outline lists blocks that cite this page, grouped by source page, with their children.
A direction button in its header flips newest first and oldest first, and the choice is remembered.

You do not have to leave to act on a citing block:

- Click the text to edit it where it lives, with the same popups and chords.
  `Esc` saves.
- Click the checkbox glyph to cycle its task state.
- Press `j` at the last block of the outline to step into the list.
  `i` or `Enter` edits, `dd` deletes, `Tab` and `Alt+Up/Down` restructure.
  Each acts on the block **on its own page**.
- Click the group's page title to go there.
  The title is a button, so it is also a keyboard stop.

The section appears when it has something to show, and never leaves a gap under a page that nothing cites.
Rules and edge cases: [Backlinks on every client](client-backlinks.md).

A page with children under a namespace, such as `projects/outl`, lists them as nested pages under its own outline.

## Outside the app

**Quick capture.**
`Cmd/Ctrl+Shift+Space` opens a one-line box and adds the line to the end of today's journal.
It works from any app, because outl registers the chord with the operating system, and it brings the window forward.
`Enter` saves, `Esc` closes, `Shift+Enter` breaks the line.
If the write fails the text stays in the box and the reason is shown.
If another application owns that chord, outl starts anyway and the chord only works while outl has focus.
The command palette and `outl://capture` still open the box.

**From a terminal or a script.**
`outl capture "text"` leaves a line that the app lands on the next launch or when you return to it.

**Open With.**
`.md`, `.markdown`, `.txt` and `.text` files list outl under Open With.
It never becomes the default.
The file is imported as a page named `open-in/<file name>` and linked from today's journal, and opening the same file again goes to that page instead of importing twice.
The original file is never written to.

**Links.**
`outl://daily/today`, `outl://daily/2026-06-25` and `outl://page/<slug>` open the app on that target: [Deep links](deep-links.md).

Both are covered in [When outl is not the app in front of you](client-outside-the-app.md).

## Sync

The dot in the top-right capsule has four states:

| Dot | Meaning |
|---|---|
| Green | At least one paired device is reachable |
| Orange | None is reachable right now |
| Dim | The first check is still running |
| Spinner | A sync is in progress |

A device that can only reach you, not the other way (a phone on a mobile network), counts as reachable.
Hover for when the last sync landed.
Click to sync now.
Right-click to open Settings at the sync section, where you pair or remove devices and read per-device activity.
Pairing step by step: [Sync your devices](sync-devices.md).
Why sync works this way: [Sync](sync.md).
Choosing the folder-based transport instead of device to device: [Desktop settings](desktop-settings.md#sync-transport).

## Plugins

The plugins button opens a browser: search, **Install** for official ones, and enable, disable or remove for the ones you have.
Plugins can add toolbar buttons, slash commands and chords.
Writing one: [Plugins](plugins.md).

## Workspaces

One workspace is open at a time.
**Settings, Workspace, Switch...** changes it on every OS.
On macOS **File, Switch Workspace...** does the same.
Your pages, selection and panels reset to the new workspace's today.

## What differs by operating system

| | macOS | Windows | Linux |
|---|---|---|---|
| Menu bar | Yes: outl, File, Edit, View, Window, Help | No | No |
| About window with version and links | Yes | No | No |
| Help menu: GitHub, Report a Problem, Sponsor | Yes | No | No |
| Switch workspace | File menu or Settings | Settings | Settings |
| Modifier key | `Cmd` | `Ctrl` | `Ctrl` |
| Install | Homebrew cask | Scoop | AppImage, `.deb`, `.rpm` |

The full per-client matrix, including what the desktop does not do: [Client parity](client-parity.md).

## What the desktop does not do

- **No character cursor in Normal mode.**
  `x`, `D`, `C`, `r`, `f`, `F`, `~` and `e` show a message instead of firing, because the desktop selects blocks, not characters.
  Edit inside the block with `i`.
- **No chord for the trash, page history or `/quote`.**
  Use the buttons, or type `> ` at the start of a block for a quote.
- **Reminders fire only while the app runs.**
  See [Reminders](reminders.md#background-delivery--what-ships-today).
- **No slash `/date` with an argument.**
  Use `/date-today` and the other fixed inserters.

## Where to go next

- [Shortcuts](shortcuts.md): every chord, side by side with the TUI and mobile.
- [Desktop settings](desktop-settings.md): each setting, what it does, where it is stored.
- [Desktop troubleshooting](desktop-troubleshooting.md): what each message means and what to do.
- [Markdown dialect](markdown-format.md): what is on disk.
- [Your first week with outl](tutorial.md): a worked routine.
