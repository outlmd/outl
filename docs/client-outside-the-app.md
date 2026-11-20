# When outl is not the app in front of you

Three things that happen while the app is closed, in the background, or in another app's hands: a capture gesture, a file another app hands over, and a reminder that has to fire.

Moved out of [`docs/clients.md`](clients.md), which had grown to the point where no edit to it was cheap.
That page keeps the cross-client architecture: the stack, who owns which fact, the locks an open takes, and the two conditions every client has to surface.

## Capturing from outside the app

The cost of a thought is the time between having it and seeing it written down, and on a phone the whole app launch sits in the middle of that.
So every client has a way to add a line to today's journal **without outl being on screen**.

| Client | Gestures |
|---|---|
| iOS | Home Screen and Lock Screen widget, the "Add to today" Shortcut (Siri and the Action button use the same one), the share sheet |
| Android | the share sheet, a home screen widget, a quick settings tile |
| Desktop | `outl capture "your line"`, and whatever you wire it to — a macOS Service or Shortcut, Raycast, Alfred, a hotkey daemon |
| TUI | `outl capture` from any terminal; it lands the next time you start outl here |

**What the words mean.** A widget or a Shortcut says *"Saved, opens in outl"*, never "added".
The line is on that device and not yet in the journal, not yet on your other devices.
That is not hedging — it is the actual state, and RFC 0363 picked it over a reassuring lie.

### Why it waits instead of being written straight away

A widget, a share extension and a Shortcut each run in **their own process**, with their own sandbox and a memory ceiling far below what opening a workspace needs.
None of them can hold the workspace open, and root [`CLAUDE.md`](../CLAUDE.md) invariant 7 forbids a second process appending to the op log behind the app's back.

So each capture is one small file in an inbox, and the app turns it into a block:

- **On launch, on every return to the foreground**, and — on iOS — within about a second when something writes while the app is already open.
- Into the journal for the day the capture was **made**, not the day it was drained. A line captured at 23:58 and drained at 00:10 belongs to the 23:58 day, which means a drain can create a past day's journal page.
- In capture order, and never twice: the block's id *is* the capture's id, so a drain interrupted between writing the op and deleting the file replays onto the same block.
- Ordered against your own typing by **drain** time. A line captured on the lock screen while the app sat in the background appears below whatever you typed after it — the honest order of what the app saw.

The block is an ordinary block from there on: it syncs, `Cmd+Z` undoes it, and it carries its capture time.

**A capture is always one line.**
Share a multi-line selection from a browser, or pipe a paragraph into `outl capture -`, and what lands is those lines joined by spaces, because a capture is one block.
Newlines are collapsed when the capture is **read**, not only when it is written, so a file from an older build or copied in from another device gets the same treatment.

**A capture that cannot be read is reported, not dropped.**
A file the app fails to parse — or one over 1 MiB, which no single line is — stays in the inbox and is counted.
The app says so when it drains, and `outl capture --list` names the folder it is sitting in.
The alternative is a line that silently never appears, which is the failure this feature exists to prevent.

**If you never open the app, the captures wait.** They are on that one device and they are not synced. `outl capture --list` shows what is still waiting in a workspace.

### `outl capture` is not `outl daily append`

