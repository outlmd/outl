# outl on your phone

The user guide for the iPhone, iPad and Android app.
It covers what is on the screen, how to write, how to find things, how to sync with your other devices, and what the phone does differently from the desktop.

Contributors: how each surface is built is in [`mobile-ux.md`](mobile-ux.md), [`mobile-components.md`](mobile-components.md) and [`mobile-internals.md`](mobile-internals.md).
If something here is wrong or missing, that is a bug in the docs: open an issue.

Related pages:

- [Capture without opening the app](mobile-capture.md): widget, Shortcut, share sheet, Control Center, quick settings tile.
- [When something does not work](mobile-troubleshooting.md): sync dot, pairing, notifications, captures that have not landed.
- [Reminders](reminders.md) and [Sync](sync.md): the full behaviour behind the phone's reminders and sync screens.

---

## What you get

The phone app is the same outliner as the desktop and the terminal, over the same files.
A page you write on the phone is a plain `.md` file that syncs to your other devices without a server or an account.

| | Phone |
|---|---|
| Journal, pages, `[[links]]`, `#tags`, backlinks | yes |
| Tasks (`TODO` / `DOING` / `DONE`), a list of every open task | yes |
| Reminders with snooze and "Done" buttons on the notification | yes, while the app is running |
| Sync with your laptop (peer to peer, no cloud) | yes |
| Properties (`key:: value`), page history, rename and merge | yes |
| Code blocks that run on the device (JavaScript, Python, Lua) | yes |
| Mermaid diagrams | yes |
| Plugins from the official registry | yes |
| Add a line from the Home Screen, Lock Screen, share sheet or a Shortcut | yes, see [capture](mobile-capture.md) |
| Vim keys, chords, a sidebar | no, the phone is touch-first and single pane |
| Restore a deleted block from the trash | not yet, do it on a computer |
| Opening a `.md` file from another app into outl | not yet |

The per-action verdicts for every client are in [`client-parity.md`](client-parity.md), which is generated from the code.

**Requirements.**
iPhone or iPad on iOS 17 or later.
Android 7.0 (API 24) or later, on an arm64 device.

---

## Install

