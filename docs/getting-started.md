# Getting started

A minute from clone to first journal entry.

## Install

### Homebrew (macOS / Linux)

```bash
brew tap outlmd/outl https://github.com/outlmd/outl
brew trust outlmd/outl             # one-time: Homebrew 4.x requires it for third-party taps

# CLI + TUI (Linux + macOS)
brew install outl-beta              # latest beta — every push to main
# brew install outl                 # latest GA (when the first one ships)

# Desktop app (macOS only, dmg)
brew install --cask outl-desktop-beta
```

> Already on the old `avelino/outl` tap? Run `brew untap avelino/outl` first — the repo moved to the `outlmd` org and a tap name doesn't follow GitHub's redirect. See [Homebrew tap → moved tap](homebrew.md#install-the-cli--tui).

The desktop cask drops `outl.app` into `/Applications`.
The dmg is unsigned today, so on first launch you'll need to right-click → Open (or `xattr -dr com.apple.quarantine /Applications/outl.app`).
CLI and desktop coexist; they share the workspace on disk through the op log.

See [Homebrew tap](homebrew.md) for the channel rules, how switching between GA and beta works, and the Gatekeeper details for the desktop cask.

### Scoop (Windows)

```powershell
scoop bucket add outl https://github.com/outlmd/outl
scoop install outl/outl-beta          # CLI + TUI + MCP server
```

Unprivileged, portable, and on PATH as `outl`.
One manifest serves both `64bit` and native `arm64`, so the line is the same on an ARM laptop.
See [Scoop bucket](scoop.md) for updates, pinning, and why the desktop app is not a Scoop package.

### Desktop app on Linux

Homebrew doesn't ship GUI casks on Linux, so the desktop app is published as direct download assets on every [GitHub release](https://github.com/outlmd/outl/releases).
Each release carries three formats, for `x86_64` and for `aarch64`:

| Asset | Use it when |
|-------|-------------|
| `outl-desktop-linux-<arch>.AppImage` | Any distro, no install — `chmod +x` and run. The portable default. |
| `outl-desktop-linux-<arch>.deb` | Debian / Ubuntu (`sudo apt install ./outl-desktop-linux-<arch>.deb`). |
| `outl-desktop-linux-<arch>.rpm` | Fedora / RHEL (`sudo dnf install ./outl-desktop-linux-<arch>.rpm`). |

`<arch>` is what `uname -m` prints: `x86_64` on a PC, `aarch64` on a Raspberry Pi 4 / 5, an ARM laptop, or an ARM cloud instance.

```bash
# AppImage — portable, no install
ARCH=$(uname -m)
curl -LO "https://github.com/outlmd/outl/releases/latest/download/outl-desktop-linux-${ARCH}.AppImage"
chmod +x "outl-desktop-linux-${ARCH}.AppImage"
"./outl-desktop-linux-${ARCH}.AppImage"
```

Both architectures are built natively (no emulation) on Ubuntu 22.04, so the floor is **glibc 2.35**: Ubuntu 22.04+, Debian 12+ (Raspberry Pi OS bookworm included), Fedora 36+, RHEL 10+.
A distro older than that is the one case where [building from source](#from-source) is the answer — `ldd --version` tells you which side of the line you are on.

Every asset ships a matching `.sha256` sidecar.
The CLI + TUI on Linux install via Homebrew above on both architectures (ARM64 Linux is a Tier 1 Homebrew platform), or as the `outl-linux-x64.tar.gz` / `outl-linux-arm64.tar.gz` asset on the same release if you'd rather not use a package manager.

### Desktop app on Windows

Homebrew doesn't run on Windows, so the desktop app ships as a direct download on every [GitHub release](https://github.com/outlmd/outl/releases).

| Asset | Use it when |
|-------|-------------|
| `outl-desktop-windows-x86_64-setup.exe` | Windows 10 / 11 on `x86_64`. NSIS installer — it pulls the WebView2 runtime if the machine doesn't already have it. |
| `outl-desktop-windows-aarch64-setup.exe` | Windows 11 on ARM (Snapdragon X, Surface Pro ARM). Native ARM64 build, same installer shape. |

```powershell
# verify the download before running it
Get-FileHash .\outl-desktop-windows-x86_64-setup.exe -Algorithm SHA256
# compare against outl-desktop-windows-x86_64-setup.exe.sha256
```

The installer is unsigned today, so SmartScreen shows "Windows protected your PC" on first run — **More info → Run anyway**.
There is no `.msi`: WiX rejects a pre-release version whose identifier isn't numeric-only (`0.12.0-beta.201`), and every build on this channel is a beta.
The installer downloads Microsoft's WebView2 bootstrapper when the runtime is not already there.
That download needs a network, and it does not run again if the runtime is removed later or the program is started without the installer.
In those cases outl shows a Windows message naming the Evergreen WebView2 runtime and where to download it, then exits.

The CLI + TUI on Windows install through [Scoop](scoop.md), or as the `outl-windows-x64.zip` / `outl-windows-arm64.zip` asset on the same release.

> The ARM64 Windows builds are the newest thing here and are marked optional in the release pipeline, which means a release can publish without them if that leg fails.
> When that happens the release notes say so, in that release's own body — so if an `aarch64` asset is not on a release, read its notes rather than assuming the naming changed.

### From source

```bash
git clone https://github.com/outlmd/outl.git
cd outl
cargo build --release
```

You need Rust 1.99+.
`rust-toolchain.toml` pins the version, so `rustup` will pick it up automatically.

Drop the binary anywhere on your `PATH`:

```bash
cp target/release/outl ~/.local/bin/
```

(Or use `cargo install --path crates/outl-cli` if that's your flavor.)

## iOS app (TestFlight beta)

The iOS client is shipping as a public TestFlight beta:

> **<https://testflight.apple.com/join/P2GdWAMd>**

Install TestFlight from the App Store, open the join link on the iPhone, accept the beta, and the **outl** app lands on the home screen.
It writes its op log to its own iCloud Drive container (`iCloud.app.outl.mobile-app`).
To share a workspace with the TUI, point `outl --workspace` at the same `Documents/` directory inside the container:

```bash
outl --workspace ~/Library/Mobile\ Documents/iCloud~app~outl~mobile-app/Documents
```

Each device writes only to its own `ops-<actor>.jsonl`, so iCloud never has to merge — the CRDT does that.

## Create a workspace

A workspace is just a directory.
Pick a path, point `outl init` at it:

```bash
outl init ~/notes
```

You'll get:

```
~/notes/
├── .outl/
│   ├── config.toml     # workspace identity + settings
│   ├── peers.toml      # P2P peers
│   └── orphans.log     # log of unmatched blocks during external edits
├── ops/                # the op log, one ops-<actor>.jsonl per device
├── pages/              # your named pages live here
├── journals/
│   └── 2026-05-25.md   # today's journal, seeded
└── templates/
    └── journal.md      # template applied to new journals
```

## Open the TUI

```bash
outl --workspace ~/notes
```

It lands you on today's journal.
Press `?` to see every keymap.

Or, if you `cd ~/notes`, just `outl` works — no subcommand means "open the TUI here."

## First moves

| You want to... | Do this |
|----------------|---------|
| Start typing | `i` (edit current block) or `o` (new block below) |
| Open `[[Avelino]]` (creating the page if needed) | type `[[`, autocomplete, press `Enter` over the link |
| Jump to today | `t` |
| Yesterday / tomorrow | `[` / `]` |
| Find any page or journal | `Ctrl+P` (fuzzy switcher) |
| Run a command | `/` (Notion-style menu) or `:` (vim palette) |
| Search the whole workspace | `/search` (alias `/s`) |
| Insert today's date as `[[link]]` | `/date-today` (in Insert mode) |
| Quit | `q` or `Ctrl+C` |

Pages you reference but haven't created yet are *real* the moment you press `Enter` on the link — outl creates `pages/<slug>.md` with `title:: <Name>` automatically.

## Try a theme

Eight built-in palettes:

```bash
outl --workspace ~/notes --theme outl
outl --workspace ~/notes --theme logseq-light
outl --workspace ~/notes --theme dracula
outl --workspace ~/notes --theme nord
outl --workspace ~/notes --theme monokai
outl --workspace ~/notes --theme solarized-dark
outl --workspace ~/notes --theme light
outl --workspace ~/notes --theme default-dark
```

To pin a theme **for every workspace** (shared between the TUI and the desktop app), edit the global config:

```toml
# ~/.config/outl/config.toml
[theme]
preset = "dracula"

[editor]
vim_mode = true
font_size = 15
```

The path is the same on macOS, Linux and Windows — XDG-style — so a single file holds your personal defaults across every client.
The desktop app's **Settings modal** writes here too; flipping the theme there updates the TUI on its next launch.

To pin a theme **only for one workspace**, edit its `.outl/config.toml`:

```toml
[theme]
preset = "monokai"
```

Or switch at runtime in the TUI: open the command palette with `:`, type `theme nord`, hit Enter.

## Edit `.md` externally

Open any file in `~/notes/pages/` with VS Code, vim, Obsidian — whatever.
The file is plain markdown:

```markdown
title:: Avelino

- some block
- another block with [[ref]] and #tag
```

There are no `id::` lines, no UUIDs, no HTML comments.
When you save and reopen the TUI, outl matches each block back to its sidecar entry and rebuilds the op log.

## Next steps

- The [TUI manual](tui.md) — every key, every overlay, persistence rules, gotchas.
- [Why outl](why-outl.md) — the pitch vs. Roam and Logseq.
- [Sync, done right](sync.md) — what makes the algorithm interesting.
- [Build your first plugin](plugin-tutorial.md) — extend outl in JavaScript; `outl plugin init` scaffolds a running plugin in 60 seconds.
