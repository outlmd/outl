# RFC 0357 — A managed config file is owned by the package manager, and no client rewrites it

| | |
|---|---|
| **Status** | Accepted |
| **Issue** | [#356](https://github.com/outlmd/outl/issues/356) |
| **PR** | [#357](https://github.com/outlmd/outl/pull/357) |
| **Date** | 2026-10-10 |
| **Reference doc** | [`docs/config.md`](../config.md) owns the `managed` directive; [`docs/nix.md`](../nix.md) owns the home-manager contract |
| **Invariant** | `crates/outl-config/CLAUDE.md` → Managed config — the save gate lives in `outl_config::save`, so every client inherits it |
| **Guarded by** | `managed_is_the_first_key_in_serialised_toml`, `managed_round_trips_and_defaults_false`, `save_honours_the_managed_directive` (`crates/outl-config/src/lib.rs`) |

## Why

A Nix/home-manager install renders `~/.config/outl/config.toml` as a symlink
into the Nix store. Every client writes that file by atomic rename — write a
temp file, then replace the target — and replacing a symlink with a rename
detaches it: the user now has a plain file where the generation graph expects
a store link, the next activation aborts with "file exists and cannot be
overridden", and the config drifts from the declaration with nobody able to
say when.

The writes are small and constant: `workspace.last` after every workspace
open, the theme toggle, the backlinks direction. A managed install survives
zero minutes before the first one fires. The flake alone was therefore not
enough to close #356 — declarative management needs the clients to *know*
they do not own the file.

## What we chose

A top-level `managed` boolean in `config.toml` (default `false`), read off
the **on-disk** file, not off a caller-constructed `Config`:

- `outl_config::save` returns `Ok` immediately when the flag is set — the
  gate is in the core save path, so the CLI, the TUI, the desktop, and any
  future client inherit it without touching their own code.
- `OUTL_CONFIG_MANAGED=1` is an escape hatch for containers and tests that
  cannot rewrite the file themselves, mirroring the existing
  `OUTL_DEVICE_DIR` override.
- The home-manager module emits `managed = true` by default;
  `crates/outl-config/CLAUDE.md` states the rule where the next save-path
  editor will hit it.
- The desktop Settings modal reads `managed` off the DTO the backend sends
  (never re-derives it), shows a notice naming `programs.outl.settings`,
  and disables Save. The Rust side pins the field: a hand-edited
  `managed = true` in `Settings` sent by the webview is overwritten from
  the authoritative on-disk value before any write.
- `managed` must serialize as the **first** key of the file:
  `toml::to_string_pretty` emits scalars before tables, and a top-level key
  emitted after `[workspace]` re-parses as `[workspace].managed` — the
  directive would vanish silently. The field order is pinned by a test
  that checks serialization position, not by a comment.

## Why not the alternatives

- **Detect a symlink at write time, no new key.** True symlinks are caught,
  but home-manager on some setups writes a real file (with
  `symlinkSupport = false`, future versions may change this again); and the
  heuristic misfires on users who hand-symlink their config and *do* want
  in-app persistence. The flag states intent; a symlink states geometry.
- **Per-client gates.** Three clients duplicating the check is invariant 8's
  anti-pattern — `outl-config` is the owner of the save path, the gate
  belongs there.
- **An environment variable only.** Nix activations are systemd user units
  and shell sessions with different environments; a directive carried inside
  the file travels with the file, which is the artifact home-manager owns.
- **Letting clients write and telling home-manager `force = true`.** Force
  reclaim silently deletes user edits on every activation — trading a loud
  abort for silent data loss, the exact shape RFC 0210 warns about.

## The opposite direction

What this change makes worse, stated plainly: on a managed install,
in-app persistence is off. The theme picker keeps previewing live, the
window-decorations toggle keeps applying, but a theme chosen in Settings
does not survive a restart — it must be declared in Nix. That is the
contract of a declarative system, and the modal notice says so in one
sentence. A user who does not want this sets `programs.outl.settings.managed =
false` and keeps full client-side persistence; the default favours the
declarative user, because that is who installed via home-manager.

The mirrored case — "can a client that reads `managed` from disk be tricked
by a caller-supplied value?" — is the desktop DTO: it reconstructs `Config`
from a flat `Settings` shape, so `managed` on a caller's `Config` is ignored
and the on-disk read wins. `save_honours_the_managed_directive` covers the
gate; `managed_is_the_first_key_in_serialised_toml` covers the re-parse
trap.

## How it cannot regress

1. `managed_is_the_first_key_in_serialised_toml` — fails if `managed` moves
   below a table header in serialization (the silent-drop trap).
2. `managed_round_trips_and_defaults_false` — fails if the key stops
   round-tripping through parse, so an existing managed file becomes
   unmanaged.
3. `save_honours_the_managed_directive` — fails if `save()` starts writing
   a managed file again, which detaches home-manager's symlink and breaks
   the next activation.
4. The exhaustive `Config` construction in the tests fails to compile if
   the field is removed; the pin in `update_settings` (desktop) fails if
   the webview regains the ability to forge the flag.

## Scope

Not covered here:

- **The desktop's `settings.json` and the device `actor` key.** They share
  the config directory (`outl_config::config_dir`) with `config.toml` and
  the desktop rewrites them freely; home-manager does not own them, so no
  gate applies.
- **Sync state (`~/.outl` device dir, `<root>/.outl/peers.json`).** Device
  keys and peer lists live outside the config file, are created locally,
  and are never rendered by home-manager.
- **Nix packaging.** `docs/nix.md` owns install/module usage; this RFC owns
  why the save gate exists, which outlives the flake.
