# Theming

outl ships nine built-in palettes (`outl`, `outl-light`, `default-dark`, `light`, `logseq-light`, `dracula`, `solarized-dark`, `nord`, `monokai`).
The hex values live in the shared **`outl-theme`** crate — every styled surface (`ref_link_fg`, `cursor_block_bg`, `bold_fg`, `status_normal_bg`, …) is a named field on a `Palette` struct, and the TUI / desktop / mobile clients each turn those hex strings into whatever their renderer expects.

This means a color change in `outl-theme/src/presets.rs` propagates to every client without a coordinated edit.

## Picking a theme

Four ways to pick, in precedence order:

1. **CLI flag** (this run only):
   ```bash
   outl --workspace ~/notes --theme dracula
   ```
2. **Per-workspace config** — overrides only this workspace:
   ```toml
   # ~/notes/.outl/config.toml
   [theme]
   preset = "dracula"
   ```
3. **Global config** — read by every client (TUI + desktop) via the shared **`outl-config`** crate:
   ```toml
   # ~/.config/outl/config.toml
   [theme]
   preset = "outl-light"
   preset_dark = "outl"
   mode = "auto"
   ```
   The desktop's Settings modal writes here, so changing the theme there propagates to the next `outl-tui` launch automatically.
4. **Default** — the `outl-light` / `outl` brand pair when nothing is configured.

Names are case- and separator-insensitive.
`dracula`, `Dracula`, `DRACULA`, `Solarized Dark`, `solarized_dark`, `solarized-dark` all resolve to the same theme.

You can also swap themes at runtime from the command palette:

```
:theme nord
```

The status line confirms the switch (`theme: nord`).

## Light / dark pair and `mode`

`[theme]` also takes `preset_dark` and `mode`, so a config can declare a light preset and a dark preset and let the client pick between them:

```toml
[theme]
preset = "logseq-light"
preset_dark = "nord"
mode = "auto"
```

`mode` is one of `light`, `dark`, or `auto` (the default).
`light` and `dark` always resolve to their named side.
`auto` is meant to follow the OS appearance setting — but **on the TUI, `auto` resolves to the dark side, always.**
A terminal has no API to ask the OS for its current appearance.
Probing for it (an OSC 11 background-color query, or reading `COLORFGBG`) is unreliable in practice: it is often wrong under tmux/screen (which answer for the multiplexer, not the active pane) and several terminal emulators don't implement it at all.
Rather than guess and sometimes get it wrong, the TUI declares the gap and always renders the dark side when `mode = "auto"`.
This is a deliberate, permanent behaviour of the TUI client, not a bug to file.

**Both GUI clients now resolve the pair.**
All three clients read `mode` / `preset_dark`, validated by `outl doctor`'s pairing check.
The desktop and mobile both call the shared `get_theme_config` command (`outl-tauri-shared::commands::theme`), which resolves `[theme]` — `preset_dark` already falls back to `preset` on the Rust side.
Both feed the result to the shared `installTheme` (`@outl/shared/theme`, moved out of `outl-mobile/src/lib/theme.ts`).
Both sides of the pair are fetched and held in memory; a `prefers-color-scheme` listener repaints locally on an OS appearance flip, with no backend round-trip mid-repaint.
`installTheme` owns the single active installation: installing a saved or restored config replaces both the cached pair and the listener, so Settings cannot leave boot-time values active.
The desktop's `App.tsx::hydrateTheme`, its Settings modal, and mobile's `App.tsx` `onMount` are the call sites.
A fresh config defaults to the brand pair `outl-light` / `outl`; an older config that explicitly contains only `preset` keeps that preset on both sides through `ThemeCfg::dark()`.

**The desktop Settings modal now writes the whole pair.**
`SettingsModal.tsx` renders a mode selector (`light` / `dark` / `auto`) plus one picker per side, backed by two new `Settings` DTO fields — `theme_dark` and `theme_mode` — alongside the existing `theme` (the light side).
Each preview reinstalls the draft pair through `installTheme`, so mode changes, preset changes, and later OS flips all use the same cached configuration.
Save installs the persisted reply; Cancel and backdrop click reinstall the configuration captured when the modal opened.
`crates/outl-desktop/src-tauri/src/settings.rs::restore_unmodeled_sections` no longer restores `preset_dark` / `mode` from disk.
Now that the modal owns all three fields, restoring any of them would silently discard the user's pick instead of protecting it.

