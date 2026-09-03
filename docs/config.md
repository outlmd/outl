# Configuration

outl reads two TOML files at launch.
They are read in this order; the second can override fields from the first.

| Layer | Path | Scope | Written by |
|---|---|---|---|
| **Global** | `~/.config/outl/config.toml` | The user's machine — every workspace, every client | The desktop app's Settings modal; you can also edit it by hand |
| **Per-workspace** | `<workspace>/.outl/config.toml` | One workspace only | `outl init` (seeds the legacy actor id); hand-edit for the rest |

The path layout is **XDG-style on every OS — including macOS**.
outl is keyboard-first and CLI-friendly; the macOS-native `~/Library/Application Support/…` location would split the TUI and desktop into two config files for no real benefit.

The reader for both files is the **`outl-config`** crate (`crates/outl-config/`).
TUI and desktop import the same module so a field can't drift between clients — extending the schema in one place lights up in both.

---

## Global config (`~/.config/outl/config.toml`)

Every field is optional; missing values fall back to the documented default.
A malformed file is logged and replaced with defaults rather than refused to boot — preferences aren't worth blocking the app on.

```toml
# ~/.config/outl/config.toml — full example with every supported field

[workspace]
# Absolute path to the workspace the user last opened. The desktop
# writes this on every `set_workspace` call; the TUI / CLI read it
# when no `--workspace` flag and no positional path is given.
last = "/Users/me/iCloud/outl"

[theme]
# Palette preset name from `outl_theme::PRESETS`.
# Choices: "outl" (default), "default-dark", "light", "dracula",
#          "solarized-dark", "nord", "monokai".
preset = "outl"

[editor]
# Vim-style modal bindings (Normal / Insert / Visual). Defaults to
# `true` — outl is keyboard-first. The desktop honours this; the TUI
# is vim-style by definition and ignores the flag.
vim_mode = true

# Outline font size in pixels (desktop only — the TUI uses your
# terminal font).
font_size = 15

[calendar]
# Optional IANA timezone name for the journal date + status-line clock.
# Omit (the default) to use the operating system's local timezone.
# Set it when the OS clock runs in the wrong zone — containers and
# Chrome OS Crostini report UTC regardless of where you are (issue #107).
timezone = "Europe/London"

[sync]
# Which transport moves the per-actor op log between devices.
#   "iroh" (default) — direct P2P over QUIC (hole punching + relay).
#   "file"           — iCloud Drive / shared filesystem. Zero infra opt-out.
# Missing [sync] falls back to "iroh" — P2P is outl's primary sync.
transport = "iroh"

# Optional relay URL for the "iroh" transport. Empty (or omitted)
# means use outl's default relay (use1-1.relay.avelino.outl.iroh.link). Set to override with
# your own iroh-relay. Ignored by the "file" transport.
relay_url = ""

[display]
# Direction of the backlinks ("Linked from") list. "newest" (default)
# puts the most recently referenced page at the top; "oldest" flips
# it. A pure display preference — never converges between devices.
backlinks_order = "newest"

[assets]
# Maximum size, in bytes, of a single uploaded file (`outl asset add`,
# the desktop/mobile "Attach file" action). 0 = unbounded. Default is
# 100 MiB. The `assets/` directory itself is fixed at
# `<workspace>/assets/` and is not configurable.
max_bytes = 104857600

[reminders]
# Whether this device turns `remind::` rules into OS notifications.
# On by default: writing `remind::` on a block is already the opt-in,
# and a device that never gets a rule never fires. Set false to keep
# the rules tracked and listed without ever being interrupted.
# Device-local — the rule itself and a snooze converge through the op
# log, this does not. See reminders.md.
enabled = true
# A fire landing inside this window is pushed to the window's end,
# never dropped. Omit (or leave empty) for no quiet hours. A window
# that wraps midnight is the normal case.
quiet_hours = "22:00-07:00"
```

> **Why `[calendar] timezone` exists at all:** [RFC 0107](rfcs/0107-page-identity.md) — the journal's date decides its slug, so "what day is it" is an identity question, not a display preference.

### Field reference

#### `[workspace]`

| Field | Type | Default | Read by | Effect |
|---|---|---|---|---|
| `last` | absolute path | _none_ | desktop, TUI, CLI | Where the next `outl` (with no args) opens. The desktop persists this on every workspace switch. If the path no longer exists, every reader silently falls through to its next fallback (CLI: cwd; desktop: workspace picker). |

