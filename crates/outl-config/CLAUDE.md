# CLAUDE.md — outl-config

Shared user-config crate for every outl client.
**One file in one place** — `~/.config/outl/config.toml` — read by the TUI, the CLI, and the desktop app via this same module.
Read this before adding a field.

## Why this crate exists

Before this crate, the desktop wrote settings to `~/Library/Application Support/app.outl.desktop/settings.json` (JSON, macOS-only path) and the TUI carried per-workspace state in `<workspace>/.outl/config.toml`.
Two readers, two writers, two schemas — flipping a theme in the desktop did nothing for the TUI on the next launch.
This crate ends that: TOML, XDG-style on every OS (including macOS), one schema, both clients import the same `Config` struct.

## Hard rule

**No client parses or writes `config.toml` by hand.**
Every read goes through [`load`] / [`load_from`]; every write goes through [`save`] / [`save_to`].
Bypassing this crate is how schema drift starts.

The desktop's `settings.rs` is the canonical adapter pattern: a flat wire-format struct for the frontend, converted via `From` impls in and out of `outl_config::Config`.
If a new client needs a different shape on the wire, do the same — adapt, don't fork the reader.

## Path layout

```
~/.config/outl/                         ← `config_dir()` (XDG-style on every OS)
├── config.toml                         ← `config_path()`
├── machine-id                          ← device fingerprint (outl-core's device store)
├── actor                               ← device-wide ULID, desktop + mobile (outl-core's device store)
└── actors/<workspace-id>               ← per-workspace ULID, CLI + TUI + MCP (outl-core's device store)
```

- macOS / Linux: respects `$XDG_CONFIG_HOME` first, else `~/.config/outl/`.
- Windows: `$XDG_CONFIG_HOME\outl\` when set, else `%APPDATA%\outl\` (whatever `dirs::config_dir()` returns, typically `C:\Users\<user>\AppData\Roaming\outl`).
- **Not** `~/Library/Application Support/…` on macOS — deliberate (see lib doc).
- **Not** `%USERPROFILE%\.config\outl\` on Windows either.
  The `~/.config` layout is not a Windows convention, and dropping the config under `%USERPROFILE%` directly would surprise PowerShell users and tools that expect Roaming.
  The `cfg(windows)` branch in `config_dir()` routes through `dirs::config_dir()` to honour that.

The `machine-id` / `actor` / `actors/` entries next to `config.toml` are **not** part of this crate's schema.
They are `outl_core::DeviceStore` (`crates/outl-core/src/device/`), which resolves its own directory via `outl_core::device_dir()`.
That is the same base path as [`config_dir`], plus an `$OUTL_DEVICE_DIR` override, so a test or container can rotate this device's identity without discarding the user's preferences.
Two functions on purpose: one answers "where are the user's preferences", the other "where is this device's identity".
If the base layout ever moves, move both.

Don't add `actor` to `Config`.
An actor id must **differ** per device, and `config.toml` is a file users copy between machines; that is the exact shape of the bug `outl-core/CLAUDE.md` → "Actor id is device-local" describes.

## Schema

```toml
[workspace]
last = "/Users/me/iCloud/outl"   # absolute path; optional

[theme]
preset = "outl"                   # name from outl_theme::PRESETS; the light side of the pair
preset_dark = "dracula"           # optional; dark side. Omit = falls back to `preset` (pre-RFC-0022 behaviour)
mode = "auto"                     # "light" | "dark" | "auto" (default); TUI treats "auto" as "dark"

[editor]
vim_mode = true                   # default true
font_size = 15                    # pixels, desktop-only

[calendar]
timezone = "Europe/London"        # optional IANA name; omit = OS local timezone

[sync]
transport = "iroh"                # "iroh" (P2P, default) | "file" (iCloud/fs opt-out)
relay_url = ""                    # optional; empty = outl's default relay (use1-1.relay.avelino.outl.iroh.link)

[tui]
mouse_capture = false             # opt-in: enables mouse wheel + click + drag-to-copy in the TUI

[display]
backlinks_order = "newest"        # "newest" (default) | "oldest" — direction of the backlinks list

[assets]
max_bytes = 104857600             # 100 MiB default; 0 = unbounded. Cap on a single uploaded file

[reminders]
enabled = true                    # default on; `remind::` on a block is itself the opt-in
quiet_hours = "22:00-07:00"       # optional; a fire landing inside is pushed to the window's end