## Built-in presets

| Name | Vibe |
|------|------|
| `outl` | The brand palette, matched to the marketing site (avelino.run). Deep-purple background, lavender accent, lemon highlight. Default theme. |
| `outl-light` | The brand light palette. Soft purple-tinted canvas (`#f6f4fb`), brand purple accent (`#7c3aed`) darkened for contrast. Recovered from what `outl-mobile` used to hardcode in `styles.css` before RFC 0022. |
| `default-dark` | The original outl-tui palette. Cyan refs, magenta tags, green code. |
| `light` | High-brightness terminals. Blue refs, red tags. |
| `logseq-light` | Logseq's default light theme. White canvas, warm dark-gray text, Blueprint blue links. Alias: `logseq`. |
| `dracula` | Iconic dark palette — pink, purple, cyan, yellow. |
| `solarized-dark` | Ethan Schoonover's classic. Muted base03 background. |
| `nord` | Arctic blue-greys. Cool, low-contrast. |
| `monokai` | Wimer Hazenberg's high-contrast. Hot pink for highlights. |

`outl theme list` prints them on a terminal.
`outl theme show <name>` dumps every style in that preset (`ref_link = Style { fg: ..., ... }`).

## Semantic surfaces

Every preset fills every field.
If you add a new field to `Theme`, **every preset must set it** — the compiler enforces this.

### Outline

| Field | Used for |
|-------|----------|
| `bullet` | The `- ` glyph on a regular block |
| `selected_bullet` | The `- ` glyph on the focused block |
| `cursor_block` | Vim-style block cursor (char under cursor in Normal) |
| `cursor_caret` | Thin caret (`▏`) at end-of-line or in Insert |
| `property_key` / `property_value` | `key:: value` lines |
| `heading` | Page title in the header |

### Inline tokens

| Field | Used for |
|-------|----------|
| `ref_link` | `[[page]]` references |
| `tag_link` | `#tag` references |
| `md_link` | `[text](url)` markdown links |
| `bold` / `italic` / `strike` / `code` | Standard markdown emphasis |
| `todo_open` / `todo_done` / `todo_done_body` | TODO / DONE prefix + DONE body |
| `dim` | Delimiters in raw render mode (`**`, `~~`, etc.) |

### Chrome

| Field | Used for |
|-------|----------|
| `border` | Panel borders |
| `hint` | Footer hint text |
| `status_normal` / `status_insert` / `status_visual` | Mode badges |
| `status_message` | Transient status messages |
| `help_title` | Section titles in the help popup, overlay titles |
| `popup_bg` | Background color for overlays |
| `list_selected` | Highlighted entry in popups (quick switcher, search) |

## Defining a new preset

Presets are constructors in `crates/outl-theme/src/presets.rs`:

```rust
pub fn my_theme() -> Palette {
    Palette {
        name: "my-theme".into(),
        bg: "#141420".into(),
        // ... fill every field with a #rrggbb hex string
    }
}
```

Then:

1. Add the name to the `PRESETS` slice in `crates/outl-theme/src/lib.rs`.
2. Add the `"my-theme" => Some(presets::my_theme())` arm to `by_name`.
3. The compiler tells you if you missed a field on `Palette`.
4. Add a TUI delegate in `crates/outl-tui/src/theme.rs`:
   ```rust
   pub fn my_theme() -> Theme {
       theme_from_palette("my-theme", &outl_theme::presets::my_theme())
   }
   ```
   `default-dark` and `light` are the two TUI presets that *don't* go through `theme_from_palette` — they build on top of ANSI named colors (`Color::Reset`, `Color::DarkGray`, …) so the user's terminal palette shows through, which is intentional for ANSI-only environments.
5. Desktop and mobile clients pick the preset up automatically through `list_themes` / `get_theme` Tauri commands.

The `every_listed_preset_resolves` test ensures every name in `PRESETS` has a working constructor.
The `every_palette_field_is_hex` test catches a typo like `"#xyz123"` or a missed `#` prefix before it hits the renderers.

