# TUI overlays

The popups that take over the keyboard: property editor, reminders, sync panel, trash and the error popup.
The quick switcher, search, slash menu, command line and template picker are covered with the commands they belong to in the [TUI manual](tui.md) and [TUI commands](tui-commands.md).

## How overlays behave
An overlay takes the keyboard until you close it.
`Esc` always closes.
`Ctrl+C` still quits.

## Property editor (`g p`)

Lists the `key:: value` pairs of the selected block and edits them without you remembering a key.
`:prop` is faster when you already know the key.

| Key | What it does |
|---|---|
| `j` / `k` or `↑` / `↓` | Move between properties |
| `Enter` | Edit the highlighted value. Renaming a key is a delete plus an add |
| `o` | Add a property: key first, `Enter` to move to the value, `Enter` to save |
| `Tab` in the key field | Complete from keys this workspace already uses, most used first. Press again to cycle |
| `[[` or `#` in the value field | The same page and tag pickers as Insert. `Tab` or `Enter` accepts |
| `d d` | Delete the highlighted property |
| `p` | Switch between this block's properties and the page's (`icon::`, `type::`, ...) |
| `Esc` or `q` | Close |

It writes through the same path as `:prop`, so a change appears in the outline immediately.
`page-slug` and `page-kind` are refused.
Editing `title` renames the page and respells its mentions.

## Reminders (`g n`)

Every block with a `remind::` rule, soonest first, with the next fire time, the block text, the rule and a snooze marker.
Written `g r` on a block, edited with `:prop remind <rule>`.
Rule grammar and what fires where: [reminders.md](reminders.md).

| Key | What it does |
|---|---|
| `j` / `k` or `↑` / `↓` | Move |
| `Enter` | Open the page and block |
| `s` | Snooze 1 hour |
| `t` | Snooze to tomorrow 9am |
| `w` | Snooze to next week 9am |
| `Esc` or `q` | Close |

A snooze is a real op, so your phone goes quiet too.
There is deliberately no "done" key here: ticking a task from a list that hides it is how you check off the wrong row.
Open it with `Enter` and use `g D`.
With no rules the list reads `No reminders. Press g r on a block to add one`.

**Delivery.**
While the TUI is open it checks about every 20 seconds and, for a rule that came due, sends a desktop notification through the terminal (OSC 9) plus a toast.
The toast always works, the desktop banner only in terminals that support OSC 9.
Nothing fires while the TUI is closed.
Reminders are off for a device with `[reminders] enabled = false`.
Delivery is held while you are typing in Insert, and arrives when you leave it.

## Sync panel (`:sync`)

What each paired device is still waiting for, and the concurrent edits that were merged on this device.
The same data the desktop Settings and the phone Devices sheet show.

| Key | What it does |
|---|---|
| `j` / `k` or `↑` / `↓` | Move between merged edits |
| `Enter` | Open the page that holds the merged block |
| `u` | Undo the merge: put your text from before it back. It is a normal edit, so it syncs |
| `Esc` or `q` | Close |

Both lists live in `.outl/state/` and survive a restart.

## Trash (`/trash`, `:trash`)

Every block you deleted in this workspace, and the way back.
A delete is a move, never a removal, so this reaches a deletion from any device and any day.
`u` only walks the current session.

| Key | What it does |
|---|---|
| `j` / `k` or `↑` / `↓` | Move between deletions |
| `r` | Restore under the block it was deleted from |
| `Enter` | On a deleted block, open the page it came from, without restoring |
| `Esc` or `q` | Close |

A deleted subtree is one row and says how many blocks come back with it.
A row that cannot be restored says why instead of offering `r`: a deleted page (its name is usually taken by now), or a block whose parent went to the trash after it did.
`Enter` still works on a refused block row, since "where did my content go" has an answer even when "bring it back" does not.
`Enter` on a deleted page refuses on purpose.
Opening a page whose file went with the delete would revive it as a new page, and turn one trash row into one per child.
To revive a page deliberately, close the overlay and open it by name.
Another device can trash the landing parent after the list was read, so `r` can refuse a row that looked fine.
The sentence appears on the overlay's own status line and the list re-reads itself.

## Error popup

A failure too long for the footer (a compile error from a code block, a multi-line trap) opens a modal with a title and the full text.
Any key dismisses it, `Ctrl+C` quits.