#### `[theme]`

| Field | Type | Default | Read by | Effect |
|---|---|---|---|---|
| `preset` | string | `"outl"` | TUI, desktop, mobile | The light side of the pair, and the only preset used when `mode = "light"`. Unknown names fall through to `outl`. The desktop Settings modal writes this field via `Settings.theme`. |
| `preset_dark` | string, optional | _none_ (falls back to `preset`) | TUI, desktop, mobile | The dark side of the pair, used when `mode = "dark"` or when `mode = "auto"` resolves dark. `None` resolves to `preset` — see the backwards-compatibility note below. |
| `mode` | `"light"` \| `"dark"` \| `"auto"` | `"auto"` | TUI, desktop, mobile | Which side of the pair to render. `"light"` and `"dark"` always resolve to their named side. `"auto"` follows the OS appearance setting — **except on the TUI**, which cannot read it and always resolves to the dark side (see [theming.md → Light / dark pair and `mode`](theming.md#light--dark-pair-and-mode)). |

**Backwards compatibility:** a config with only `preset` set behaves exactly as it did before `preset_dark` and `mode` existed.
`preset_dark` defaults to `None`, which resolves to `preset`, so `auto` alternates between the same preset on both sides — byte-identical to a config that only ever had one theme.
Setting a second preset in `preset_dark` is what makes `mode` do anything.
Pinned by `a_config_with_only_preset_behaves_exactly_as_before` (`crates/outl-config/src/schema.rs`).

**Both GUI clients now resolve the pair.**
Desktop and mobile both call the shared `get_theme_config` command (`outl-tauri-shared::commands::theme`), which resolves `[theme]` server-side (`preset_dark` already falls back to `preset`).
Both feed the result to the shared `installTheme` (`@outl/shared/theme`) to follow `mode` / `prefers-color-scheme` the same way the TUI's `resolve_preset_name` does.
See [theming.md → Light / dark pair and `mode`](theming.md#light--dark-pair-and-mode) for the client-by-client detail.

**The desktop now writes the whole pair.**
The flat `Settings` DTO (`crates/outl-desktop/src-tauri/src/settings.rs`) carries all three fields: `theme` (`preset`), `theme_dark` (`preset_dark`), `theme_mode` (`mode`).
`save` no longer restores any of them from disk, since the modal is now their sole owner.
A config that only ever set `preset` gets an explicit `preset_dark` (equal to `preset`) written on the first modal save.
That is a one-time, behaviour-preserving change to the file, since `ThemeCfg::dark()` already resolved to the same value implicitly.

`outl doctor` warns when a configured pair has two light or two dark sides (e.g. `mode = "light"` naming a dark preset) — a misconfigured pair, not a resolution bug.

Available presets: `outl`, `outl-light`, `default-dark`, `light`, `logseq-light`, `dracula`, `solarized-dark`, `nord`, `monokai`.
See [theming.md](theming.md) for the look of each.

#### `[editor]`

| Field | Type | Default | Read by | Effect |
|---|---|---|---|---|
| `vim_mode` | bool | `true` | desktop | When `false`, the desktop drops the modal `Normal / Insert / Visual` model and only listens to OS-standard chrome chords (`⌘P`, `⌘B`, …). The TUI is vim-style by definition and ignores this. |
| `font_size` | integer (pixels) | `15` | desktop | Outline body font size. The TUI uses the user's terminal font; setting this has no effect there. |

#### `[sync]`

| Field | Type | Default | Read by | Effect |
|---|---|---|---|---|
| `transport` | `"iroh"` \| `"file"` | `"iroh"` | every client (TUI / desktop / mobile / MCP) | Which transport ships each device's `ops-<actor>.jsonl` to the others. `"iroh"` opens direct P2P QUIC connections to paired peers; `"file"` is the opt-out that relies on iCloud Drive / a shared filesystem. Missing `[sync]` defaults to iroh (P2P is the primary sync). |
| `relay_url` | string (URL) | _empty_ | TUI peer-sync wiring | iroh relay used for NAT traversal + fallback. Empty means outl's default relay (`use1-1.relay.avelino.outl.iroh.link`); set it to point at your own `iroh-relay`. Ignored when `transport = "file"`. See [relay.md](relay.md). |

#### `[snapshot]`

| Field | Type | Default | Read by | Effect |
|---|---|---|---|---|
| `enabled` | bool | `true` | TUI / desktop / mobile | Master switch for materialised-state snapshots on disk. The CLI ignores this (always off — its work is ephemeral). When `true`, `Workspace::apply` writes a snapshot every `op_threshold` ops so the next boot skips the full op-log replay. |
| `op_threshold` | integer (ops) | `10_000` | TUI / desktop / mobile | How many ops between snapshot writes. Lower = faster boot, more disk churn; higher = less churn, slower boot. |

#### `[storage]`

| Field | Type | Default | Read by | Effect |
|---|---|---|---|---|
| `lru_cap` | integer (ops) | `20_000` | TUI / desktop / mobile | Maximum number of ops held in `JsonlStorage`'s in-memory cache. `0` keeps the legacy unbounded behaviour (every op resident forever). Any positive value caps the cache so RSS stays roughly constant regardless of workspace history; cold ops stay addressable through the per-actor offset index (`ops-<actor>.idx`). Mobile pins this to `min(lru_cap, 5_000)` to stay well under iOS jetsam. See [RFC #137](https://github.com/outlmd/outl/issues/137). |

#### `[tui]`

| Field | Type | Default | Read by | Effect |
|---|---|---|---|---|
| `mouse_capture` | bool | `false` | TUI only | When `true`, the TUI captures mouse events: the scroll wheel moves the outline selection, a click selects the block under the pointer, and dragging selects a range that is copied as clean outl markdown to the OS clipboard on release. Default is `false` because capturing the mouse disables the terminal's own text-selection (Shift-drag). The keyboard yank (`yy` / `Y` / Visual `y`) always writes to the clipboard regardless of this flag. |

#### `[display]`

| Field | Type | Default | Read by | Effect |
|---|---|---|---|---|
| `backlinks_order` | `"newest"` \| `"oldest"` | `"newest"` | every client (TUI, desktop, mobile) | Sort direction for the backlinks ("Linked from") list. `"newest"` puts the page holding the most recently created referencing block at the top; `"oldest"` flips it. Blocks within a page always keep document order. The TUI toggles it with `Ctrl+O` (see [shortcuts.md](shortcuts.md)); the desktop and mobile apps expose a direction button in the backlinks header. A pure display preference — it never converges between devices (issue #142). |

#### `[assets]`

> **Why the bytes live in `assets/` and never in the op log:** [RFC 0202](rfcs/0202-file-assets.md).

| Field | Type | Default | Read by | Effect |
|---|---|---|---|---|
| `max_bytes` | integer (bytes) | `104857600` (100 MiB) | every client that imports a file (CLI `outl asset add`, MCP `outl_asset_add`, desktop/mobile "Attach file") | Upper bound on a single uploaded file. A file over the cap is rejected before it is copied into `<workspace>/assets/`. `0` means unbounded. The `assets/` directory location itself is fixed, not configurable. |

#### `[reminders]`

| Field | Type | Default | Read by | Effect |
|---|---|---|---|---|
| `enabled` | boolean | `true` | every client's delivery loop | Whether this device turns `remind::` rules into OS notifications. Defaults on because `remind::` on a block is already the explicit opt-in, and a device with no rules never fires (so it never prompts for permission either). Set `false` to keep the rules parsed and listed while never being interrupted by them. Read by the desktop and mobile poll loops and by the TUI's event-loop tick. |
| `quiet_hours` | string `"HH:MM-HH:MM"` | unset | desktop + mobile schedulers | A fire landing inside this window is **pushed to the window's end**, never dropped. Wraps midnight (`"22:00-07:00"`) and same-day windows (`"13:00-14:00"`) both work. An unparseable value is ignored rather than failing the config load — a typo here must never keep the app from opening. A fire pushed past its rule's own `until` is over, not deferred to the morning. |

Both are **device-local on purpose**: quiet hours are a property of *this* phone or laptop, not of the workspace.
The rule itself and a snooze converge through the op log; see [reminders.md](reminders.md).

#### `[backup]`

| Field | Type | Default | Read by | Effect |
|---|---|---|---|---|
| `enabled` | boolean | `true` | the TUI's background snapshot pass | Take periodic local git snapshots of the workspace. Costs nothing on an unchanged workspace (no diff → no commit) and degrades to a logged warning where there is no `git` on `PATH`. |
| `interval_minutes` | integer | `30` | the TUI's background snapshot pass | Minimum minutes between automatic snapshots. A **floor, not a schedule** — the pass wakes on this cadence and commits only when at least this long has passed since the newest snapshot, so a burst of edits never becomes a burst of commits. The elapsed time is read back out of the git history, so it survives a restart. |

Backups default **on** for the same reason reminders do.
The failures they catch — a bad projection, an `outl import` aimed at a workspace that already had pages, a page deleted with the app then closed — are ones you discover *after* the moment when you could have enabled a safety net.

Which clients run the automatic pass today, and the rules a client that wires it up must keep, live in [clients.md → Automatic backups](clients.md#automatic-backups).

They are **device-local**, and the repository lives **outside the workspace** — under `~/.config/outl/backups/<name>-<hash>.git`, with the workspace as git's `--work-tree`.
Two reasons:

- The workspace is a **sync surface** on every transport except iCloud.
  A `.git/` inside it means Syncthing / Dropbox / a shared mount replicate git's object store, index and lock files under eventual consistency.
- If you already keep your notes in your own git repo, outl must not touch it.
  It doesn't: your staging area, your branch, your hooks and your `info/exclude` are never read or written, and no snapshot is ever committed into your history.

`outl backup list|restore` find the repository automatically; a workspace you **move or rename** starts a fresh history (the old one stays on disk).

What is captured: `ops/` (the source of truth), `pages/`, `journals/`, `templates/`, `assets/`, and `.outl/config.toml`.
What is excluded: caches the next boot rebuilds — `.outl/snapshots/`, `*.idx`, lock files, `*.tmp`.

**A `.gitignore` in your workspace cannot exclude the captured paths.**
Work-tree ignore rules beat the backup repo's own exclusions, so a `*.jsonl` line used to drop the op log out of every snapshot while the CLI still printed commit ids.
Those paths are now staged with `git add --force`, and every snapshot **verifies that each `ops/*.jsonl` on disk is in the commit** before reporting success — a history missing the source of truth is reported as an error, not a backup.

> The iroh transport also reads `~/.outl/identity.key` (this device's ed25519 keypair, per-machine) and `<workspace>/.outl/peers.json` (the paired-device list, per-graph).
> Those are managed by `outl peer …`, not by this config file — see [sync.md → iroh transport](sync.md#transport-2-iroh-p2p).

---

## Per-workspace config (`<workspace>/.outl/config.toml`)

Written by `outl init`; carries the device's per-workspace identity and (optionally) workspace-scoped overrides.

```toml
# Per-workspace config — auto-generated by `outl init`, can be
# hand-edited.

[workspace]
# LEGACY actor id (a ULID), seeded at `outl init`. This is NOT where a
# device reads the actor it writes under — that lives outside the
# workspace, in the device store (see below). Exactly one device adopts
# this value on first open; every other device mints its own.
actor_id = "01HKZX9YBPDC5XJZ3R8K2QGM7E"

# Machine id of the device that owns `actor_id`. Stamped when this
# file is CREATED, by the device that created it. Every other device
# reads it as "taken" and generates a fresh actor rather than sharing
# this one. Absent on a workspace created before the device store
# existed — and then nobody adopts `actor_id` at all.
actor_claimed_by = "01HKZXA1M4N7QF0V6T2WSD9YB3"

# Persistent storage backend. JSONL (one append-only `ops-<actor>.jsonl`
# per device) is the ONLY backend, so this is almost always omitted.
# Omitting it means "jsonl" — leave it out unless you have a reason.
storage = "jsonl"

[theme]
# Workspace-only override. When set, takes precedence over the
# global `[theme] preset` while you're inside this workspace.
preset = "monokai"
```

### Where the actor id actually lives

`.outl/config.toml` sits **inside** the workspace, so Syncthing, Dropbox, NFS, a shared volume and `git clone` all replicate it.
A device actor read from here would therefore be the *same* on two machines, both would append to one `ops-<actor>.jsonl`, and the loser's ops would disappear with no error.
(iCloud Drive never exposed this because it drops dot-prefixed paths, so `.outl/` never travelled.)

The actor a device writes under lives in the **device store** instead — a directory outside every workspace:

```text
$OUTL_DEVICE_DIR, else $XDG_CONFIG_HOME/outl, else ~/.config/outl/
├── machine-id                  # this device's id + its host binding
├── actor                       # device-wide actor (desktop + mobile)
└── actors/
    ├── <workspace-id>          # that workspace, at the directory it was bound to
    └── <workspace-id>.<hash>   # a second copy of it on this same device
```

Nothing to configure: it is created on first open.
Copying `.outl/config.toml` between machines is safe — the second machine sees `actor_claimed_by` naming someone else and mints its own actor.
Copying the whole workspace *directory* on one machine is safe too: the binding records which directory it belongs to, so a second live copy forks while a plain move or rename keeps its actor.

The store is not beyond reach of a *whole-`$HOME`* clone (Migration Assistant, a Time Machine restore, a VM image, chezmoi, NFS), so `machine-id` is bound to a hash of an OS identifier of the physical machine and reminted when that changes.
On iOS no such identifier is reachable, so a restored device backup is a known gap.
Details in [storage.md → Where the actor id lives](storage.md#where-the-actor-id-lives--outside-the-workspace).

### `[workspace] storage` and peer sync

| Field | Type | Default | Read by | Effect |
|---|---|---|---|---|
| `storage` | `"jsonl"` | `"jsonl"` (when absent) | TUI | Selects the persistent backend. JSONL is the only one, so the key is normally absent. The TUI treats **absent OR `"jsonl"`** as a shareable workspace and starts its peer-sync threads (the iroh transport + the filesystem poller); only an explicit non-`jsonl` value turns them off. |

This matters because a workspace **created by a GUI client or P2P sync** (not by `outl init`) seeds its `config.toml` without a `storage` line.
The TUI must read that absence as the jsonl default, or it would open such a workspace with **no peer sync at all**.
The symptom was the TUI never receiving a paired phone's edits — the desktop, which the phone had already reached over iroh, wrote those ops to the shared `ops/`, but the TUI never started a poller to notice.
Storage is a trait with one persistent impl (`JsonlStorage`); the `storage` key is a legacy selector from when a second backend was on the table, kept only so an explicit opt-out is still expressible.

---

## Precedence chains

### Workspace path

When you type `outl` (no args):

1. Subcommand-positional path (`outl page get … <PATH>`).
2. Global flag `--workspace <DIR>`.
3. `[workspace] last` from `~/.config/outl/config.toml`.
4. Current working directory (the `cd ~/notes && outl` fallback).

A path from `config.toml` that no longer exists on disk is skipped silently and the chain falls through to the next step.

### Theme

When the TUI / desktop decides which palette to render:

1. `--theme <preset>` CLI flag (TUI only).
2. Per-workspace `[theme] preset` from `<workspace>/.outl/config.toml`.
3. Global `[theme] preset` from `~/.config/outl/config.toml`.
4. Built-in default — `outl`.

Unknown preset names fall through to the next step rather than erroring.

---

## Editing safely

The TOML reader (`outl-config::load`) is **forgiving by design**:

- Missing file → defaults, no warning (first launch is normal).
- Malformed TOML → defaults + a `tracing::warn` log line, the app boots normally.
- Unknown fields → ignored.
  Older binaries reading a newer config don't choke; you can add fields ahead of time.
- Partial schema (e.g. only `[theme]` populated) → other sections fall back to their per-section `Default`.

Saving (`outl-config::save`) writes atomically — the new content lands in `config.toml.tmp` and the file is renamed on top.
A crash mid-write never leaves a truncated config.

---

## Migrating from earlier versions

The desktop briefly stored its settings as JSON at `~/Library/Application Support/app.outl.desktop/settings.json` (and the actor at the same directory).
That path is no longer read.
If you upgrade from one of those builds:

- The desktop picks up `~/.config/outl/config.toml` (creates it on first save).
- The actor ULID at `~/.config/outl/actor` is generated fresh on first run; your local op log keeps writing under the new id.
- Your workspace's `ops/` directory is unchanged — only the **client's** identity rotates, not the workspace's history.

If you want to preserve the old actor id, copy it from the old path into `~/.config/outl/actor` before launching the desktop.
