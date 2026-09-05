# Reminders (`remind::`)

A `[[2026-12-12]]` in a block gets you a backlink on that day's journal.
That's great for **recall** and useless for **interruption** — you still have to open the app on the right day.

`remind::` is the opt-in that turns a block into something the OS will tell you about.

```markdown
- TODO #fup [[@joão]] about project abc [[2026-12-12]]
  remind:: 3pm every 1h until DONE
```

Reads as English: *remind me at 3pm, every hour, until it's done.*

## Explicit opt-in, always

A `[[date]]` alone **never** schedules a notification.
Plenty of people use `[[date]]` purely for backlinking, and notifications are noisy — the moment a link becomes a buzz, the linking stops.
No `remind::`, no interruption.

| block has a date | has `remind::` | what happens |
|---|---|---|
| `[[2026-12-12]]` | no | nothing — backlink only |
| `[[2026-12-12]]` | `remind:: 10am` | one fire on 2026-12-12 at 10:00 |
| `[[2026-12-12]]` | `remind:: 10am every 1h` | repeats until DONE |
| no date | `remind:: 10am` | fires **today** at 10:00 |
| no date | no | nothing |

## Syntax

```ebnf
remind     ::= TIME ("every" INTERVAL)? ("until" STOP)? ("max" N)?

TIME       ::= "now" | "10am" | "3pm" | "15:00" | "1:30pm"
INTERVAL   ::= N ("min" | "h" | "d")          # 30min, 1h, 2d
STOP       ::= "DONE" | TIME | ISO_DATE       # until DONE, until 6pm, until 2026-12-20
N          ::= 1..999
```

Case-insensitive — `3PM EVERY 1H UNTIL DONE` parses the same as the lowercase form.

| written | means |
|---|---|
| `remind:: 10am` | one fire at 10:00 |
| `remind:: 10am every 1h` | from 10:00, hourly, until DONE |
| `remind:: 10am every 1h until 6pm` | stops at 18:00 on the anchor day |
| `remind:: 10am every 1h max 5` | at most 5 fires |
| `remind:: 3pm every 30min until DONE` | the typical "nag me" |
| `remind:: now every 15min until DONE` | start immediately, loop |

**`until DONE` is the implicit default.**
Writing no `until` clause means the same thing.

**A 24-hour time needs the colon.**
`15:00` works; a bare `15` is too ambiguous to guess between "3pm" and "the 15th", so it's rejected.

### Caps

| cap | value | what happens past it |
|---|---|---|
| `every` floor | 1 minute | rejected (`every 30s` is never what you meant, and silently rewriting it would hide the typo) |
| `max` ceiling | 10 fires | clamped down, with a warning |
| `until TIME` | must be after the anchor | the clause is dropped, the rest of the rule still schedules |

### When a rule doesn't parse

Nothing is lost.
The property stays on disk verbatim, the block is untouched, and the rule simply doesn't schedule — the parse banner shows which line to fix.
This is the same permissive recovery the rest of the outl dialect uses (see [Markdown dialect](markdown-format.md)).

The warnings you can see: `remind_missing_anchor`, `remind_invalid_time`, `remind_invalid_interval`, `remind_invalid_stop`, `remind_max_clamped`.
`outl doctor` lists them per file.

## What fires, and when

1. The first fire lands on the anchor — the rule's `TIME` on the block's `[[date]]`, or today when it carries none.
2. With `every`, the next fire is one interval after the last one.
3. A rule whose anchor is **already past** when the block is written fires immediately, then follows `every`.

Two behaviours worth knowing:

- **A device that was asleep owes you one banner, not a backlog.**
  Close the laptop at 10:00 on an `every 1h` rule and open it at 18:00: you get one reminder, not eight.
- **Two dates in one block schedule twice.**
  `[[2026-12-12]] and [[2026-12-15]]` fires on both — you wrote both dates on purpose.

### What cancels a reminder

| you do | effect |
|---|---|
| flip `TODO` → `DONE` | every pending fire is cancelled, including on a rule with an explicit `until 6pm` |
| delete the block | cancelled |
| edit the `remind::` value | rescheduled from scratch |
| edit the block's `[[date]]` | rescheduled |

### Snooze

Snoozing writes an `Op::SnoozeRemind` into the op log, so **it converges**: silencing a nag on your phone silences the same block on your laptop.
Presets are 1 hour, tomorrow, and next week; the desktop panel and the mobile sheet also offer "Resume" to clear it early.

### Quiet hours

Device-local. Delivery is on by default, quiet hours are not set:

```toml
[reminders]
enabled = true              # default
quiet_hours = "22:00-07:00" # unset by default
```

A fire landing inside the window is **pushed to the window's end**, never dropped — you asked for it, you get it, just not at 3am.
A window that wraps midnight is the normal case and is handled; so is a same-day window like `13:00-14:00`.

One exception: a fire pushed past its own `until` is genuinely over.
`remind:: 9pm every 1h until 11pm` with quiet hours starting at 22:00 stops at 21:00 — waking you at 07:00 for an 11pm deadline is not what the rule said.

