# CLAUDE.md — outl-theme

Shared palette definitions consumed by every outl renderer:

- **`outl-tui`** — maps each hex into `ratatui::style::Color` + the modifier (`BOLD`, `UNDERLINED`, `ITALIC`) the surface always wants.
- **`outl-desktop`** — writes each field as a CSS custom property on `<html>` (e.g. `--color-outl-accent: #a78bfa`); Tailwind class utilities reference them at render time.

One palette, two render strategies, **zero forking**.

> **The design specification is [`DESIGN.md`](../../DESIGN.md).**
> This file is the crate's contract — how `Palette` is built, tested and consumed.
> `DESIGN.md` is what the colours *mean*: every role, the theming resolution, spacing, the component inventory, and the live exceptions named so none of them is cited as precedent.
> A new token or preset touches both.

## Why this crate exists

The TUI used to own its own `Theme` struct (`outl-tui/src/theme.rs`) with hardcoded `ratatui::Color` variants per preset.
The desktop, born later, needed the same colors — and copy-pasting hex literals into TS was a guarantee that "Dracula" on the TUI would drift from "Dracula" on the desktop the first time anyone tweaked a shade.
This crate is the source of truth: the TUI's `theme::from_palette` now derives from here, and the desktop's `commands::theme` ships the palette to the frontend over the Tauri wire.

## Hard rule

**No client hardcodes hex values for a preset.**
Every preset is a `themes/<family>.toml` and gets pulled by name via [`by_name`] or by iterating [`preset_names`].
A new client that paints something **imports `outl_theme`** — it does not redefine "what dracula looks like".

If a client needs a hue this crate doesn't expose yet, **add the field here first** (every preset, plus the `Palette` struct), then consume it.
Splitting the hue local to one client is how "the link blue on the desktop looks wrong vs the TUI" bugs are born.

## Dependency policy

This crate is **deliberately dependency-light**: `serde` (the Tauri wire), `toml` (the theme format) and `thiserror` (the load errors). No renderer crate.
Do not add `ratatui` here — the TUI converts in its own crate.
Do not add CSS / Tailwind tooling — the desktop converts in its own crate.
Do not add `outl-config` either: this crate owns the theme *format*, `outl-config` owns *where the user's themes live* ([`outl_config::theme_dir`]), and the client joins the two. That split is what keeps the directory a parameter instead of something this crate resolves from `XDG_CONFIG_HOME` — see "The user's directory is a parameter" below.
This is what lets every other crate cheaply depend on us.

## One format, two sources

A theme is a `.toml` file: `[dark]` and `[light]` sections, one per family, every name derived from the filename and the section.

- **Built-ins** live in `themes/` and are `include_str!`'d into the binary. outl ships as a binary, so a built-in theme cannot be a file the user has to have.
- **The user's own** live in `~/.config/outl/themes/` and are read at runtime.

They are the **same format, parsed by the same code** ([`spec::parse_family`]), and that is the whole reason the built-ins are TOML rather than Rust. Two formats would mean two parsers, two validators, and an `extends = "dracula"` in a user's file with no way to reach the Rust `fn` it names.

What the format derives rather than accepts:

| Written | Derived |
|---|---|
| the filename (`gruvbox.toml`) | the family name |
| `[dark]` / `[light]` | `gruvbox` (dark), `gruvbox-dark`, `gruvbox-light` |
| nothing | `name` — writing it is an error, not an override |
| nothing | `[light]` inherits `[dark]` field by field |

`extends = "<theme>"` inherits from another theme instead of the sibling, and is what a user's file uses to start from a built-in.

**The compiler no longer checks that a preset is complete**, and the replacement is `cargo test`: `spec` refuses a non-hex value, an unknown key, a written `name` and a missing field at parse time, so a malformed built-in fails CI rather than a user's launch. For a *user's* theme the same refusal reaches them instead: `outl theme list` names the file on stderr and `outl doctor` warns.

## The user's directory is a parameter

[`registry::Registry::with_user_dir`] takes the path. This crate never resolves `$HOME` or `XDG_CONFIG_HOME`.