`outl daily append --text=…` writes the block into the op log immediately, and is the better command when a second of workspace boot is free.
`outl capture` writes a file and lets the app land it. Two things make that the right trade for a capture gesture: it pays no workspace boot, and it does not mint ops under a second actor when a GUI client is already holding the device's write actor.
The full comparison is in [`cli.md` → Capture](cli.md#capture).

Per-client coverage: `CaptureOutsideTheApp` in [`client-parity.md`](client-parity.md).
Design, the rejected alternatives, and what this makes worse: [RFC 0363](rfcs/0363-ios-quick-capture.md).

## Opening a file from the OS ("Open With → outl")

A file manager can hand outl a `.md` / `.markdown` / `.txt` / `.text` file that lives **outside** the workspace.
outl is not a text editor and never writes back to that file: the gesture means *import a copy of this into my outline*, so what lands is an ordinary page whose content arrived through the op log like every other page.

Where it lands: a page titled **`open-in/<file name>`**.
The slash is a *title* namespace, not a slug — `page::is_valid_slug` rejects `/` because a slug is one path component — so the file projects to `pages/open-in-<name>.md` and `open-in` becomes a real parent page listing everything ever opened this way ([Nested pages](client-backlinks.md#nested-pages-issue-275)).

**Re-opening the same file navigates; it does not import again.**
The page records where it came from in a `page-source::` property, which is **internal and never rendered into the `.md`**: its value is a local absolute path, a `.md` is often in git, and `outl export hugo` copies every property outside its deny-list into published front matter.
Importing twice into one page would duplicate every block, and overwriting would delete whatever the user wrote on the page after the import — so the second open returns the page the first one produced.
That property is also what keeps two *different* files with the same name apart: `~/a/notes.md` and `~/b/notes.md` both want `open-in/notes`, and the second gets `open-in/notes 2` rather than being silently merged into the first.

The content split follows the paste pipeline, not a second rule: a bulleted `.md` lands as an outline, a bullet-free payload lands as one block per non-blank line (see [Copy and paste](client-editing.md#copy-and-paste)).

**The imported page is linked from today's journal.**
A `[[open-in/<name>]]` block lands in the journal for the day the import happened, so the page has a trail: outl is journal-first, and a page reachable only by search is a page you forget you have.
It is a ref, not plain text, so the page itself lists that entry under its backlinks.
The ref names whichever of title or slug actually resolves: a ref is matched by slugified name before an exact title, so a page whose title no longer derives its slug (a stem of only punctuation or emoji, which falls back to `open-in-untitled`) is linked by slug, or the link would open the `open-in` index instead.
Re-opening a file imports nothing and therefore writes no second entry, however many times you open it.

| Client | How a file reaches it |
|---|---|
| Desktop | `fileAssociations` in `tauri.conf.json` puts outl in the OS "Open With" list (`rank: Alternate`, so it never steals another app's default). macOS delivers an Apple Event (`RunEvent::Opened`), Linux / Windows deliver `argv`. Both buffer until the frontend announces it is listening, then emit `open-file://import` — "the process is running" is not the same as "the webview has mounted", and a slow workspace boot is exactly when a user opens a file. Both routes end at the `open_external_file` command |
| Mobile | The command is registered and **nothing hands it a file** — no share-sheet / document-type association is declared yet, so iOS and Android never offer outl in "Open in". Recorded as `Capability::OpenExternalFile` = `Missing` |
| TUI | Not applicable — a file manager has no terminal process to hand a file to |

Policy (namespace, the `source::` rule, the extension / size / UTF-8 refusals) lives in `outl_actions::open_with` and nowhere else; each client only turns an OS gesture into one path string.

## Reminders (`remind::`)

A block can carry a notification rule as a property — `remind:: 3pm every 1h until DONE`.
The user-facing spec is [reminders.md](reminders.md); this section is the **client contract**.

**One owner for the schedule.**
`outl_actions::reminders::next_fire_at(rule, anchor_date, state, quiet, now)` decides when a rule fires.
It is pure and clock-free — `now` is a parameter — and every surface calls it: the TUI overlay, the desktop panel, the mobile sheet, and each OS notification bridge.
**Never re-derive a schedule in TypeScript or Swift.**
A second opinion about "in 3 hours" is drift that reaches the user at 3am, on one device, before it reaches a test.

**The scan reads the tree, not the `.md`.**
`scan_reminders` finds carriers through `Tree::nodes_with_property(REMIND_KEY)`.
Reading the projection instead would have been stale by construction: the GUI clients write `.md` asynchronously (see [Async projection writes](client-sync.md#async-projection-writes-performance)), so a rule authored a moment ago isn't on disk, and the user who just pressed "remind me" would open the list to find nothing.

**What converges and what doesn't.**

| state | converges? | lives in |
|---|---|---|
| the rule, the block's `[[date]]`, TODO/DONE | ✅ | op log |
| snooze | ✅ | `Op::SnoozeRemind` |
| "this device already fired it" | ❌ | `<root>/.outl/reminders-fired.json` — a dotfile, so iCloud drops it and iroh never ships it |
| quiet hours, the enabled flag | ❌ | `~/.config/outl/config.toml` |

Snoozing on the phone must silence the laptop; the phone having buzzed must not stop the laptop from buzzing.
That split is the whole design.

**Delivery is per-client, not shared.**
`outl_tauri_shared::reminder_runtime::take_due` answers *what* came due and updates the device-local fired log; each client wraps it with its own OS call (`tauri-plugin-notification` on both GUI clients today).
A 30s poll drives it — the schedule has minute granularity, so a tighter tick only adds IPC, and the fired log makes a double poll harmless.

**Every client delivers, including the TUI.**
That is why `take_due` and the fired log live in `outl-actions` and not behind the Tauri layer: the TUI can't depend on `outl-tauri-shared`, and putting them there would have made "the client that shipped first" the owner.
The GUI clients wrap `take_due` with `tauri-plugin-notification`; the TUI wraps it with an OSC 9 escape plus a toast on its event-loop tick.
A terminal session still has no background presence, so a reminder due with the TUI closed is lost to that client.

**Presentation is shared.**
`@outl/shared`'s `formatNextFire` ("in 3h", "tomorrow 09:00") and `groupReminders` (Today / Tomorrow / This week / Later / Done) are used by both GUI clients — two implementations drift on the edge cases (exactly 60 minutes, midnight rollover) long before anyone notices.

**App-closed delivery is not covered yet.**
See [reminders.md → Background delivery](reminders.md#background-delivery--what-ships-today) for what each OS still needs.