`enabled = false` turns this device silent: the rules still parse and still show up in the reminders list, they just never interrupt.
It defaults to `true` because writing `remind::` on a block is already the opt-in, and a device that never gets a rule never fires.
The OS asks for notification permission on the first actual fire, not when you flip a switch.

## Where you see them

| client | list | author | delivery |
|---|---|---|---|
| **TUI** | `g n` overlay | `g r`, `g R` | OSC 9 notification + a toast, while the TUI is open |
| **Desktop** | `Cmd/Ctrl+Shift+R` panel | `Cmd+R`, `g r` / `g R` in Normal | OS notification while the app runs |
| **Mobile** | bell icon in the header | long-press a block → *Remind me…* | iOS notification while the app runs |

Editing a rule after you write it: click (desktop) or tap (mobile) the `⏰` chip under the block, or `:prop remind <rule>` in the TUI.
An empty value clears the rule, which is how you stop a block nagging without deleting it.
Every `key:: value` a block carries renders as a chip and edits the same way — `remind::` isn't special-cased.

Turning delivery on lives in Settings on the desktop, and in the Reminders sheet itself on mobile.
Mobile has no settings screen, and `config.toml` sits inside the iOS sandbox, so a switch anywhere else would be unreachable.

Chords are in the shared catalog, so they can't drift — see [Shortcuts](shortcuts.md).

> **Why `g n` and not `Ctrl+R` in the TUI?**
> `Ctrl+R` is already Redo, and a terminal can't distinguish `Ctrl+R` from `Ctrl+Shift+R`.
> The `g` family (`g j`, `g x`, `g d`) is the honest home for it there; the desktop still takes `Cmd/Ctrl+Shift+R`.

## Background delivery — what ships today

**Today: reminders fire whenever the app is running**, foreground or backgrounded, on macOS / Linux / Windows / iOS — and in the TUI, which fires an OSC 9 desktop notification plus a toast on its event loop.

