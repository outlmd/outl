# Desktop settings

Open settings with `Cmd/Ctrl+,` or the gear in the top-right capsule.
The modal holds every setting the desktop app exposes, in this order.

Settings are stored in `~/.config/outl/config.toml`, the same file the TUI reads, so a theme you pick here is the theme the TUI starts with next time.
On Windows the file sits in the platform's config directory.
The whole file, including keys the modal does not show, is in [Configuration](config.md).

**Save and Cancel.**
Most rows are a draft until you press **Save**.
Theme and Diagrams apply on screen as you pick them, so you can judge them, and **Cancel** (or a click outside the modal) puts the previous values back.
Two things do not wait for Save: the crash-reports switch, and the backlinks direction button on a page.

**If a configuration manager owns the file.**
When Nix or home-manager manages `config.toml`, a notice at the top says so and Save is disabled.
A theme still previews live, but it does not persist.
Change the value in your Nix config (`programs.outl.settings`) and rebuild: [Nix](nix.md#declarative-config).

## Vim mode

A checkbox that is stored in the config and does nothing on the desktop.
Normal, Insert and Visual chords are live whatever it says.
Whether you use them is up to you: select blocks with `j` and `k` and edit with `i`, or click and type.

The TUI is modal by definition and also ignores it.

## Theme

Three controls that work as a pair:

| Control | Choices | What it does |
|---|---|---|
| Mode | Auto, Light, Dark | Auto follows the operating system's appearance and switches when it does |
| Light | Any theme | The theme used when the light side is on screen |
| Dark | Any theme | The theme used when the dark side is on screen |

The label **active** marks the side currently showing for the chosen mode.

The built-in list is in [Theming](theming.md).
Your own go in `~/.config/outl/themes/<name>.toml` and show up in both lists: [Theming](theming.md#your-own-theme).

On launch the window background matches your theme before the page draws.
With mode Auto it starts on the dark side and corrects itself one frame later if your system is light.

## Diagrams

`Pele` draws `mermaid` fences as diagrams.
`Off` leaves them as highlighted source.
It applies as you pick it.

## Font size

Outline text size in pixels, from 10 to 32.
The default is 15.
A value that is not a number goes back to 15.

## Workspace

The folder currently open, and **Switch...** to pick another.
The modal closes when the switch happens, not when you cancel the folder dialog.
On macOS **File, Switch Workspace...** does the same.

Switching does not change your device's actor id.

## Reminder notifications

Whether this device turns `remind::` rules into system notifications.
It is on by default.
A workspace with no reminders never prompts for permission: the OS asks the first time one fires.
Turn it off and reminders stay listed without interrupting you.

## Quiet hours

A window such as `22:00-07:00`.
A reminder that falls inside waits until the window ends and is never dropped.
Leave it empty for none.
Both reminder settings are per device and do not sync, so your phone and laptop can keep different hours: [Reminders](reminders.md).

## Sync transport

| Choice | Meaning |
|---|---|
| `iroh` (default) | Device to device over an encrypted connection. Pair devices in the Sync section below |
| `file` | No direct connection. outl relies on a folder that something else syncs, such as iCloud Drive or Syncthing |

The change takes effect after you restart the app.
With `file`, pairing is refused, because binding a connection would override the choice you made.
Background and trade-offs: [Sync](sync.md).

## Sync (devices)

The section under the transport shows the same dot as the top bar, a **Refresh** button, and your paired devices.
Refresh forces a sync, then reloads what is on screen.
If it cannot sync, it says why: for example, because another outl process on this machine (usually `outl mcp serve`) holds the device's connection.
Removing a device here removes it from this workspace only.

Below it, per-device activity shows how many changes each device has contributed, labelled "This device" or a short device id, and a one-click Markdown export into a new folder next to the workspace.

## Privacy

**Crash reports.**
When outl crashes it sends the error, where in the code it happened, your OS and the app version.
It never sends page titles, block text or file paths, and there is no account to attach it to.
The switch saves the moment you flip it.
If outl cannot read or save the flag, the row says so and the switch stays disabled.
You can also turn it off from a terminal with `outl telemetry off`.
A one-time notice about this appears in the app on first launch.

## Settings that are not in the modal

| Setting | Where |
|---|---|
| Backlinks direction (newest or oldest first) | The direction button in the "Linked from" header on any page |
| Sidebar | `Cmd/Ctrl+Shift+E`. Starts hidden on every launch |
| Focus mode | `Cmd/Ctrl+Shift+F`. Remembered on this device |
| Time zone, snapshot policy, asset size cap, backups, relay URL | `config.toml`, see [Configuration](config.md) |
| Plugins | The plugin browser in the top bar |
| Recent pages | Remembered per device, not synced |

Keys the modal does not show are preserved when you press Save.