[backup]
enabled = true                    # default on; automatic local git snapshots of the workspace
interval_minutes = 30             # floor between automatic snapshots, not a schedule
```

Nine sections, each modelled as its own struct ([`WorkspaceCfg`], [`ThemeCfg`], [`EditorCfg`], [`CalendarCfg`], [`SyncConfig`], [`TuiCfg`], [`DisplayCfg`], [`AssetsCfg`], [`RemindersCfg`]).
`ThemeCfg` additionally carries a [`ThemeMode`] enum field (`mode`); see below.
`RemindersCfg::enabled` defaults to **`true`**, the one non-`Default::default()` bool in the schema: `remind::` on a block is itself the opt-in, so defaulting off just made a written rule silently do nothing.
`RemindersCfg::quiet_window()` parses `"22:00-07:00"` into `(start, end)` minutes past midnight and returns `None` on anything unparseable, so a typo degrades to "no quiet hours" instead of failing the load.
`CalendarCfg::timezone` is an optional IANA name resolved at boot by `outl_actions::clock::init`; missing/empty/unknown falls back to the OS local timezone (the previous behaviour).
It exists for environments where the OS clock lies about the zone — containers and Chrome OS **Crostini** run in UTC regardless of the user's real timezone (issue #107).
`SyncConfig::transport` is a [`SyncTransportKind`] enum (`File` | `Iroh`, serde `lowercase`); missing `[sync]` falls back to `Iroh` (P2P is outl's primary sync), and `transport = "file"` is the explicit iCloud/filesystem opt-out.
`SyncConfig::relay_url()` treats an empty string as `None`, which the iroh transport resolves to outl's default relay (`use1-1.relay.avelino.outl.iroh.link`; see [`docs/relay.md`](../../docs/relay.md)).
`TuiCfg::mouse_capture` (default `false`) is read by the TUI at boot in `runtime.rs` to decide whether to call `EnableMouseCapture` and listen for `Event::Mouse`; the desktop ignores this section entirely.
`DisplayCfg::backlinks_order` is a [`BacklinksOrder`] enum (`Newest` | `Oldest`, serde `lowercase`, default `Newest`) — a pure display preference, same "never converges between devices" policy as `theme.preset` (root `CLAUDE.md` invariant #7).
`ThemeCfg` (RFC 0022) models a light/dark preset *pair*, not a single preset.
`preset` is the light side, `preset_dark: Option<String>` is the dark side, and `mode` is a [`ThemeMode`] enum (`Light` | `Dark` | `Auto`, serde `lowercase`, default `Auto`).
`ThemeCfg::dark()` returns `preset_dark` when set, else falls back to `preset`.
That fallback is what keeps a pre-RFC-0022 config with only `preset` behaving byte-for-byte the same (`mode = "auto"` alternating between the same preset on both sides).
`ThemeMode` names a *side* to render, not a colour, so nothing stops a misconfigured pair (a dark preset in `preset`); that is surfaced by `outl doctor`, not resolved here.
`BacklinksOrder::newest_first()` returns the `bool` `outl_actions::sort_backlinks` expects.
`BackupCfg::enabled` defaults to **`true`** — the second non-`Default::default()` bool in the schema, for the same reason as `RemindersCfg::enabled`.
The failures a backup catches (a projection bug, a mis-aimed `outl import` over a populated workspace, a page deleted with the app then closed) are ones the user discovers *after* the window to enable a safety net has closed.
It costs nothing on an unchanged workspace (no diff, no commit) and degrades to a `warn!` where there is no `git` on `PATH`.
The engine is `outl_actions::backup`; this section only carries the preference.
`AssetsCfg::max_bytes` (default `100 * 1024 * 1024`, `0` = unbounded) is the upper bound on a single file `outl_actions::import_asset` copies into `<workspace>/assets/`; the directory itself is fixed by `outl-ws`'s layout, not configurable here.
`#[serde(default)]` everywhere — a missing field falls back to the type's `Default`, so an older binary reading a newer config doesn't choke and a newer binary reading an older config doesn't blow up.

## Behaviour contract (read this before changing anything)

| Situation | What this crate does |
|---|---|
| File missing | Returns `Config::default()` silently. First launch is normal. |
| File present, empty | Returns `Config::default()`. |
| File present, malformed TOML | Returns `Config::default()` **+ `tracing::warn!`**. Never panics. |
| Unknown field | Ignored. Older binary survives a newer config. |
| Partial section (e.g. only `[theme]` populated) | Other sections fall back to their per-section `Default`. |
| `save()` | Atomic write (`config.toml.tmp` → rename). Creates `~/.config/outl/` if missing. A crash mid-write never leaves a truncated config. |