**On mobile the banner is actionable.**
It carries **Snooze 1h** and **Done**, both of which resolve the reminder without opening the app, and tapping the banner itself opens the page scrolled to the block that buzzed.
The TUI and the desktop deliver the same reminder as a banner you can only read: neither delivery channel carries a callback (OSC 9 is one string to the terminal; `tauri-plugin-notification`'s actions and `actionPerformed` event are `#[cfg(mobile)]`, and its desktop `show()` drops the `notify-rust` handle).
On those two, the reminders surface (`Ctrl+R` / `Cmd+Ctrl+Shift+R`) is where you snooze or tick off what came due.
That split is declared once, for all three clients, in `outl_shortcuts::capability_support` (`Capability::ReminderNotificationActions`) and shows up in [`docs/client-parity.md`](client-parity.md).
(OSC 9 is honoured by iTerm2, kitty, WezTerm and ghostty; terminals that ignore it still show the toast. Same best-effort contract as the OSC 52 the yank path uses.)
The GUI clients poll every 30 seconds, the TUI every tick; the backend keeps a device-local "already fired" log (`<root>/.outl/reminders-fired.json`, 7-day TTL) so polling twice never double-buzzes and losing the file costs you at most one duplicate.
A reminder that comes due with the TUI **closed** is lost to that client, which is the honest limit of a terminal session.

**Not yet: delivery with the app fully closed.**
That needs per-OS scheduling registered ahead of time, and each platform wants something different:

- **iOS** — `UNCalendarNotificationTrigger` requests registered in advance (the system caps pending requests at 64), re-filled from a `BGAppRefreshTask`.
- **macOS** — a small launch agent on a `StartCalendarInterval`, rather than keeping the app resident in the tray.
- **Windows** — `ScheduledToastNotification`, or a Task Scheduler helper.
- **Linux** — a systemd user timer firing a helper binary.

All four are tracked as follow-ups to [issue #63](https://github.com/outlmd/outl/issues/63).
Until they land, a reminder for a day you never open outl will not reach you — worth knowing before you rely on it for something that matters.

## What converges and what doesn't

This is the [invariant #7](../CLAUDE.md) line, drawn explicitly:

| state | converges? | lives in |
|---|---|---|
| the `remind::` rule, the block's `[[date]]` | ✅ | block text + properties → op log |
| `TODO` / `DONE` | ✅ | text prefix → op log |
| snooze | ✅ | `Op::SnoozeRemind` |
| "this device already fired it" | ❌ | `<root>/.outl/reminders-fired.json`, local, 7-day TTL |
| quiet hours, the enabled flag | ❌ | `~/.config/outl/config.toml`, per device |

The split is the whole point: snoozing on one device must silence every device, but one device having buzzed must not stop another from buzzing.

## Client wiring (desktop panel, mobile sheet)

Which component lists the reminders on each GUI client, which command authors a rule, and where delivery is polled from.
The schedule math is never here — see [For contributors](#for-contributors) below.

### Desktop (`outl-desktop`)

`<RemindersPanel />` (`Cmd/Ctrl+Shift+R`, or `g n` in Normal) lists every block carrying a `remind::`, grouped Today / Tomorrow / This week / Later / Done.
`Cmd+R` (and `g r` / `g R` in Normal) authors a rule on the selected block via `set_block_remind`.
`Ctrl+R` deliberately stays **Redo** on Linux / Windows, which is why authoring is `Cmd+R` only.

Grouping labels and the "in 3h" column come from `@outl/shared` (`groupReminders` / `formatNextFire`) — the same functions the mobile sheet uses — and the instants behind them come from `outl_actions::reminders` in Rust.
**Nothing about when a reminder fires is computed in the frontend.**

Delivery is a 30s `setInterval` in `<AppShell />` calling `deliver_due_reminders`, which turns the shared "what's due" answer into an OS banner via `tauri-plugin-notification`.
The Rust side keeps the device-local fired log, so polling twice never double-buzzes and a laptop that was asleep owes one banner, not a backlog.
`[reminders] enabled` (Settings modal) defaults to **on**: `remind::` on a block is already the opt-in, and a device with no rules never fires, so defaulting off only bought the user a rule that silently did nothing.
macOS asks for permission on the first actual fire.

**App-closed delivery is not covered yet**: a launch agent on a `StartCalendarInterval` is the follow-up (see [Background delivery](#background-delivery--what-ships-today), above).

### Mobile (`outl-mobile`)

The header bell opens `<RemindersSheet />` — every block with a `remind::`, grouped Today / Tomorrow / This week / Later / Done, with 1h / Tomorrow / Next week snooze chips per row.
Long-pressing a block offers **Remind me…**, which prompts for the rule in its own syntax and writes it via `set_block_remind`.
A prompt rather than a native time picker on purpose: the rule language is richer than a clock (`3pm every 1h until DONE`), and a picker that can only express the anchor would quietly hide the repeat.
The picker is a follow-up, not a substitute.

Grouping + the "in 3h" column come from `@outl/shared` (`groupReminders` / `formatNextFire`), shared byte-for-byte with the desktop panel; the instants come from `outl_actions::reminders` in Rust.

Delivery is a 30s `setInterval` in `Journal.tsx` calling `deliver_due_reminders` (`tauri-plugin-notification` → `UNUserNotificationCenter`).
It fires whenever the app is running, foreground or backgrounded.

Every banner is stamped with the `outl.reminder` category and carries the block's `blockId` / `pageSlug` as extras.
The category and its buttons come from the `reminder_action_catalog` command, so the ids the OS is told about are the same constants `deliver_due_reminders` stamps — a rename cannot turn a button into a no-op.
Registration happens at boot, not on first delivery: iOS resolves a banner's category at delivery time, and one naming an unregistered category still shows, just with no buttons and no error.

What each does: **Snooze 1h** writes `Op::SnoozeRemind` (so it silences every device), **Done** resolves the block's own page first and marks it `DONE` (the banner can arrive while you are looking at any page), and a plain tap opens the page and scrolls the block into view rather than zooming into it.

**Both device-local settings live in the sheet, not in a settings screen** — mobile has none, and `config.toml` sits inside the iOS sandbox, so they'd otherwise be unreachable from the device.
Delivery is a switch; quiet hours are two native `<input type="time">` pickers rather than the desktop's text field, because typing `22:00-07:00` on a phone means switching keyboard layouts twice.
Both write through `set_reminder_settings`, which replaces the pair — so every call sends **both** values, or flipping the switch would wipe a configured window.
The UI only moves after the write returns, so a failed save can't leave it lying.
Split / join for the wire string is `lib/quiet-hours.ts` (unit-tested: a half-filled window saves as empty, since `"22:00-"` is just something the backend drops).

Rows carry a **Done** button, matching the desktop panel: it resolves the block's own page id first (the sheet lists the whole workspace, so the open page is usually a different one) and only applies the refreshed view when the user is looking at that page.

**App-closed delivery is not covered yet.**
That needs `UNCalendarNotificationTrigger` requests registered ahead of time (the system caps pending requests at 64) and re-filled from a `BGAppRefreshTask` — the same shape as the existing `bg_sync.rs` work.
See [Background delivery](#background-delivery--what-ships-today), above.

## For contributors

`outl_actions::reminders::next_fire_at` is the **single owner** of the schedule math — pure, clock-free, takes `now` as a parameter.
Every surface (the TUI overlay, the desktop panel, the mobile sheet, each OS bridge) calls it.
A second opinion in TypeScript or Swift about when a reminder fires is exactly the drift that reaches the user before it reaches a test.

The pieces:

| crate | owns |
|---|---|
| `outl-md` | `remind::` syntax → `RemindRule`, plus the `ParseWarningKind` variants |
| `outl-core` | `Op::SnoozeRemind` and the tree's snooze table |
| `outl-actions` | `next_fire_at` (pure) + `scan_reminders` + `snooze` + `take_due` / the fired log. Every client delivers, so none of this sits behind a client layer |
| `outl-config` | `[reminders]`, device-local |
| `outl-tauri-shared` | the DTOs, the commands, and the fired-log runtime both GUI clients share |