## Tips

- **Don't overlap modifiers on the same field across themes.** Solarized's `bold` is `fg(orange) + BOLD`; Dracula's is similar but on orange too.
  Keep modifiers semantic (BOLD for bold, etc.) and let the color carry the personality.
- **Backgrounds**: the RGB presets (`outl`, `logseq-light`, `dracula`, `solarized-dark`, `nord`, `monokai`) paint `bg` across the whole TUI canvas and use `fg` as the base text color, so a light theme stays readable on a dark terminal (and vice versa).
  Only the two ANSI presets (`default-dark`, `light`) keep `Color::Reset` and inherit the terminal's own background/foreground — that's their point.
- **Underline on `ref_link` and `tag_link` is intentional.** They're the only "clickable" things in pretty-render mode, and the underline is the visual affordance.
- **Contrast matters more than tone.** Test your theme against a workspace with lots of refs, tags, code, and TODOs.

## How each client consumes the palette

| Client | What it does with the hex |
|---|---|
| **`outl-tui`** | `crates/outl-tui/src/theme.rs::theme_from_palette` converts each `#rrggbb` to `ratatui::Color::Rgb(r, g, b)` and re-applies the consistent modifiers (`BOLD` on `bold`, `UNDERLINED` on links, `ITALIC` on `italic`, `CROSSED_OUT` on `strike`). The six RGB presets (`outl`, `logseq-light`, `dracula`, `solarized-dark`, `nord`, `monokai`) are one-line delegates; `default-dark` and `light` stay manual on ANSI named colors. |
| **`outl-desktop`** | The Tauri commands `list_themes()` and `get_theme(name)` return the `Palette` as JSON. The frontend writes each field as a CSS custom property on `<html>` (`--color-outl-accent`, `--color-outl-ref-link-fg`, …) so Tailwind class utilities like `text-(--color-outl-accent)` resolve at runtime, and flips `color-scheme` (light/dark) from the palette's `bg` luminance so native controls and scrollbars follow. Settings modal exposes the dropdown. Chrome surfaces never hardcode a hue — translucent layers derive from `--color-outl-fg` (`bg-(--color-outl-fg)/10`) so they adapt to light and dark presets alike. |
| **`outl-mobile`** | Shared `@outl/shared/theme::installTheme` fetches both sides of the configured pair and holds both `Palette` objects in memory, then calls `applyPaletteToRoot` (RFC 0022, issue #22). A `prefers-color-scheme` media-query listener swaps tokens on an OS appearance flip without a second backend round-trip; a fresh default config uses `outl-light` / `outl`. |

### Desktop CSS custom-property namespaces

`applyPaletteToRoot` (`@outl/shared/theme`, RFC 0022 — moved out of the desktop crate's own `src/lib/palette.ts`) writes one CSS custom-property namespace on every theme switch: the canonical **`--color-outl-*`** set (`bg-(--color-outl-bg-elev)`, `border-(--color-outl-fg)/15`, etc.).

The legacy `--color-ios-*` / `--color-iosd-*` namespace it used to also write is gone.
`@outl/shared/markdown` (`MarkdownInline`, `EmbeddedSubtree`) no longer reads it either — see [`outl-frontend-shared/CLAUDE.md`](../crates/outl-frontend-shared/CLAUDE.md#theming-note).
`src/styles.css` still declares the legacy tokens in its `@theme` block; nothing reads them, and deleting the block is a later, gated task.

`src/styles.css` provides boot-default values for `--color-outl-*` so the page isn't flash-unstyled before `applyPaletteToRoot` runs.
`color-scheme` is set from the palette's `bg` luminance so native controls (scrollbars, `<select>`) follow the active preset.

## Future

- **User TOML overrides** — `[theme.colors]` table letting you tweak fields without rebuilding the binary.
- **Theme hot-reload on config change** — listen on `~/.config/outl/config.toml` and per-workspace `.outl/config.toml` via `notify`.
- **`outl theme preview`** — render every preset side-by-side on the same fixture so you can pick by feel.

None of these are wired today; they're tracked when there's user demand.