That is root `CLAUDE.md` invariant 9's third question (*how does a test get its own copy?*) answered in the signature. `XDG_CONFIG_HOME` is process-wide, so a crate that reached for the real config directory is a crate whose tests write into the developer's own `~/.config/outl` — which already happened in 0.11.0 and cost three flaky doctor tests.

The clients join the two owners, each reporting a load failure through the channel it has:

| Client | Resolves | A broken theme file |
|---|---|---|
| `outl theme list` / `show` | `outl_config::theme_dir()` | named on **stderr** (stdout stays greppable) |
| `outl doctor` | `theme_dir` parameter | a **warning** naming the file |
| TUI | `runtime::preset::load_catalog` | `tracing::warn!` only — the palette resolves before `App` exists, so there is no toast or status line yet. **Recorded gap**, not an oversight: the two commands above are the surfaces that name it |
| desktop / mobile | `commands::theme::catalog` | `tracing::warn!` only, same reason |

## Cost

`Registry::builtin()` parses all eight files on first touch: **2.0 ms**, once per process, behind a `OnceLock`. `by_name` after that is a `BTreeMap` lookup. `with_user_dir` clones the built-ins and reads one directory: **40 µs**.

Per-theme lazy parsing was considered and dropped — it buys under 2 ms and costs a second code path.

## What this crate owns

- [`Palette`] — the struct of named hex strings (one per semantic surface).
  Field naming follows "what the surface IS", not "what it looks like" (`ref_link_fg`, not `purple_underline`).
  Two surfaces that genuinely share a style share the field.
- [`spec`] — **the single owner of what a theme file is**: the `[dark]` / `[light]` sections, name derivation, `extends`, and every refusal ([`spec::ThemeError`]).
- [`registry::Registry`] — the catalog. [`registry::Registry::builtin`] is the eight embedded families (eleven presets: `outl`, `outl-light`, `ansi`, `ansi-light`, `logseq-light`, `dracula`, `solarized`, `nord`, `monokai`, `gruvbox`, `gruvbox-light`); [`registry::Registry::with_user_dir`] adds the user's.
- The legacy name map in `src/registry.rs` (`default-dark`, `dark`, `light`, `default`) — the four spellings the derivation does not produce, kept so no existing `config.toml` breaks. **It may only ever get shorter**; a new theme never gets a row.
- [`preset_names`] — the canonical user-visible order, derived from the files rather than declared.
- [`by_name`] — case- and separator-insensitive lookup so `"Solarized Dark"`, `"solarized_dark"`, and `"solarized-dark"` all resolve. **Built-ins only** — a client that honours the user's directory asks a [`registry::Registry`].
- [`default`] / [`all`] — fallback + iterator helpers for pickers.
- `Palette::destructive` (RFC 0022) — the delete-confirmation / "cannot be undone" hue, distinct from `warn`.
  Every preset sets its own family's red; see `themes/*.toml`.