**iOS.**
Join the TestFlight beta from the link in the [README](../README.md#install), install the app, open it.

**Android.**
Download `outl-android-arm64.apk` from the latest [GitHub release](https://github.com/outlmd/outl/releases) and open it.
Android asks once to allow installs from your browser or file manager.

---

## First launch

Two screens, then your journal.

1. **Where do your notes live?**
   On this device.
   The workspace is a folder inside the app, and there is no account and no cloud.
   Tap **Keep on this device**.
   iCloud and arbitrary folders are not offered on the phone today.
2. **Sync (optional).**
   Pair with your laptop now, or tap the skip button.
   A single device works fine, and you can pair later from Settings.
3. **What's what.**
   One sheet that names the bottom bar, the `+`, the week strip, the month label and the block long-press.
   It opens once.
   Re-open it any time from **Settings → What's what**.

On an empty journal outl also shows a short tip about the outline: every line is a block, and `[[` links a page.

---

## The screen

```
┌──────────────────────────────────────────┐
│ ↩  ‹ Today, Nov 25 ›      ⇊ ⇈ ⧉ ● ◐ ⚙   │  header
│  Nov 2026 ⌄   M  T  W  T  F  S  S        │  week strip
│                                          │
│  ▸ a block                               │
│    ▸ a child block                       │  outline
│  ☐ a task                                │
│                                          │
│  Linked from ...                         │  backlinks, nested pages
│                                          │
│  [Journal] [Search]  (+)  [Tasks] [Rem.] │  bottom bar
└──────────────────────────────────────────┘
```

**Header.**

| Control | What it does |
|---|---|
| Back arrow (left, only off a journal) | Back to today's journal. |
| Title with `‹` `›` | On a journal, steps one day. Tap the title to jump to today. On a page, a tap renames it and a long press opens its properties. |
| Fold all, Unfold all | Collapse or expand every block on the page. |
| Plugin buttons | One glyph per toolbar button a plugin contributes. |
| Plugins (four squares) | Opens the plugin sheet: **Commands** to run one, **Browse** to install from the official registry. |
| Sync dot | Tap to sync now. Press and hold to open **Devices and sync**. Colours are in [the sync section](#sync-with-your-other-devices). |
| Focus mode | Dims everything except the block you are writing in. Tap again to leave. |
| Gear | Settings. |

**Week strip.**
Tap a day to open it.
A dot under a day means that journal has text.
Tap the month label to open the full calendar.
**Today** appears whenever you are on another day.
Swipe sideways on the strip to step one day.

**Bottom bar.**

| Button | What it opens |
|---|---|
| Journal | Today. |
| Search | The one search box over pages, tasks, blocks and actions. The `+` inside it starts a new page. |
| `+` (centre) | On today's journal it adds a block in place. From anywhere else it opens a small sheet that appends a line to today and stays open, so a burst of thoughts is one gesture each. |
| Tasks | Every open task across the workspace, grouped by page. Tap one to jump to it. |
| Reminders | Every block with a `remind::` rule, grouped Today, Tomorrow, This week, Later, Done. |

The bar hides while you edit a block or select a range, because the keyboard toolbar takes its place.

**Refresh.**
Pull down on a page to sync and reload.
The sync dot does the same.

---

## Write

### Edit a block

Tap a block to edit it.
Return starts the next block.
Tap outside, or the keyboard button on the right of the toolbar, to finish.

The keyboard has a toolbar above it.
`+` (new block) is pinned on the left and hide-keyboard on the right.
The buttons between them scroll sideways:

| Button | Does |
|---|---|
| New line | New block after this one. |
| Indent, Outdent | Nest under the block above, or back out. |
| `[[` | Starts a page link. A strip of matching pages appears above the keyboard. |
| Toggle TODO | Cycles the block's task state. |
| Bold, Italic, Code | Wrap the selection in `**`, `*` or backticks. |
| Move up, Move down | Reorder among siblings. |
| Undo, Redo | Step back and forward through your edits. |
| `#` | Starts a tag. The strip lists pages, because a tag is a page. |
| `((` | Starts a block reference. |
| Delete block | Removes the block (asks first when it has children). |

The order of the middle buttons follows what you use most.
It only changes between editing sessions, never while the keyboard is up, so a button you just pressed stays where it was.
To freeze it, open **Settings → Keyboard toolbar → Lock button order**.
**Reset button order** forgets your usage counts.
Both apply the next time the keyboard opens and are per device.

Missing on purpose: strikethrough and a link button.
Type `~~text~~` and `[label](url)` by hand.
Typing `/` does not open a command menu either.
Plugin commands are behind the four-squares button, and templates are in the long-press menu.

### Tasks

A block that starts with `TODO` is a task.
Tap its checkbox to walk `TODO`, `DOING`, `DONE`, then back to a plain block.
**Tasks** in the bottom bar lists everything still open.

### Links, tags and references

| Type | Result |
|---|---|
| `[[Page name]]` | Link to a page. Tapping it opens the page, and creates it if it does not exist yet. |
| `[[2026-11-25]]` | Link to that day's journal. |
| `#tag` | Same as a link to a page called `tag`. |
| `((blk-…))` | Shows the text of another block. Copy the handle with **Copy block ref** in the long-press menu. |
| `!((blk-…))` | Embeds that block with its children. Use **Copy block embed**. |
| `[label](https://…)` | Opens in the system browser. Only `http`, `https` and `mailto` open. |

Every page lists what links to it under **Linked from**.
Tap a row there to edit that block in place, or its checkbox to toggle the task.
Tap the row's header to open the page it lives on.

### Move around a page

- **Tap a block's bullet** to zoom into it.
  It becomes the whole page, with a breadcrumb and a back button above it.
- **Tap the triangle** next to a block with children to fold it.
- **Swipe a block left** to delete it.
  A block with children asks first.
- **Long press** a block, or its bullet, for the menu below.

### The long-press menu

Press and hold for about half a second.
Moving your finger more than a few points cancels it.

| Row | Does |
|---|---|
| Run `<language>` | Only on a code fence. Runs it on the device and writes the result under it. JavaScript, Python and Lua run on iOS and Android. |
| Toggle TODO | Same as the checkbox. |
| Select blocks | Starts a range selection, see below. |
| Reselect last selection | Brings the previous range back. |
| Copy text | The block's text, as Markdown, to the clipboard. |
| Cut block, Copy block, Paste block | Whole-block clipboard. Paste appears once you have copied one. |
| Copy block ref, Copy block embed | `((blk-…))` and `!((blk-…))`. |
| New block above | Adds a sibling above and opens the keyboard on it. |
| Remind me… | Opens the reminder sheet, see [Reminders](#reminders). |
| Properties… | The block's `key:: value` fields. |
| Insert template | Pick a structural template to stamp under this block. |
| Attach file | Pick a file; it is stored with the workspace and linked from the block. |
| Indent, Outdent, Move up, Move down | Structure. Move rows hide at the ends of the list. |
| Delete | Removes the block and asks first if it has children. |

Long press a block in a **Linked from** section for a shorter menu: **Toggle TODO**, **New block below**, **New block above**, **Indent**, **Outdent**, **Move up**, **Move down** and **Delete**.

> Cut moves a copy: the block gets a new id, so any `((blk-…))` that pointed at the original goes stale.
> Copy the block ref first if something else cites it.

### Select several blocks

1. Long press a block and choose **Select blocks**.
2. Tap other blocks to stretch the range, or use the up and down arrows in the bar to grow it one row at a time.
3. The bar at the bottom acts on the whole range: **Indent**, **Outdent**, **Move ↑**, **Move ↓**, **Copy**, **Delete**.
4. **Done** leaves the selection without changing anything.

A parent selected with its children moves as one unit.

### Properties

A block's `key:: value` fields show as one dim line under its text, with `+N more` past two fields.
Tap the line to open the properties sheet.
Long press the page title to open the page's own properties, and from there **Page history**.

History lists what the op log recorded for the page.
A restore from there is a new edit, so it syncs and can itself be undone.

### Rename or merge a page

Tap a page title to rename it.
If the new name is taken, outl asks whether to merge this page into the one that holds it.
Nothing is written until you accept.
You can also long press a page in the **Search** sheet for **Rename page...** and **Merge into...**.
Journals cannot be renamed, because the date is the name.
There is no undo for a merge.

### Paste

Pasting from another app converts rich text to Markdown.
Plain text that looks like an outline, or has several paragraphs, becomes one block per item.

### A hardware keyboard

With a keyboard attached (iPad, or a phone with one), the arrow keys walk between blocks: left at the start of a block goes to the end of the previous one, down on the last line goes to the next block.
At the top or bottom of the page the key behaves normally.

---

## Find things

**Search** shows pages, tasks, blocks and actions in one pass.
With an empty query it lists your pages, 25 of them, in slug order.
Each section opens at three rows with an **N more** button under it.
Start the query with `>` to see only actions.
Pinned pages come first.
Each page row has a pin button on its right edge, and journals cannot be pinned.

**Start a page that is not a daily note:** tap the `+` next to the search box and type a name.
A name that already exists opens that page instead of making a second one.

**Calendar:** tap the month label.
Days with text carry a dot.

---

## Reminders

Put a rule on a block as a property: `remind:: 3pm every 1h until DONE`.
You do not have to type it.

1. Long press the block and choose **Remind me…**.
2. Pick a time, a repeat and, for repeating rules, an "until".
   **Edit as text** shows the rule exactly as it will be written, and anything the pickers cannot express stays reachable there.
3. **Remove** stops the block from nagging without deleting it.

The **Reminders** tab lists every rule.
Each row has snooze chips (**1 hour**, **Tomorrow 9am**, **Next week**) and **Done**.
A snooze silences the reminder on every device you own.

At the top of the tab:

- **Notify me on this device** turns delivery on or off for this phone only.
- **Quiet hours** takes two times.
  A reminder that comes due inside the window fires after it ends, it is not dropped.

The first time something is due, the OS asks for notification permission.
A banner carries **Snooze 1h** and **Done** buttons.
Tapping the banner opens the page and scrolls to the block.

**Reminders fire only while outl is running**, in the foreground or the background.
If the app has been closed or the OS has reclaimed it, the notification does not arrive until you open outl again.
App-closed delivery is not built yet, see [reminders.md](reminders.md#background-delivery--what-ships-today).

---

## Capture without opening the app

Add a line to today from the Home Screen, Lock Screen, share sheet, Control Center, a Shortcut or the Android quick settings tile.
The steps for each are in [mobile-capture.md](mobile-capture.md).

---

## Sync with your other devices

outl syncs peer to peer.
There is no server holding your notes and no account.
Two devices that you paired exchange changes directly, over your Wi-Fi when they are on the same network and through an encrypted relay when they are not.

### Pair a phone with your laptop

1. On the laptop, show a pairing code: the desktop app's Sync panel (pair a new device), or `outl peer pair` in a terminal, which prints it as a QR.
2. On the phone, open **Settings → Devices and sync** (or press and hold the sync dot) and tap **Scan QR**.
3. Allow the camera when asked, and point it at the code.
4. The device appears in the list.
   Notes start to move.

Going the other way works too: **Show my QR** on the phone, scan it from the other device.
Pairing codes are single use and expire.
The first time outl looks for devices on your Wi-Fi, iOS asks for **Local Network** access.
Allow it, or the phone cannot find a laptop on the same network.

The Devices sheet also shows:

- a name for this device (what your other devices call it);
- live sync progress;
- who wrote what: a count of changes per device;
- the concurrent edits outl merged this session, each with a word diff and an **Undo**;
- **Export as Markdown**, which copies every page and attachment into a new folder next to the workspace.

### The sync dot

| Dot | Meaning |
|---|---|
| Spinner | A sync pass is running. |
| Grey | Checking for devices. |
| Green | A paired device is reachable. |
| Orange | No paired device is reachable, or the phone has no network, or nothing is paired. |

Orange does not mean your notes are at risk.
They are on the phone and sync when a device comes back.
Fixes are in [mobile-troubleshooting.md](mobile-troubleshooting.md#the-sync-dot-is-orange).

### When it syncs

- **App open:** continuously, and every few seconds the phone dials your devices itself, which is what makes it work behind a mobile carrier.
- **App in the background:**
  - iOS suspends the app's network a few seconds after you leave.
    outl finishes the pass in flight, and iOS then grants a few short windows a day.
    Turn on **Background App Refresh** for outl in iOS Settings, or sync only happens while the app is open.
  - Android freezes the process shortly after you leave, and runs a catch-up about every 15 minutes while the process is still alive.
    If Android has reclaimed it, sync resumes when you open outl.
- **Nothing paired:** the app never wakes in the background.

Details and the limits of each OS: [sync.md → Background sync](sync.md#background-sync-on-ios).

---

## Settings

The gear in the header opens one sheet.

| Row | What it does |
|---|---|
| Devices and sync | Pair, see devices, sync now, export. |
| What's what | The first-run walkthrough again. |
| Diagrams | **Pele** draws Mermaid fences as diagrams, **Off** leaves them as highlighted source. Shared with the desktop on the same machine. |
| Lock button order, Reset button order | The keyboard toolbar, see [Edit a block](#edit-a-block). |
| Crash reports | On by default. Turn it off here. A report carries a stack trace, the OS and the CPU, and no note content. This is the only switch on the phone, because there is no `outl` command to run. |

The theme follows the system light and dark appearance.
Every other option is in [`config.md`](config.md).
The phone has no terminal, and its Settings sheet exposes only the rows above, so a setting without a row there is not reachable from the phone today.

---

## Code, diagrams, plugins

**Code blocks.**
A fenced block with a language shows **Run `<language>`** in the long-press menu.
JavaScript, Python and Lua run in-process on the phone.
Rust and Lisp do not run on mobile.
See [client-editing.md → Running code blocks](client-editing.md#running-code-blocks).

**Diagrams.**
A `mermaid` fence renders as a diagram unless you turned **Diagrams** off.

**Plugins.**
Open the four-squares button.
**Browse** lists the official registry and installs with one tap.
**Commands** lists what installed plugins contribute and runs them.
Mobile has no slash menu and no chords, so those two contribution types of a plugin never appear.
Plugins you install by hand, outside the registry, need the CLI.
How to write one: [plugins.md](plugins.md).

---

## Where your notes are

The workspace is a folder in the app's own storage.
Because iOS does not list the app's storage in the Files app, the supported ways to get notes out are:

- sync them to a computer, where they are plain `.md` files you can open anywhere;
- **Export as Markdown** from the Devices sheet, which writes a copy next to the workspace folder.

The folder layout and the file format are the same everywhere: [markdown-format.md](markdown-format.md).

---

## Phone versus desktop

| Thing | Phone | Desktop |
|---|---|---|
| Layout | One pane. Search replaces the sidebar. | Sidebar, tabs, panels. |
| Keys | Touch. A keyboard toolbar instead of chords. | Chords and vim modes. |
| Selecting blocks | Long press, **Select blocks**, tap to stretch. | `v` and `j`/`k`. |
| Slash menu | None. | `/`. |
| Deleted blocks | Not restorable from the phone yet. | Trash. |
| Automatic backups | No. Another device you sync with takes them. | Yes. |
| Open a file from the OS in outl | Not yet. | Yes. |
| About screen with the version | Not yet. See the build in the App Store, TestFlight or Android Settings. | Yes. |
| Quote toggle | Type `> ` at the start of a block. | One gesture. |
| Mark done in one tap | Tap the checkbox through to DONE. | Yes. |

---

## Privacy

- Notes never leave your devices except to another device you paired.
- The relay used when two devices cannot reach each other directly carries encrypted traffic and does not store your notes.
- Crash reports are on by default and contain no note content.
  You can switch them off in Settings.
- Camera access is used only to scan a pairing QR.
- Local network access is used only to find your other devices on the same Wi-Fi.

Full policy: [privacy.md](privacy.md).