The forgiving read path is **load-bearing for UX**: a user editing TOML by hand mid-typo doesn't lose every preference; they just see defaults until the next save fixes the file.
Do not make load fail-fast — fail-fast belongs in the workspace itself, not in user preferences.

## Adding a field

1. Add the field to the relevant struct in `src/schema.rs` with `#[serde(default)]` (or a per-type `Default` impl).
2. Update the example in `src/lib.rs`'s module doc.
3. Update `docs/config.md` — the user-facing schema table.
4. Update `crates/outl-cli/CLAUDE.md` and/or `crates/outl-desktop/CLAUDE.md` and/or `crates/outl-tui/CLAUDE.md` if a new client now reads the field.
5. Wire the reader in the consuming crate (`outl-tui/src/runtime.rs` for TUI, `outl-desktop/src-tauri/src/settings.rs` for desktop).
6. Add a `tests` case covering the partial-TOML path (only the new section populated) to confirm the default still applies.

If the field is **per-workspace** (not global), it doesn't belong here — it belongs in `<workspace>/.outl/config.toml`, written by `outl-cli`'s `init` command.
If the field **must converge between devices**, it doesn't belong in TOML at all — it goes through the op log (root `CLAUDE.md` invariant #7).

## Where each field is read

| Field | Reader | File |
|---|---|---|
| `workspace.last` | TUI/CLI fallback in `resolve_path`; desktop on boot | `crates/outl-cli/src/main.rs::resolve_path`, `crates/outl-desktop/src-tauri/src/lib.rs::run` |
| `theme.preset` | TUI palette resolver; desktop settings | `crates/outl-tui/src/runtime.rs::resolve_theme`, `crates/outl-desktop/src-tauri/src/commands/theme.rs` |
| `editor.vim_mode` | Desktop only (TUI ignores) | `crates/outl-desktop/src-tauri/src/settings.rs` |
| `editor.font_size` | Desktop only | `crates/outl-desktop/src-tauri/src/settings.rs` |
| `calendar.timezone` | Every client at boot, via `outl_actions::clock::init` (resolves the IANA name once into the process-wide clock) | `crates/outl-tui/src/runtime.rs`, `crates/outl-cli/src/main.rs`, `crates/outl-desktop/src-tauri/src/lib.rs`, `crates/outl-mobile/src-tauri/src/lib.rs` |
| `sync.transport` / `sync.relay_url` | TUI peer-sync wiring | `crates/outl-tui/src/actions/lifecycle/peer_sync.rs::wire_sync_transport` (config-driven; replaces the `OUTL_IROH=1` env gate) |
| `tui.mouse_capture` | TUI only | `crates/outl-tui/src/runtime.rs` (conditionally emits `EnableMouseCapture` and arms the `Event::Mouse` branch) |
| `display.backlinks_order` | TUI at boot (`runtime.rs`, applied post-construction); GUI clients on every `build_page_view` call | `crates/outl-tui/src/runtime.rs`, `crates/outl-tauri-shared/src/helpers.rs::build_page_view` (desktop + mobile share this reader) |
| `assets.max_bytes` | Every file-import path: CLI `outl asset add`, MCP `outl_asset_add`, desktop/mobile "Attach file" + drag-drop, TUI `/upload` + paste-a-path | `crates/outl-cli/src/cmd/asset.rs`, `crates/outl-tauri-shared/src/commands/asset.rs`, `crates/outl-tui/src/commands/builtins/asset.rs` + `crates/outl-tui/src/actions/paste.rs` (all route through `outl_actions::asset::import_asset(root, source, max_bytes)`) |

Update this table whenever a new reader appears.

## What this crate does NOT do

- ❌ Parse the **per-workspace** `<workspace>/.outl/config.toml`.
  That belongs to `outl-cli::cmd::init` and the workspace-open path; it's a different schema (per-device `actor_id`, workspace-only overrides).
- ❌ Hold the actor ULID.
  Lives next to `config.toml` as a separate file, owned by the consumer.
- ❌ Provide a settings UI / form schema.
  Each client renders its own.
- ❌ Validate semantic correctness (does the theme name exist? is the path readable?).
  Validation is the consumer's job — this crate just round-trips bytes.

## Verify before "done"

```bash
cargo fmt --all
cargo clippy -p outl-config --all-targets -- -D warnings
cargo test -p outl-config
```

If you touched the schema, also smoke the readers:

```bash
cargo test -p outl-tui      # runtime::resolve_theme tests
cargo test -p outl-desktop  # settings round-trip tests
```
