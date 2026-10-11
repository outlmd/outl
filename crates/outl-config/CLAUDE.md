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
Every read goes through [`load`] / [`load_from`] (or [`load_result`] / [`load_result_from`], which also say *where the config came from*); every write goes through [`save`] / [`save_to`].
Bypassing this crate is how schema drift starts — and it is also how the guard below gets bypassed.

The desktop's `settings.rs` is the canonical adapter pattern: a flat wire-format struct for the frontend, converted via `From` impls in and out of `outl_config::Config`.
If a new client needs a different shape on the wire, do the same — adapt, don't fork the reader.

## The `managed` directive — a no-op save

`Config` has one field that is **not** a user preference: the top-level `managed` bool.
When it is set (or `OUTL_CONFIG_MANAGED` is in the environment), [`save`] returns `Ok(())` **without writing the file**.
That is what lets Nix/home-manager own `config.toml` end to end: the file stays a symlink into the Nix store, and no client detaches it by persisting `workspace.last`, the theme toggle, or the backlinks direction.

Three rules to keep it working:

1. **`managed` is the first field of `Config`.**
   `toml::to_string_pretty` emits scalar fields before tables, so `managed` lands before `[workspace]`.
   As any later field it would re-serialize *after* a table and re-parse as `[workspace].managed`, silently dropping the directive.
   `managed_is_the_first_key_in_serialised_toml` pins this.
2. **The gate reads the on-disk verdict via [`managed`], never the passed `&Config`.**
   The desktop reconstructs a fresh `Config` from its flat DTO (which does not carry the flag), and the file the package manager owns is the only authority on whether it is owned.
   So `managed()` = `OUTL_CONFIG_MANAGED` ∪ `load().managed`, checked inside `save` before touching disk.
3. **`save_to` stays ungated.**
   It is the explicit-path primitive (tests, and any future non-default path); only the default-path `save` honours the directive.

`[workspace] last` freezing under a managed config is the accepted cost — the desktop can no longer store the last-opened workspace.
`docs/nix.md` → "Declarative config" tells the user to pin it declaratively.

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
managed = false                   # top-level: owned by Nix/home-manager → save() is a no-op (default false)

[workspace]
last = "/Users/me/iCloud/outl"   # absolute path; optional

