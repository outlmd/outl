# CLAUDE.md — outl-theme

Shared palette definitions consumed by every outl renderer:

- **`outl-tui`** — maps each hex into `ratatui::style::Color` + the modifier (`BOLD`, `UNDERLINED`, `ITALIC`) the surface always wants.
- **`outl-desktop`** — writes each field as a CSS custom property on `<html>` (e.g. `--color-outl-accent: #a78bfa`); Tailwind class utilities reference them at render time.

One palette, two render strategies, **zero forking**.

## Why this crate exists

The TUI used to own its own `Theme` struct (`outl-tui/src/theme.rs`) with hardcoded `ratatui::Color` variants per preset.
The desktop, born later, needed the same colors — and copy-pasting hex literals into TS was a guarantee that "Dracula" on the TUI would drift from "Dracula" on the desktop the first time anyone tweaked a shade.
This crate is the source of truth: the TUI's `Theme::from_palette` now derives from here, and the desktop's `commands::theme` ships the palette to the frontend over the Tauri wire.

## Hard rule

**No client hardcodes hex values for a preset.**
Every preset lives in `src/presets.rs` and gets pulled by name via [`by_name`] or by iterating [`PRESETS`].
A new client that paints something **imports `outl_theme`** — it does not redefine "what dracula looks like".

If a client needs a hue this crate doesn't expose yet, **add the field here first** (every preset, plus the `Palette` struct), then consume it.
Splitting the hue local to one client is how "the link blue on the desktop looks wrong vs the TUI" bugs are born.

## Dependency policy

This crate is **deliberately dependency-light**: only `serde` (for the Tauri wire) and no renderer crate.
Do not add `ratatui` here — the TUI converts in its own crate.
Do not add CSS / Tailwind tooling — the desktop converts in its own crate.
This is what lets every other crate cheaply depend on us.

## What this crate owns

- [`Palette`] — the struct of named hex strings (one per semantic surface).
  Field naming follows "what the surface IS", not "what it looks like" (`ref_link_fg`, not `purple_underline`).
  Two surfaces that genuinely share a style share the field.
- The nine built-in presets in `src/presets.rs`: `outl`, `outl-light`, `default-dark`, `light`, `logseq-light`, `dracula`, `solarized-dark`, `nord`, `monokai`.
- [`PRESETS`] — the canonical user-visible order (alphabetical-ish, brand first).
- [`by_name`] — case- and separator-insensitive lookup so `"Solarized Dark"`, `"solarized_dark"`, and `"solarized-dark"` all resolve.
- [`default`] / [`all`] — fallback + iterator helpers for pickers.
- `Palette::destructive` (RFC 0022) — the delete-confirmation / "cannot be undone" hue, distinct from `warn`.
  Every preset sets its own family's red; see `src/presets.rs`.
- [`Palette::is_light`] (RFC 0022) — BT.601 perceived luminance over `bg`, exposed as a Rust method (never a wire field, so it can't cross the Tauri bridge).
  Its one non-test caller today is `outl doctor`'s light/dark pairing check.
  `mode = "auto"` does not call it — the TUI's `resolve_preset_name` matches on `ThemeMode` directly and always resolves `Auto` to the dark side, and no GUI client resolves `auto` in Rust yet.
  The desktop's `color-scheme` CSS comes from `isLightHex` in `outl-frontend-shared/src/theme/palette.ts`, a hand-synced client-side copy of this same check, by design — see `docs/rfcs/0022-unified-design-tokens.md`.

## What this crate does NOT own

- ❌ Renderer-specific style flags (`BOLD`, `UNDERLINED`).
  Those live with the renderer that knows what "bold" means in its medium.
- ❌ CSS variable names.
  The desktop chooses its prefix; this crate only ships hex strings.
- ❌ Theme overrides / per-workspace customization.
  That's `outl-config`'s job — this crate is just the catalog.
- ❌ Color manipulation (lighten, darken).
  Add hand-tuned variants as new fields, not as runtime derivations — a runtime-derived shade looks different on a CRT than on an OLED.

## Adding a preset

1. Add a `pub fn my_preset() -> Palette { … }` in `src/presets.rs`.
2. Set every field — the `every_palette_field_is_hex` test will fail if any field is empty or non-hex.
3. Add the canonical name to [`PRESETS`] in `src/lib.rs` (alphabetical, after `outl`).
4. Add a match arm in [`by_name`] (include any user-friendly aliases — case/separator insensitivity is handled, but `"Solarized Dark"` → `"solarized-dark"` aliasing is per-arm).
5. Add a one-line TUI delegate in `crates/outl-tui/src/theme.rs`:
   ```rust
   pub fn my_preset() -> Theme {
       theme_from_palette("my-preset", &outl_theme::presets::my_preset())
   }
   ```
   Also add the name to `PRESETS` and a match arm in `by_name` in that file.
   `default-dark` and `light` are the only two TUI presets that bypass `theme_from_palette` — they use ANSI named colors so the terminal's palette shows through.
6. Desktop and mobile clients pick up the preset automatically via `list_themes` / `get_theme` Tauri commands — no extra wiring needed there.
7. Update `docs/theming.md` with a one-line description.
8. Update `docs/config.md`'s "Available presets" list.

The `every_listed_preset_resolves` test catches forgetting step 4; the `name_matches_listed_preset` test catches forgetting to set `Palette.name`.

## Adding a field

A new field on `Palette` is a **coordinated change** across every consumer.
Steps:

1. Add the field to `Palette` in `src/palette.rs`.
   Name it after the surface, not the color.
2. Add a value in **every** preset in `src/presets.rs`.
   Don't ship a `String::new()` placeholder — `every_palette_field_is_hex` will fail.
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
| Hex values per preset | `outl-theme` | Single source. |
| `ratatui::Color` mapping + modifiers | `outl-tui::theme::Theme::from_palette` | Terminal-only knowledge. |
| CSS custom-property names + Tailwind class wiring | `outl-frontend-shared::src/theme/palette.ts` (`applyPaletteToRoot`, exported as `@outl/shared/theme`) + each client's `src/styles.css` | DOM-only knowledge. |
| Which preset is active (per-workspace / global) | `outl-config::ThemeCfg.preset` | User preference, not a palette concern. |
| Resolving the active preset for a given run | `outl-tui::runtime::resolve_theme` / `outl-desktop::commands::theme` | Each client picks; this crate just exposes the catalog. |
