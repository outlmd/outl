# Getting started

Ten minutes from nothing to a synced journal on two devices.
This page installs outl, creates a workspace, writes a first note, links it, and pairs a second device.
Every command below was run against the current build.

## Pick your first client

You can start on any client and add the others later.
They all read the same workspace, and each one can do the same core things: journal, `[[links]]`, `#tags`, backlinks, tasks, sync.

| Client | Pick it when | Install |
|---|---|---|
| **TUI + CLI** (`outl`) | You live in a terminal, or you want scripts, cron and MCP | [Homebrew, Scoop, Nix, source](#install-the-cli-and-tui) |
| **Desktop** | You want a window, a mouse and OS shortcuts on macOS, Linux or Windows | [Desktop app](#install-the-desktop-app) |
| **iOS** | You want your journal on a phone | [TestFlight](#install-on-ios) |
| **Android** | Same, on Android | [Signed APK](#install-on-android) |

What each client can and cannot do, row by row, is in [Client parity](client-parity.md).
A shorter version is in [Which client does what](clients.md#which-client-does-what).

## Install the CLI and TUI

One binary, `outl`, is both the terminal UI and the command line.
Run it with no subcommand and you get the TUI.

### macOS and Linux (Homebrew)

```bash
brew tap outlmd/outl https://github.com/outlmd/outl
brew trust outlmd/outl             # one-time: Homebrew 4.x requires it for third-party taps
brew install outl-beta             # nightly beta cut from main
# brew install outl                # stable channel, once the first stable release ships
```

Already on the old `avelino/outl` tap?
Run `brew untap avelino/outl` first.
The repository moved to the `outlmd` organisation and a tap name does not follow GitHub's redirect.
Channels, upgrades and switching between stable and beta are in [Homebrew tap](homebrew.md).

### Windows (Scoop)

```powershell
scoop bucket add outl https://github.com/outlmd/outl
scoop install outl/outl-beta       # CLI + TUI + MCP server
```

Scoop is unprivileged and puts `outl` on your `PATH`.
One manifest covers `64bit` and native `arm64`.
See [Scoop bucket](scoop.md) for updates and pinning.

### Nix (Linux)

```bash
nix run github:outlmd/outl -- --help       # try it without installing
nix profile add github:outlmd/outl         # install
```

There is also a Home Manager module that writes your `config.toml`.
Details, and why there is no Nix-built macOS package, are in [Nix](nix.md).

### From source

```bash
git clone https://github.com/outlmd/outl.git
cd outl
cargo build --release
cp target/release/outl ~/.local/bin/
```

`rust-toolchain.toml` pins the Rust version, so `rustup` picks it up on its own.

### Linux without a package manager

Each [release](https://github.com/outlmd/outl/releases) carries `outl-linux-x64.tar.gz` and `outl-linux-arm64.tar.gz`.
Windows has `outl-windows-x64.zip` and `outl-windows-arm64.zip`.

## Install the desktop app

### macOS

```bash
brew install --cask outl-desktop-beta      # same tap as above
```

The app is unsigned for now, so the first launch is blocked by Gatekeeper.
Right-click → Open no longer clears it on macOS 15.
Use either of these:

```bash
xattr -dr com.apple.quarantine /Applications/outl.app
```

or open **System Settings → Privacy & Security** and click **Open Anyway** next to the outl entry.
Step-by-step with screenshots of the wording: [Homebrew tap → First-launch Gatekeeper warning](homebrew.md#first-launch-gatekeeper-warning).

### Linux

Homebrew has no GUI casks on Linux, so download from the [releases page](https://github.com/outlmd/outl/releases).
Each release has three formats for `x86_64` and `aarch64`:

| Asset | Use it when |
|---|---|
| `outl-desktop-linux-<arch>.AppImage` | Any distro, no install: `chmod +x` and run |
| `outl-desktop-linux-<arch>.deb` | Debian or Ubuntu: `sudo apt install ./outl-desktop-linux-<arch>.deb` |
| `outl-desktop-linux-<arch>.rpm` | Fedora or RHEL: `sudo dnf install ./outl-desktop-linux-<arch>.rpm` |

`<arch>` is what `uname -m` prints.
The builds need glibc 2.35 or newer: Ubuntu 22.04+, Debian 12+ (Raspberry Pi OS bookworm included), Fedora 36+, RHEL 10+.
`ldd --version` tells you which side of the line you are on.
Older than that, [build from source](#from-source).

On Nix: `nix profile add github:outlmd/outl#outl-desktop`.

### Windows

Download `outl-desktop-windows-x86_64-setup.exe` (or `-aarch64-` on Windows 11 ARM) from the [releases page](https://github.com/outlmd/outl/releases).
The installer is unsigned, so SmartScreen shows "Windows protected your PC": click **More info → Run anyway**.
It downloads Microsoft's WebView2 runtime if the machine lacks it, which needs a network.
Verify the download with the matching `.sha256` file if you want to:

```powershell
Get-FileHash .\outl-desktop-windows-x86_64-setup.exe -Algorithm SHA256
```

## Install on iOS

Install **TestFlight** from the App Store, then open <https://testflight.apple.com/join/P2GdWAMd> on the iPhone and accept the beta.
The **outl** app lands on the home screen.

On first launch the app tells you where notes live: in its own folder on the phone, with no folder picker and no iCloud option.
It works offline and syncs peer to peer with your other devices.
There is no account to create.
iOS pairing is by camera: see [Sync a second device](#sync-a-second-device).

## Install on Android

Download `outl-android-arm64.apk` from the latest [release](https://github.com/outlmd/outl/releases) and open it on the phone.
Android asks you to allow installs from your browser or file manager the first time.

Each release is signed with a fresh key, so installing a new build over an old one fails with `INSTALL_FAILED_UPDATE_INCOMPATIBLE`.
Uninstall the old build first.
Uninstalling removes the phone's local copy, and its device identity with it.
Whatever had already synced to another device comes back when you pair the phone again.
Background on the signing gap: [#171](https://github.com/outlmd/outl/issues/171), [Android platform](android-platform.md).

## Create a workspace

A workspace is a plain directory.
Desktop and mobile create one for you on first run: the desktop asks you to pick a folder, the phone uses its own.
For the terminal:

```bash
outl init ~/notes
```

```text
Initialized outl workspace at /Users/you/notes
  ops:      /Users/you/notes/ops
  config:   /Users/you/notes/.outl/config.toml
  journal:  /Users/you/notes/journals/2026-11-25.md
  scope:    global
```

What you get:

```text
~/notes/
├── .outl/
│   ├── config.toml         # workspace settings
│   ├── workspace-id        # what makes two devices the same graph
│   ├── peers.toml          # placeholder; paired devices live in peers.json after you pair
│   └── orphans.log         # blocks that lost their identity during an outside edit
├── assets/                 # attached files, named by content hash
├── journals/
│   ├── 2026-11-25.md       # today, seeded with one empty bullet
│   └── 2026-11-25.outl     # sidecar: block ids for the .md next to it
├── ops/
│   └── ops-<actor>.jsonl   # this device's op log, the source of truth
├── pages/
│   ├── templates-journal.md   # the template stamped onto every new journal day
│   └── templates-journal.outl
└── templates/
```

The `.md` files are the notes.
Open them in any editor; they hold no ids, no HTML comments, nothing outl-specific beyond `key:: value` properties and `[[links]]`.
Block ids live in the `.outl` file beside each page, and history lives in `ops/`.
You never edit either by hand.
Full anatomy: [Workspace layout](concepts.md).

Running `outl init` on an existing workspace is safe.
Passing `--bare` writes no journal and no template, which is only for a server replica ([Self-hosting](self-hosting.md)).

## Write your first note

### In the terminal

```bash
outl --workspace ~/notes     # or: cd ~/notes && outl
```

You land on today's journal, in Normal mode, with one empty bullet and the hint `press i to write · ? for keys`.
Normal mode moves around; Insert mode types.

| Do this | Press |
|---|---|
| Start typing in the block | `i` |
| Finish the block | `Esc` |
| New block below, and type | `o` |
| Link a page | type `[[`, pick from the list, or type the name and close with `]]` |
| Open the link under the cursor | move onto it with `w` or `l`, then `Enter` |
| Back to today | `t` |
| Previous or next day | `[` and `]` |
| Find any page or day | `Ctrl+P` |
| Every key, in context | `?` |
| Commands (`/` menu, `:` vim line) | `/` or `:` |
| Quit | `q q` (a single `q` only arms it) or `Ctrl+C` |

Try this:

1. Press `i`, type `met with [[Avelino]] about #distsys`, press `Esc`.
2. Press `w` until the cursor sits on `Avelino`, then `Enter`.
   outl creates the page `Avelino` and opens it.
   The status line says `created page "Avelino"`.
3. The page shows a **Backlinks** section with today's block.
   `Ctrl+B` hides and shows it.
4. Press `t` to go back to today.

Nothing waits for a save command.
Each commit (`Esc`, `Enter`) writes the `.md` and records the ops.
Run `cat ~/notes/journals/$(date +%F).md` in another terminal and you will see the line.

A link only becomes a page when you open it.
Typing `[[Avelino]]` on its own does not create `pages/avelino.md`.
This keeps a typo from littering the workspace.

### On the desktop

First launch is two steps: pick the folder, then optionally pair a device.
Skip the second and you still have a complete app.
You land on today's journal.
Click a block or press `i` to type, `Cmd/Ctrl+P` to jump to a page, `Cmd/Ctrl+J` for today, `?` with nothing focused for help.
The sidebar (`Cmd/Ctrl+Shift+E`) and backlinks (`Cmd/Ctrl+Shift+B`) are one chord away.
The full tour is [Using outl on the desktop](desktop.md); chords are in [Shortcuts](shortcuts.md).

### On a phone

First launch explains where notes live, offers pairing, and lands on today's journal.
Tap a block to type.
The bottom bar has Journal, Search, Tasks and Reminders, and the `+` in the middle adds a block to today wherever you are.
Long-press a block for its menu.
**Settings → What's what** reopens the walkthrough.
The full guide is [outl on your phone](mobile.md).

## Sync a second device

Sync is peer to peer, end-to-end encrypted, with no account and no server holding your notes.
Pairing is how two devices become one graph.
The mental model, which saves you from the one common mistake:

> One device is the **host**: it already has your notes.
> The other is the **joiner**.
> Pairing makes the joiner adopt the host's workspace id, so from then on both folders are the same graph.
> Run the joiner's step from an **empty folder** (or a fresh app install).
> The joiner does not keep an unrelated notes folder as a separate workspace.

| Joining device | On the host | On the joiner |
|---|---|---|
| Phone | Desktop: **Settings → Sync → Pair a new device…** shows a QR. Terminal: `outl peer pair` | **Settings → Devices and sync → Scan QR** |
| Another computer | `outl peer pair` prints a QR and a ticket | `outl peer pair --ticket <ticket>` from an empty folder |
| Server replica | `outl peer pair` | [Self-hosting](self-hosting.md) |

The phone scans and the desktop shows.
A desktop cannot scan a QR, so to *join* from a desktop use the terminal command.
The TUI has no pairing screen; run `outl peer pair` from the shell.

On a computer:

```bash
# host
cd ~/notes
outl peer pair

# joiner
mkdir ~/notes && cd ~/notes
outl peer pair --ticket <ticket>      # expect: Joined the host's workspace (...)
outl sync                             # pull the history once
```

The pairing code works once and expires after two minutes.
After the first `outl sync`, an open desktop, TUI or phone app keeps syncing on its own.
`outl peer status` shows who is reachable, and the dot in the desktop and phone top bar is green when at least one paired device answers.

The step-by-step guide for every host and joiner combination, the transport choice, daily use and what to do with a lost device is [Sync your devices](sync-devices.md).
The reasoning behind the merge is [Sync, done right](sync.md).
When it does not work: [Troubleshooting → Sync](troubleshooting.md#sync).

## Bring your notes from another tool

```bash
outl import auto <source> ~/notes --dry-run     # measure first, writes nothing
outl import auto <source> ~/notes               # then do it
```

`<source>` is a Roam JSON backup, a Logseq graph folder, a Logseq DB `.edn` export, or an Obsidian vault.
`auto` detects which.
Name the format (`roam`, `logseq`, `logseq-db`, `obsidian`) when you want to be explicit.
The destination is created if it does not exist.
A real run refuses to write into a workspace that already has pages and tells you to use a fresh directory.
`--force` overrides that and overwrites the pages the import writes, so reach for it only when that is what you want.

The dry run prints what would happen.
For a tiny Logseq graph it reads like this:

```text
Import summary (logseq):
  pages:               1
  journals:            1
  blocks:              4
  block refs:          1 resolved, 0 page-fallback, 0 unresolved
  tasks:               NOW: 1, TODO: 1
  artifacts stripped:  1 (id:: lines, #+ directives, LOGBOOK drawers, dropped frontmatter keys)
  reconciliation:
    pages:             2/2 emitted (0 merged, 0 skipped)
    blocks:            4/4 emitted (0 lifted to page props, 0 in skipped pages)
```

Read the `reconciliation` lines: emitted should equal found.
`((uid))` references the importer cannot resolve stay as the source wrote them, so `grep '((' pages/*.md` finds what is left.
What each dialect becomes, and the flags (`--preserve-timestamps`, `--no-assets`, `--keep-source-ids`): [Importing a graph](import.md).

## Check that everything is healthy

```bash
outl doctor
```

It reads the op log, the projections and the sidecars and ends with `integrity OK` or a list of what is wrong.
It writes nothing unless you pass `--repair`.
Details and the repair rules: [outl doctor](doctor.md).

## Where to go next

| You want to | Read |
|---|---|
| Learn the editor through a week of real use | [Your first week with outl](tutorial.md) |
| Ten minutes of guided terminal practice | [Your first 10 minutes in the TUI](tui-quickstart.md) |
| Every key and overlay in the terminal | [TUI manual](tui.md) |
| The desktop app, panel by panel | [Using outl on the desktop](desktop.md) |
| The phone app | [outl on your phone](mobile.md) |
| Put notes on more devices | [Sync your devices](sync-devices.md) |
| Undo a bad import or a deleted page | [Backups and restore](backups.md) |
| Every shortcut on every client | [Shortcuts](shortcuts.md) |
| Change colours or write a theme | [Theming](theming.md) |
| Settings and `config.toml` | [Configuration](config.md) |
| Script outl or plug it into an AI assistant | [CLI](cli.md), [MCP](mcp.md) |
| Query your notes | [Query code blocks](query.md) |
| Something broke | [Troubleshooting](troubleshooting.md) |
| Ask a human | [Discord](https://discord.gg/fDDXRtFhA6) or [GitHub issues](https://github.com/outlmd/outl/issues) |