[theme]
preset = "outl-light"             # name from outl_theme::PRESETS; the light side of the pair
preset_dark = "outl"              # optional; dark side. Omit = falls back to `preset` (pre-RFC-0022 behaviour)
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
icons = "emoji"                  # "emoji" (default) | "nerd-font"
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
`TuiCfg::mouse_capture` (default `false`) is read by the TUI at boot in `runtime/mod.rs` to decide whether to call `EnableMouseCapture` and listen for `Event::Mouse`; the desktop ignores this section entirely.
`TuiCfg::icons` (default `emoji`) is read by the TUI at boot in `runtime/mod.rs`; `nerd-font` is an explicit opt-in for terminals with a Nerd Font installed.
`DisplayCfg::backlinks_order` is a [`BacklinksOrder`] enum (`Newest` | `Oldest`, serde `lowercase`, default `Newest`) — a pure display preference, same "never converges between devices" policy as `theme.preset` (root `CLAUDE.md` invariant #7).
`ThemeCfg` (RFC 0022) models a light/dark preset *pair*, not a single preset.
`preset` is the light side, `preset_dark: Option<String>` is the dark side, and `mode` is a [`ThemeMode`] enum (`Light` | `Dark` | `Auto`, serde `lowercase`, default `Auto`).
A wholly missing section defaults to the brand pair `outl-light` / `outl`.
`ThemeCfg` uses custom deserialization to distinguish that from a present legacy section: an explicit `preset` with no `preset_dark` leaves the dark side unset.
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
| File missing | Returns `Config::default()` silently, `ConfigSource::Missing`. First launch is normal. |
| File present, empty | Returns `Config::default()`, `ConfigSource::Parsed` — every field has a serde default, so `""` is a valid config. **Deliberate, and the reason the write path is where zero-byte files are prevented** — see below. |
| File present, malformed TOML | Returns `Config::default()` + `ConfigSource::Unreadable(detail)` **+ `tracing::warn!`**. Never panics. |
| File present, unreadable (permissions, a directory in the way) | Same as malformed: `Unreadable`. We do not have these bytes; why is not the point. |
| Unknown field | Ignored. Older binary survives a newer config. |
| Partial section (e.g. only `[theme]` populated) | Other sections fall back to their per-section `Default`. |
| `save()` | No-op when `managed()` reports external ownership; otherwise an atomic write (`.config.toml.tmp.<ulid>` → rename). Creates `~/.config/outl/` if missing. Concurrent saves never publish a zero-byte config. |
| `save()` over an `Unreadable` file | **Refuses** with `SaveError::Unreadable`. Nothing is written; the file stays byte-for-byte. |

The forgiving read path is **load-bearing for UX**: a user editing TOML by hand mid-typo doesn't lose every preference; they just see defaults until the file is fixed.
Do not make load fail-fast — fail-fast belongs in the workspace itself, not in user preferences.

### The write guard (issue #284)

`load` returning defaults for a broken file is only safe if nothing writes those defaults back.
Every caller does *load → mutate one field → save*, and `save` serializes the **whole** struct, so one bad character plus one UI toggle used to replace the user's theme, `vim_mode`, timezone and `[sync] transport` with defaults — the file they could have fixed was gone.

So `save_to` re-reads the file and refuses when it is `Unreadable`.
The check lives **in `save_to`, not in the callers**: a caller that loaded hours ago, or never loaded at all, cannot answer whether the file on disk parses, and a caller that forgets is exactly the bug.
`read_at` is the single owner of the verdict, shared by the load and the guard, so the two cannot disagree.

There is **no `force` variant**, deliberately: the escape hatch is the file, which is hand-editable by design (`docs/config.md`) and still intact.
`SaveError::Unreadable`'s message names the path and the failing line, because that text is what reaches the user — the TUI status line, the desktop error toast, the mobile banner.

Two sentences, one owner each, and they answer different questions:

| Sentence | Owner | Answers |
|---|---|---|
| `settings not saved: <path> could not be read (line N: …). Fix that file, then try again` | `SaveError::Unreadable` | "why did my change not stick" |
| `<path> could not be read (line N: …) — every preference is running on defaults, and settings will not save until it parses. Nothing has overwritten the file.` | `Loaded::notice()` | "why is nothing the way I left it" |

`Loaded::notice()` is what the TUI prints on its first frame and what `outl doctor` warns with — **neither writes its own wording** (root `CLAUDE.md` invariant 12: the reason text belongs in the catalog, not the client).
A client that needs a shorter version should shorten it here, for everyone.

What is **not** covered yet: a boot-time notice in the desktop and mobile GUIs. The TUI names it on its first frame; the GUI clients only surface it when a write is attempted.

Do not "simplify" the guard into a flag the caller passes, and do not drop it because `load_result` exists — a verdict a caller may ignore is not a guard.

### The scratch name is per write, not per file

Lives in `src/atomic.rs`, which owns one question — *given that the write is allowed, how does it land?* — while `lib.rs` owns the other half (read `config.toml`, classify it, refuse to write over one it could not read).
`save_to` composes into `.config.toml.tmp.<ulid>` and publishes by rename.
The ULID is not decoration: this file has several writers by design (the TUI and the desktop app share it), and one shared scratch name is one shared **inode**.
Writer B's `File::create` truncates the body A already `fsync`ed, A's `rename` publishes those zero bytes, and B goes on writing through a descriptor that now points at the published `config.toml` while its own rename fails `ENOENT` — the user sees "could not write", and what is on disk is a zero-byte config.
Same fix and same reason as `outl_core::snapshot::scratch_path` and `outl_core::storage::sidecar`'s `tmp_path_for`; do not invent a third shape.

**A zero-byte `config.toml` is the worst possible landing spot for this crate, and that is why the fix is here and not in a fourth `ConfigSource` verdict.**
Every field carries `#[serde(default)]`, so `""` deserializes into a whole `Config`: the file is `Parsed`, the #284 write guard finds nothing to refuse, and the next save writes defaults over the user's theme, `vim_mode` and `[sync] transport`.

**And calling zero bytes `Unreadable` would not be that fix.**
An empty `config.toml` is a legitimate config meaning "all defaults" — `touch ~/.config/outl/config.toml` is how a user starts one by hand, and the behaviour table above has said so since this crate existed.
Refusing to save over it would lock that user out of every settings toggle with a message telling them to repair a file that is not broken: a guard turned into a wall (root `CLAUDE.md` invariant 11).
So the length stays uninterpreted and the cause is removed instead.

A unique name hands back one question in exchange (root `CLAUDE.md` invariant 9 — what cleans it up?): nothing recycles the name any more, so a process *killed* between the `create` and the `rename` leaves its scratch forever.
`TempFile` covers every in-process exit path; `sweep_stale_scratch` covers the one it cannot, unlinking scratch siblings older than 24h on a later save.
The age is a margin, not a deadline: unlinking a *live* writer's scratch would fail its rename with the very `ENOENT` this is here to stop producing.

Pinned by `tests/concurrent_save.rs` (end to end) and `every_scratch_name_is_its_own` + `the_sweep_takes_only_scratch_files_nobody_is_writing` (unit).

## Adding a field

1. Add the field to the relevant struct in `src/schema.rs` with `#[serde(default)]` (or a per-type `Default` impl).
2. Update the example in `src/lib.rs`'s module doc.
3. Update `docs/config.md` — the user-facing schema table.
4. Update `crates/outl-cli/CLAUDE.md` and/or `crates/outl-desktop/CLAUDE.md` and/or `crates/outl-tui/CLAUDE.md` if a new client now reads the field.
5. Wire the reader in the consuming crate (`outl-tui/src/runtime/mod.rs` for TUI, `outl-desktop/src-tauri/src/settings.rs` for desktop).
6. Add a `tests` case covering the partial-TOML path (only the new section populated) to confirm the default still applies.

If the field is **per-workspace** (not global), it doesn't belong here — it belongs in `<workspace>/.outl/config.toml`, written by `outl-cli`'s `init` command.
If the field **must converge between devices**, it doesn't belong in TOML at all — it goes through the op log (root `CLAUDE.md` invariant #7).

## Where each field is read

| Field | Reader | File |
|---|---|---|
| `workspace.last` | TUI/CLI fallback in `resolve_path`; desktop on boot | `crates/outl-cli/src/main.rs::resolve_path`, `crates/outl-desktop/src-tauri/src/lib.rs::run` |
| `theme.preset` | TUI palette resolver; desktop settings | `crates/outl-tui/src/runtime/preset.rs::resolve_theme`, `crates/outl-desktop/src-tauri/src/commands/theme.rs` |
| `editor.vim_mode` | Desktop only (TUI ignores) | `crates/outl-desktop/src-tauri/src/settings.rs` |
| `editor.font_size` | Desktop only | `crates/outl-desktop/src-tauri/src/settings.rs` |
| `calendar.timezone` | Every client at boot, via `outl_actions::clock::init` (resolves the IANA name once into the process-wide clock) | `crates/outl-tui/src/runtime/mod.rs`, `crates/outl-cli/src/main.rs`, `crates/outl-desktop/src-tauri/src/lib.rs`, `crates/outl-mobile/src-tauri/src/lib.rs` |
| `sync.transport` / `sync.relay_url` | TUI peer-sync wiring | `crates/outl-tui/src/actions/lifecycle/peer_sync.rs::wire_sync_transport` (config-driven; replaces the `OUTL_IROH=1` env gate) |
| `tui.icons` | TUI chrome icon set | `crates/outl-tui/src/runtime/mod.rs` |
| `tui.mouse_capture` | TUI only | `crates/outl-tui/src/runtime/mod.rs` (conditionally emits `EnableMouseCapture` and arms the `Event::Mouse` branch) |
| `display.backlinks_order` | TUI at boot (`runtime/mod.rs`, applied post-construction); GUI clients on every `build_page_view` call | `crates/outl-tui/src/runtime/mod.rs`, `crates/outl-tauri-shared/src/helpers.rs::build_page_view` (desktop + mobile share this reader) |
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
