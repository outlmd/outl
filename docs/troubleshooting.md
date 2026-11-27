# Troubleshooting

Find the symptom, read what it means, do the next step.
This page covers the first hour and the questions that come up most.
Each client also has a deeper page for its own symptoms, linked at the bottom.

Your notes are not at risk in anything below.
A delete in outl is a move to the trash, never a physical removal, and the `.md` files on disk are plain text.

## Start here

Two commands answer most questions before you read further.

```bash
outl --version      # the build you are running; put it in any bug report
outl doctor         # checks the op log, every page and every sidecar; changes nothing
```

`outl doctor` ends with `integrity OK` or a list of what is wrong and what `--repair` would do about it.
It never writes anything unless you pass `--repair` ([outl doctor](doctor.md)).

The TUI writes a log you can `tail -f` while you reproduce a problem: `<workspace>/.outl/tui.log`.

## Install

**`brew install outl-beta` says the formula is unknown, or `brew tap` fails.**
You need the full tap URL once, then a trust step:

```bash
brew tap outlmd/outl https://github.com/outlmd/outl
brew trust outlmd/outl
```

If you were on the old `avelino/outl` tap, run `brew untap avelino/outl` first.
A tap name does not follow GitHub's redirect after the move to the `outlmd` organisation.

**`brew install` pulls an old version after a new release.**
Run `brew update` first.
Homebrew caches tap state ([Homebrew tap → Troubleshooting](homebrew.md#troubleshooting)).

**macOS says `"outl.app" Not Opened` or "could not verify".**
The desktop app is unsigned for now.
Right-click → Open no longer clears this on macOS 15.
Run:

```bash
xattr -dr com.apple.quarantine /Applications/outl.app
```

or use **System Settings → Privacy & Security → Open Anyway** ([steps](homebrew.md#first-launch-gatekeeper-warning)).

**Windows shows "Windows protected your PC".**
The installer is unsigned for now.
Click **More info → Run anyway**.
If outl then shows a message naming the Evergreen WebView2 runtime and exits, install that runtime from Microsoft and start outl again.
The installer only downloads it when it runs, and it needs a network to do so.

**`scoop install outl/outl-beta` says the bucket is unknown.**
Add the bucket with its URL: `scoop bucket add outl https://github.com/outlmd/outl`.
More in [Scoop bucket → Troubleshooting](scoop.md#troubleshooting).

**The Linux AppImage or `.deb` will not start, and `ldd --version` shows glibc older than 2.35.**
The builds need Ubuntu 22.04, Debian 12, Fedora 36 or newer.
On an older distro, [build from source](getting-started.md#from-source).

**Android: installing a new APK fails with `INSTALL_FAILED_UPDATE_INCOMPATIBLE`.**
Every release is signed with a fresh key, so Android refuses to update in place.
Uninstall the old build and install the new one.
The phone's local copy goes with the app; pair it again and whatever had synced to another device comes back ([#171](https://github.com/outlmd/outl/issues/171)).

## Starting outl

**`Error: no outl workspace at <path> — run outl init <path> first`.**
`outl` looks for `.outl/config.toml` in the folder you pass with `--workspace` (`-w`), or in the current folder if you pass nothing.
Either `cd` into your workspace, pass `--workspace ~/notes`, or create one with `outl init ~/notes`.
The same code, `NO_WORKSPACE`, is what scripts see with `--json`.

**`Error: outl-tui requires an interactive terminal (stdout is not a TTY)`.**
The TUI draws a screen, so it cannot run in a pipe or a CI step.
Use CLI subcommands there (`outl page list`, `outl search`, `outl daily append`; see [CLI](cli.md)).

**Colours look wrong, or washed out.**
A terminal that does not advertise 24-bit colour gets the plain ANSI theme on purpose, because that renders correctly everywhere.
Set `COLORTERM=truecolor` if your terminal supports it, or pick a theme explicitly with `outl --theme dracula`.
An explicit theme is never second-guessed.
`outl theme list` prints the eleven built-in presets.
See [Theming](theming.md).

**I run outl from a source build and it behaves like a different device.**
The repo's `.cargo/config.toml` sets `$OUTL_DEVICE_DIR`, which makes every binary cargo launches a separate device with its own identity.
Outl logs a warning naming the directory.
Run `OUTL_DEVICE_DIR= cargo run --release -p outl-desktop` to use the machine's real identity ([details](sync.md#troubleshooting-sync)).

## Writing and editing

**`Enter` on a link starts editing the block instead of opening the link.**
In the TUI the cursor has to be *on* the link.
Press `w` or `l` until it is, then `Enter`.
Only `[[page]]`, `#tag`, journal dates and `((block))` references open this way; a plain `[text](url)` link opens with `g x`.

**I typed `[[Some Page]]` and no file appeared.**
That is expected.
A page exists once you open it, so a typo does not create a file.
Move onto the link and press `Enter` (TUI), or click it (desktop, phone).

**`Ctrl+Enter` does nothing in tmux or macOS Terminal.**
Those terminals send a plain `Enter` for it.
Use `Ctrl+T`, which does the same thing: cycle a block's task state.

**A single `q` does not quit.**
It arms the quit chord and the status line says `press q again to quit`.
Press `q q`, `Z Z` or `Ctrl+C`.

**Copying a block does not reach my system clipboard over SSH or inside tmux.**
The TUI tries the OS clipboard, then an OSC 52 escape sequence.
Your terminal has to allow OSC 52, and some disable it by default.
The status line reads `yanked N block(s) (clipboard unavailable)` when both paths fail ([TUI manual](tui.md#clipboard)).

**I edited a `.md` in another editor and the change is not on screen.**
Press `Ctrl+L` in the TUI to re-read the workspace.
The desktop and the phone pick it up on their own.
outl matches each block back to its sidecar entry and records the difference as ops, so a clean `.md` edit is safe.
Blocks it cannot match are written to `.outl/orphans.log` and `outl doctor` lists them.

**A banner says a page has stopped syncing.**
The page's `.md` holds lines that exist in no op, usually from an edit made while the sidecar was missing.
outl refuses to overwrite those lines, so the page is frozen until you record them.
The banner names the command: run `outl reconcile --ahead-of-log` in the workspace, then reopen the page.
Why it refuses rather than overwrites: [Clients → Surfacing a page that stopped syncing](clients.md#surfacing-a-page-that-stopped-syncing).

## I deleted something

Deleting is a move to the trash, so it is recoverable.

- **A block, from any day or device.**
  TUI: `/trash` or `:trash`, select the row, press `r`.
  CLI: `outl trash list`, then `outl trash restore`.
  The desktop has a Trash panel too.
  On the phone: **Settings → Trash**, or "Open trash" in the search palette.
- **A page.**
  Open it again by name (`Ctrl+P` in the TUI) and it comes back.
- **Just now.**
  `u` undoes in the TUI, up to 200 steps per session.
- **Before that, or after a bad import.**
  Local snapshots: [Backups and restore](backups.md).

## Sync

Short answers.
The long ones, with every client's steps, are in [Sync your devices](sync-devices.md) and [Sync, done right → Troubleshooting](sync.md#troubleshooting-sync).

**I paired two devices and nothing shows up.**
Pairing links the devices; it does not move history.
Run `outl sync` on the device that is missing notes, or open the app on it, which syncs on launch.

**The log says `rejecting sync from peer on a different workspace`.**
The joining device never adopted the host's workspace id.
Pair again from an **empty folder** on the joiner and look for the line `Joined the host's workspace (...)`.
Both devices need a recent, matching build (`outl --version`).

**The sync dot is orange, or `outl peer status` says `offline`.**
The other device has to be running a long-lived outl: the desktop app, the TUI, `outl mcp serve` or `outl serve`.
A one-shot CLI command opens no endpoint, and a phone answers only while its app is open.
Open outl on the other device and look again.

**`outl sync` exits with code 3.**
It had nothing to do and that is not a failure: another process on this device holds the sync endpoint, P2P is off, or no device is paired.

**Pairing hangs for about 30 seconds and times out.**
One of the addresses in the ticket is unreachable and stalls the whole connect.
Turn off VM bridges and VPN interfaces, generate a new ticket, and put both devices on a network that allows device-to-device traffic (guest Wi-Fi often does not).
Detail: [Sync → Troubleshooting](sync.md#troubleshooting-sync).

**The pairing code stopped working.**
A code works once and expires after two minutes.
Start pairing again for a fresh one.

**I lost a device.**
Run `outl peer revoke-all`, then pair each device you still have.
`outl peer remove` only removes a device on the machine you run it on ([Sync → Locking out a device you no longer have](sync.md#locking-out-a-device-you-no-longer-have)).

## Importing

**`already holds N page(s)` and the import stops.**
A real import into a workspace that has pages would overwrite them, so it refuses.
Import into a fresh directory instead, or pass `--force` if overwriting is exactly what you want.
`--dry-run` is always safe.

**`((UID))` appears in my pages after an import.**
That block reference pointed at something that was not in the export.
outl leaves it exactly as the source wrote it so you can find it: `grep -rn '((' pages/ journals/`.

**`outl import logseq` refuses my folder, and it has a `db.sqlite`.**
That is a Logseq database graph.
Export it from Logseq with **Export graph → Export EDN file** and run `outl import logseq-db <export.edn> ~/notes` ([details](import.md#logseq-db-graphs-logseq-db)).

**I want to see what an import will do before it does it.**
Add `--dry-run`.
It writes nothing and prints the same report, with counts per feature and a reconciliation of pages and blocks.

## Where things live

| What | Where |
|---|---|
| Your notes | The workspace folder: `journals/`, `pages/`, `assets/` |
| Workspace settings | `<workspace>/.outl/config.toml` |
| Your settings for every workspace | `~/.config/outl/config.toml` (same path on macOS, Linux and Windows) |
| The paired-device list | `<workspace>/.outl/peers.json` |
| This device's identity | `~/.outl/identity.key`, plus the device store ([Device store](device-store.md)) |
| The TUI log | `<workspace>/.outl/tui.log` |

## Privacy

**Does outl phone home?**
It sends crash reports, which are on by default in release builds and never contain note content.
Turn them off in Settings → Privacy on the desktop and phone, or run `outl telemetry off` in a terminal.
`outl telemetry status` shows where it stands.
Builds you compile yourself do not report.
Full policy: [Privacy](privacy.md).

## Still stuck

1. Run `outl --version` and `outl doctor`.
2. Search [the issues](https://github.com/outlmd/outl/issues); someone may have hit it.
3. Ask in [Discord](https://discord.gg/fDDXRtFhA6), or open an issue with the version, the client, what you did and what you expected.
   Do not paste note content; the output of `outl doctor` is safe to share.

Per-client pages with more symptoms:

- [Terminal troubleshooting](tui-troubleshooting.md)
- [Desktop troubleshooting](desktop-troubleshooting.md)
- [Mobile troubleshooting](mobile-troubleshooting.md)
- [Sync your devices](sync-devices.md)
- [Backups and restore](backups.md)
