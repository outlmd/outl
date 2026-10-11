# Nix

outl is a [flake](../flake.nix).
One input exposes two surfaces: a set of **packages** and a **home-manager module** for per-user configuration and background sync.

| Flake output | Source | What it is |
|---|---|---|
| `packages.outl` *(default)* | [`flake.nix`](../flake.nix) | The CLI (`outl`) and the TUI (`outl-tui`). |
| `packages.outl-desktop` | [`flake.nix`](../flake.nix) | The Tauri 2 desktop app. |
| `homeManagerModules.default` | [`hm-module.nix`](../hm-module.nix) | `programs.outl` — manages `~/.config/outl/config.toml`, optionally installs the packages and a per-user sync service. |

> **Linux only.**
> Packages build for `x86_64-linux` and `aarch64-linux`.
> There is no Nix-built macOS package: the Tauri desktop build needs Apple SDK frameworks that current nixpkgs no longer exposes as stable attributes.
> On macOS the home-manager module still writes your `config.toml`; you supply the binary yourself (see the [Homebrew tap](homebrew.md)).

## The flake

The flake pins `nixpkgs-unstable`, `flake-utils`, [`rust-overlay`](https://github.com/oxalica/rust-overlay), and [`home-manager`](https://github.com/nix-community/home-manager).

You do not need a Rust toolchain on your machine — the flake supplies one via `rust-overlay`. There is no binary cache, so the first build of each package compiles from source.

## Packages

### CLI + TUI — `packages.outl`

One derivation builds both binaries (`cargo build -p outl-cli -p outl-tui`):

- `outl` — the CLI (workspace ops, `serve`, `peer`, `doctor`, …).
- `outl-tui` — the terminal UI.

```bash
nix profile add github:outlmd/outl     # default package = outl
outl --help
outl-tui
```

Or run without installing:

```bash
nix run github:outlmd/outl -- --help
```

### Desktop — `packages.outl-desktop`

The Tauri 2 app.
The build compiles the Solid frontend with Bun first, then builds the Rust shell with the `tauri/custom-protocol` feature so the webview embeds the frontend instead of pointing at a dev server.
The package also emits a freedesktop `.desktop` entry and hicolor icons into its `$out/share`, so a full desktop session picks up the launcher and icon with no extra setup (home-manager extends this to `~/.local/share` for bare Wayland launchers — see the module below).

```bash
nix profile add github:outlmd/outl#outl-desktop
outl-desktop
```

### Dev shell

`devShells.default` gives you the pinned Rust toolchain plus `cargo-tauri`, `bun`, and `nodejs`, with the package dependencies already on the path:

```bash
nix develop github:outlmd/outl
```

## Home-manager module

[`hm-module.nix`](../hm-module.nix) is exposed as `homeManagerModules.default` under the `programs.outl` namespace.
Its job is **configuration management**: it renders `~/.config/outl/config.toml` from typed options, and (on Linux) can install the packages and start a per-user background sync.

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    outl.url = "github:outlmd/outl";
  };

  outputs = { nixpkgs, outl, ... }: {
    homeConfigurations.you = nixpkgs.legacyPackages.x86_64-linux.home.manager.config {
      imports = [ outl.homeManagerModules.default ];

      programs.outl = {
        enable = true;
        installDesktop = true;            # also put outl-desktop on the PATH

        settings = {
          theme.preset = "gruvbox";
          editor.vimMode = true;
          sync.transport = "iroh";
          reminders.quietHours = "22:00-07:00";
        };

        # Optional per-user background sync (Linux only):
        services.sync = {
          enable = true;
          workspace = "/home/you/notes";
        };
      };
    };
  };
}
```

When enabled, the module:

- Writes `~/.config/outl/config.toml` from `programs.outl.settings` (every key is typed; anything not yet modeled goes in `settings.extraConfig`).
- On Linux, adds `programs.outl.package` to `home.packages` (and `outl-desktop` too, if `installDesktop`).
- If `installDesktop` (Linux), also installs the freedesktop launcher entry and the hicolor icons into `~/.local/share` (`~/.local/share/applications` and `~/.local/share/icons`), symlinked from the `outl-desktop` package so the app menu, the "Open With" menu and Wayland launchers (e.g. `rofi -show drun` under a tiling WM) show the right entry and icon.
  The package is the single source — these are symlinks, not a second copy.
- If `services.sync.enable`, starts a **user** systemd unit `outl-sync` that runs `outl serve --workspace <path>` and restarts on failure.
  `services.sync.watch` and `.sync` toggle the watcher and endpoint halves (`--no-watch` / `--no-sync`); `.rustLog` sets `RUST_LOG`.
  The service runs as *you*, using your own identity (`~/.outl`) and device store (`~/.config/outl`) — pairing is plain `outl peer pair`, and the daemon picks up new peers on its own.

> The service runs while you are logged in.
> For a dedicated always-on box (a NAS, a VPS) where nobody sits at a login, enable lingering so your user manager stays alive without a session: `loginctl enable-linger <user>`.
> That is what keeps the box paired 24/7.

### Settings

`programs.outl.settings` mirrors [`outl.toml`](config.md).
A few of the keys:

| Option | Type | Default | Meaning |
|---|---|---|---|
| `managed` | bool | `true` | Emits the top-level `managed = true` directive so no client rewrites `config.toml`. See [Declarative config](#declarative-config). |
| `theme.preset` | enum | `"outl-light"` | Light side of the pair. One of `outl`, `outl-light`, `default-dark`, `light`, `logseq-light`, `dracula`, `solarized-dark`, `nord`, `monokai`, `gruvbox`. |
| `theme.presetDark` | enum \| null | `null` | Dark side — the preset the TUI renders under `mode = "auto"`/`"dark"`. `null` follows `preset`; the terminal is themed by a lone `theme.preset`. |
| `theme.mode` | enum | `"auto"` | `"light"`, `"dark"`, or `"auto"`. A terminal reads `auto` as **dark**, so `"auto"` themes it by the dark side. |
| `editor.vimMode` | bool | `true` | Vim-style modal bindings (desktop). |
| `sync.transport` | enum | `"iroh"` | `"iroh"` (P2P QUIC) or `"file"` (iCloud / shared FS). |
| `reminders.quietHours` | str? | `null` | e.g. `"22:00-07:00"`. |
| `backup.enabled` / `backup.intervalMinutes` | bool / int | `true` / `30` | Automatic git snapshots of the workspace. |
| `tui.icons` | enum | `"emoji"` | TUI chrome icons: `"emoji"` (works with any terminal font) or `"nerd-font"` (needs a patched font). |
| `extraConfig` | attrs | `{}` | Merged last, for keys the module does not model yet. |

The full key list is in the module source ([`hm-module.nix`](../hm-module.nix)); the meaning of each key is in [Configuration](config.md).

### Declarative config

`programs.outl` writes `config.toml` as a **symlink** into the Nix store.
That only survives if nothing replaces the file — and outl's clients write their settings by **atomic rename**, which detaches the symlink and leaves a plain regular file.
The next `home-manager switch` then refuses to overwrite its own managed target.

So the module sets `managed = true` in the generated file by default, and outl treats that as "do not rewrite":

- Every client (`outl`, `outl-desktop`, `outl-tui`, mobile) skips its config write, so the symlink stays a symlink across switches.
- The **desktop Settings modal disables Save** and points you at `programs.outl.settings`; the theme picker still previews live.
- Because the modal can no longer persist it, **`workspace.last` freezes** unless you pin it. Pin it declaratively to keep the desktop reopening your workspace:

  ```nix
  programs.outl.settings.workspace.last = "/home/you/outl";
  ```

  (Omit it and the desktop just opens the workspace picker on launch — every reader falls through cleanly.)

To go back to app-managed settings (the client persists its own state again), set `programs.outl.settings.managed = false`.
A client write will then detach the symlink again, so also set `xdg.configFile."outl/config.toml".force = true` to let home-manager reclaim it on the next switch.

> **Migrating a machine that already hit the conflict.**
> If you were using this module before the `managed` default, your `~/.config/outl/config.toml` is already a stray regular file and the switch is failing with *"file exists and cannot be overridden"*.
> The module already sets `force = true` on that target, so a `home-manager switch` now takes it back over; from then on the `managed` directive keeps it a symlink. (Drop the stray file by hand first only if you'd edited it.)


### Theme precedence

The theme is the one setting whose effect is easy to misread, because outl stores a **light/dark pair** and resolves it through layers the module does not fully control.

1. **Light/dark pair vs. the terminal.**
   `preset` is the light side; `presetDark` is the dark side.
   A terminal cannot read OS appearance, so the TUI renders the **dark** side whenever `mode` is `"auto"` (the default) or `"dark"`.
   This module makes `presetDark` default to *following* `preset`, so a single `settings.theme.preset = "x"` themes the terminal and the desktop together.
   Set `presetDark` explicitly only when you want the two sides to differ — for example a custom light side with the brand `"outl"` dark theme.
   When you customise neither, the module keeps outl's brand pair `outl-light` / `outl`, so the terminal defaults to the brand dark theme rather than to the light preset.

2. **Global vs. workspace (less common).**
   This module writes the **global** `~/.config/outl/config.toml`, which is outl's *lowest* theme precedence.
   Precedence is first-hit-wins: `--theme <preset>` → a per-workspace `<root>/.outl/config.toml` `[theme] preset` → the global file → the built-in default.
   A default `outl init` workspace has **no** `[theme]` section, so home-manager's global value applies everywhere.
   Only a workspace whose `.outl/config.toml` you have hand-added a `[theme] preset` to **overrides** home-manager for that workspace — the "I set `preset` in Nix but this one folder still shows another theme" case.
## Which do I use?

| Situation | Use |
|---|---|
| Just want the CLI/TUI on any Nix machine | `packages.outl` |
| Want `config.toml` managed + optional logged-in sync on a laptop/desktop | `homeManagerModules.default` |
| A dedicated box that must stay paired 24/7 (NAS, VPS) | `homeManagerModules.default` + `loginctl enable-linger <user>` |
