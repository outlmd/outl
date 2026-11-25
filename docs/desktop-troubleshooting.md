# Desktop troubleshooting

Find the symptom, read what it means, do the next step.
Every message quoted here is the text the app shows.

If nothing here matches, run `outl doctor` against the same workspace: it checks the folder from outside the app ([doctor](doctor.md)).
Then open an issue from **Help, Report a Problem** (macOS) or <https://github.com/outlmd/outl/issues>.

## The app will not start

**Windows: "outl needs the Microsoft Edge WebView2 Runtime, and this PC does not have it."**
outl draws its window with Microsoft's WebView2.
Install the Evergreen runtime from <https://developer.microsoft.com/en-us/microsoft-edge/webview2/>, then open outl again.
Nothing is lost: outl exited before it opened your workspace.

**Windows: "outl could not open its window after N attempts."**
The runtime is installed but did not hand outl a window, which usually means the machine was busy at the time.
Close other apps and open outl again.
If it keeps happening, reinstall the WebView2 Runtime from the same page.
The message prints the error Windows reported: include it in an issue.

**A blank window on launch.**
If your system theme is light and your theme preference is Auto, the window may flash dark for one frame before it corrects.
That is expected: the window cannot read the OS appearance before the page draws.
A window that stays blank is not: open an issue.

**Launching again does nothing.**
Only one outl runs.
A second launch brings the first window forward.
Look behind other windows, or on another desktop.

## The first screen is slow, or read-only

A large workspace needs a moment to load.
outl shows today's journal immediately, marked as still loading, and you cannot edit it until the workspace opens.
A few seconds on a workspace of hundreds of thousands of changes is normal.
Sync, the file watcher and plugins start after the first screen is drawn, so the first seconds are not spent on them.
The phases and budgets: [Boot performance](boot-performance.md).

## Sync

**The dot is orange.**
No paired device is reachable at the moment.
The usual causes: the other device is asleep or closed, it is offline, or the two of you have not paired.
Open Settings, Sync, and look at the devices.
Click **Refresh** to try now.
You do not need an orange dot to be fixed to keep working: your changes are saved here and ship when the device is back.

**I have no paired device and the dot is orange.**
That is the correct state.
Pair one from Settings, Sync, or ignore it.

**Refresh says another outl process holds the connection.**
A device has one network endpoint, and one process owns it.
`outl mcp serve` (started by Claude Desktop or another MCP client at login) often wins it before the app opens.
While it holds the endpoint, the window cannot sync directly: it still syncs through the workspace folder, but the dot stays orange.
Quit the other process and click Refresh.
The window asks for the endpoint again.

**Pairing is refused.**
If **Sync transport** is `file`, pairing is off by design.
Switch it to `iroh` in Settings and restart the app.

**Sync seems stuck after I changed the transport.**
The transport changes at the next launch.
Quit outl fully and open it again.

Pairing and fixing sync: [Sync your devices](sync-devices.md).
How sync works and what it moves: [Sync](sync.md), [What reaches disk and the other devices](client-sync.md).

## A page is not syncing

**"This page isn't syncing."**
A banner above the outline.
Some lines in the page's `.md` file were never recorded by outl, so they exist only on this device, and until that is fixed changes from your other devices do not show up on this page either.
The banner shows one of the affected lines and the file path.

You can keep editing: outl will not overwrite those lines, and your changes are saved.
To fix it, run this in the workspace folder:

```sh
outl reconcile --ahead-of-log
```

The banner clears the next time you open the page.
Why it happens (almost always: you edited the `.md` file in another editor while outl was closed): [Surfacing a page that stopped syncing](clients.md#surfacing-a-page-that-stopped-syncing).

## Things on screen

**A block I just created cannot be copied as a reference: "no ref handle yet — save and retry."**
The block has not been written to disk yet, and a reference needs a stable handle that appears when it is.
Wait a second and copy again.

**A restore in the trash says it cannot come back.**
The row gives the reason in place.
The two common ones: a deleted page, whose name is usually taken by now, and a block whose parent was deleted after it.
When the row looked restorable and then refuses, another device changed the tree in between: the panel reloads itself and the row shows the new state.

**I cannot rename or delete a page.**
It is a journal.
A journal's name is its date.

**I renamed a page and the old name came back.**
Something opened the old name: a stale `[[link]]`, a Recent row, a script.
Opening the old name renames the page back, and the links outl rewrote then point at nothing.
Rename it again, and fix or remove whatever still names the old page.
The cause is in [Reopening the old name undoes the rename](client-page-lifecycle.md#reopening-the-old-name-undoes-the-rename).

**I renamed a page to a name that exists.**
outl asks if you want to merge instead.
Cancel and nothing changes.
**A merge cannot be undone.**

**A backlink row will not take my edit.**
An edit on a block that lives on another page refuses in two cases: the block cannot be found by its exact identity, or its source page has stopped syncing (see the previous entry).
It refuses instead of writing the text somewhere else, and the editor stays open on what you typed so you can copy it.
Open the source page from the group title and check.
The rules are in [Editing a backlink where it is cited](client-backlinks.md#editing-a-backlink-where-it-is-cited).

**A mermaid diagram shows as code.**
Diagrams are off in Settings, or the diagram source does not parse.
Check Settings, Diagrams.

**The task list ends at 200.**
It lists the first 200 and the header shows the real count.
Use a [query block](query.md) to see the rest.

**Vim chords such as `x` or `D` show a message.**
The desktop selects blocks, not characters, so character-level commands are not available in Normal mode.
Press `i` and edit the text.
The list is in [What the desktop does not do](desktop.md#what-the-desktop-does-not-do).

## Capture and files

**`Cmd/Ctrl+Shift+Space` does nothing from other apps.**
Another application already owns that chord, so the operating system refused outl's registration.
outl starts anyway.
The chord works inside outl, and the command palette and `outl://capture` open the same box.
Free the chord in the other app, then restart outl.

**A capture did not appear.**
The box keeps your text and shows why when a write fails.
For `outl capture` from a terminal, the line lands the next time the app opens or comes to the front, and `outl capture --list` shows lines still waiting.
A file that cannot be read is counted and reported, not dropped: [When outl is not the app in front of you](client-outside-the-app.md).

**Open With imported a file but I edited the original and nothing changed.**
Open With imports a copy once.
Opening the same file again goes to the existing page and does not re-import.
Edit the page in outl.

**A link to an attached file does not open.**
The file lives in the workspace's `assets/` folder.
If a synced device has not received it yet, the link has nothing to open.

## Reminders

**No notification arrived.**
Check, in order:

1. outl is running.
   Reminders fire only while the app is open.
2. Settings, Reminder notifications is on.
3. The time is not inside Quiet hours.
   A reminder inside the window waits for its end.
4. The OS allows notifications from outl.
   The system asks the first time one fires.
5. The block carries a `remind::` line.
   A `[[date]]` alone never schedules anything.

The reminder list (`Cmd/Ctrl+Shift+R`) shows what is scheduled and when it next fires.

## Settings

**Save is disabled and a notice says the settings are managed externally.**
Nix or home-manager owns `config.toml`.
Edit the value in your Nix configuration and rebuild: [Nix](nix.md#declarative-config).

**My settings reset.**
They live in `~/.config/outl/config.toml`.
An empty or missing file loads the defaults.
Check that the file is there and not zero bytes, and see [Configuration](config.md).

## Reporting a problem

Say what you pressed, what you expected, and what happened.
Include your OS, the outl version (on macOS, the About window), and the text of any message.
A crash report is sent only if crash reporting is on, and it never contains your notes: [Desktop settings, Privacy](desktop-settings.md#privacy).
