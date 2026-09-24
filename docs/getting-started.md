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

### Desktop app on Linux

Homebrew doesn't ship GUI casks on Linux, so the desktop app is published as direct download assets on every [GitHub release](https://github.com/outlmd/outl/releases).
Each release carries three formats for `x86_64`:

| Asset | Use it when |
|-------|-------------|
| `outl-desktop-linux-x86_64.AppImage` | Any distro, no install — `chmod +x` and run. The portable default. |
| `outl-desktop-linux-x86_64.deb` | Debian / Ubuntu (`sudo apt install ./outl-desktop-linux-x86_64.deb`). |
| `outl-desktop-linux-x86_64.rpm` | Fedora / RHEL (`sudo dnf install ./outl-desktop-linux-x86_64.rpm`). |

```bash
# AppImage — portable, no install
curl -LO https://github.com/outlmd/outl/releases/latest/download/outl-desktop-linux-x86_64.AppImage
chmod +x outl-desktop-linux-x86_64.AppImage
./outl-desktop-linux-x86_64.AppImage
```

Every asset ships a matching `.sha256` sidecar.
The CLI + TUI on Linux still install via Homebrew above.

### Desktop app on Windows

Homebrew doesn't run on Windows, so the desktop app ships as a direct download on every [GitHub release](https://github.com/outlmd/outl/releases).

| Asset | Use it when |
|-------|-------------|
| `outl-desktop-windows-x86_64-setup.exe` | Windows 10 / 11 on `x86_64`. NSIS installer — it pulls the WebView2 runtime if the machine doesn't already have it. |

```powershell
# verify the download before running it
Get-FileHash .\outl-desktop-windows-x86_64-setup.exe -Algorithm SHA256
# compare against outl-desktop-windows-x86_64-setup.exe.sha256
```

The installer is unsigned today, so SmartScreen shows "Windows protected your PC" on first run — **More info → Run anyway**.
There is no `.msi`: WiX rejects a pre-release version whose identifier isn't numeric-only (`0.12.0-beta.201`), and every build on this channel is a beta.

The CLI + TUI on Windows are the separate `outl-windows-x64.zip` asset on the same release.

### From source

```bash
git clone https://github.com/outlmd/outl.git
cd outl
cargo build --release
```

You need Rust 1.88+.
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