- [`Palette::is_light`] (RFC 0022) — BT.601 perceived luminance over `bg`, exposed as a Rust method (never a wire field, so it can't cross the Tauri bridge).
  Its one non-test caller today is `outl doctor`'s light/dark pairing check.
  `mode = "auto"` does not call it — the TUI's `resolve_preset_name` matches on `ThemeMode` directly and always resolves `Auto` to the dark side, and no GUI client resolves `auto` in Rust yet.
  The desktop's `color-scheme` CSS comes from `isLightHex` in `outl-frontend-shared/src/theme/palette.ts`, a hand-synced client-side copy of this same check, by design — see `docs/rfcs/0022-unified-design-tokens.md`.

## What this crate does NOT own

- ❌ Renderer-specific style flags (`BOLD`, `UNDERLINED`).
  Those live with the renderer that knows what "bold" means in its medium.
- ❌ CSS variable names.
  The desktop chooses its prefix; this crate only ships hex strings.
- ❌ **Where** the user's themes live.
  That is [`outl_config::theme_dir`]; this crate takes the directory as an argument.
  (It *does* own the format of a user's theme — a custom theme is a catalog entry, not a preference.)
- ❌ Which preset is active.
  That is `outl_config::ThemeCfg` — still a preference, still not ours.
- ❌ Color manipulation (lighten, darken).
  Add hand-tuned variants as new fields, not as runtime derivations — a runtime-derived shade looks different on a CRT than on an OLED.

## Adding a preset

**Two steps.** It used to be eight, spread over four crates.

1. Write `themes/<family>.toml` with a `[dark]` section, a `[light]` section, or both.
   Start the file with a comment citing where the hex came from — that is the only thing that makes "is this still Dracula?" answerable.
   The first side needs every field; the second only its delta.
2. Add one line to `BUILTIN` in `src/registry.rs`, in picker position.

Everything else derives. [`preset_names`], [`by_name`], the `-dark` / `-light` suffixes, the TUI's resolver, and the desktop / mobile `list_themes` + `get_theme` all read the registry.

Then the docs: `docs/theming.md`'s preset table and `docs/config.md`'s list. And regenerate the browser mock's fixture:

```sh
OUTL_UPDATE_FIXTURES=1 cargo test -p outl-theme --test presets_fixture
```

**A new preset never gets a legacy-name row**, and never a hand-written alias: if it needs a spelling the derivation does not produce, the filename is wrong.

Two things the TUI still decides for itself, and only these: `ansi` and `ansi-light` bypass `theme_from_palette` and are built from ratatui's *named* colours so the terminal's own palette shows through (`crates/outl-tui/src/theme.rs::from_palette`).

## Adding a field

A new field on `Palette` is a **coordinated change** across every consumer.
Steps:

1. Add the field to `Palette` in `src/palette.rs`.
   Name it after the surface, not the color.
2. Add a value in **every** `themes/*.toml`.
   A side that inherits (`[light]` over `[dark]`, or anything with `extends`) only needs it where it differs.
   Skip one and the load fails at `cargo test` with the field named — `spec` refuses an incomplete palette rather than defaulting it.
3. Update the TUI's `Theme::from_palette` (`crates/outl-tui/src/theme.rs`) to render the new field with its modifier of choice.
4. Update the desktop's CSS variable wiring (`crates/outl-desktop/src/styles.css` or `crates/outl-frontend-shared/src/theme/palette.ts`) — pick a `--color-outl-<field>` name and use it in the relevant Tailwind class.
5. Document the field's intent in the doc comment.

Skipping any step lights up a regression on one client and not the other — exactly the failure mode this crate exists to prevent.

## Verify before "done"

```bash
cargo fmt --all
cargo clippy -p outl-theme --all-targets -- -D warnings
cargo test -p outl-theme
```

If you touched a field or a preset, also smoke the renderers:

```bash
cargo test -p outl-tui      # Theme::from_palette tests
cargo test -p outl-desktop  # palette → CSS wire tests
```

## Cross-crate cheat sheet

| Concept | Owner | Why |
|---|---|---|
| What a theme file is (sections, `extends`, name derivation, refusals) | `outl_theme::spec` | One format for built-ins and the user's alike. |
| Hex values per preset | `outl-theme`'s `themes/*.toml` | Single source. |
| Where the user's themes live | `outl_config::theme_dir` | Path resolution is config's; the format is ours. |
| `ratatui::Color` mapping + modifiers | `outl-tui::theme::Theme::from_palette` | Terminal-only knowledge. |
| CSS custom-property names + Tailwind class wiring | `outl-frontend-shared::src/theme/palette.ts` (`applyPaletteToRoot`, exported as `@outl/shared/theme`) + each client's `src/styles.css` | DOM-only knowledge. |
| Which preset is active (per-workspace / global) | `outl-config::ThemeCfg.preset` | User preference, not a palette concern. |
| Resolving the active preset for a given run | `outl-tui::runtime::resolve_theme` / `outl-desktop::commands::theme` | Each client picks; this crate just exposes the catalog. |
