# Theming

outl ships eleven built-in palettes (`outl`, `outl-light`, `ansi`, `ansi-light`, `logseq-light`, `dracula`, `solarized`, `nord`, `monokai`, `gruvbox`, `gruvbox-light`), and takes your own from `~/.config/outl/themes/`.
The hex values live in the shared **`outl-theme`** crate — every styled surface (`ref_link_fg`, `cursor_block_bg`, `bold_fg`, `status_normal_bg`, …) is a named field on a `Palette` struct, and the TUI / desktop / mobile clients each turn those hex strings into whatever their renderer expects.

This means a color change in `outl-theme/themes/*.toml` propagates to every client without a coordinated edit.

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
4. **Default** — the `outl-light` / `outl` brand pair when nothing is configured, **unless the terminal does not accept 24-bit colour**, in which case the TUI falls back to the ANSI-based `ansi` instead ([#453](https://github.com/outlmd/outl/issues/453)).

   Every preset but `ansi` and `ansi-light` maps each palette field through `Color::Rgb`, so an unconfigured launch used to emit 24-bit SGR whatever the terminal could render: approximated on a 256-colour tmux without `Tc`, and potentially unreadable on a plain `xterm` or `TERM=linux`.

   The signals are `COLORTERM` in (`truecolor`, `24bit`) and a terminfo direct-colour `TERM` entry (`*-direct`, `*-direct16`, `*-direct256`). Both are widely set, which is what separates this from the OS appearance setting — that one has no reliable terminal answer, which is why `mode = "auto"` is pinned to dark rather than probed.

   **This applies to level 4 only.** A preset you named with `--theme`, in the per-workspace config, or in the global config is an explicit choice and is honoured as-is: ask for `dracula` on a 256-colour terminal and you get `dracula`. Overriding an explicit request would be the bigger surprise.

   A terminal that supports 24-bit colour and sets neither signal gets the ANSI preset, which renders correctly. The opposite default renders wrong, so absence is read as "no".

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
| `outl-light` | The brand light palette. Paper-white canvas (`#ffffff`) with a barely raised `bg_elev` (`#f7f7fa`), cool grays and one violet accent (`#6d28d9`, 7.1:1). Started as what `outl-mobile` hardcoded in `styles.css` before RFC 0022; refreshed toward a calmer single-accent canvas. |
| `ansi` | The original outl-tui palette, built from the 16 ANSI colours. Cyan refs, magenta tags, green code. On a terminal it renders through ratatui's *named* colours, so your own palette shows through. Was `default-dark`, which still resolves. |
| `ansi-light` | The ANSI pair's light side, for high-brightness terminals. Blue refs, red tags. Was `light`, which still resolves. |
| `logseq-light` | Logseq's default light theme. White canvas, warm dark-gray text, Blueprint blue links. Alias: `logseq`. |
| `dracula` | Iconic dark palette — pink, purple, cyan, yellow. |
| `solarized` | Ethan Schoonover's classic. Muted base03 background. Alias: `solarized-dark`. |
| `nord` | Arctic blue-greys. Cool, low-contrast. |
| `monokai` | Wimer Hazenberg's high-contrast. Hot pink for highlights. |
| `gruvbox` | morhetz's retro-groove dark. Warm charcoal, cream text, muted yellow/green/aqua. Alias: `gruvbox-dark`. |
| `gruvbox-light` | morhetz's retro-groove light. Warm cream canvas, dark warm-gray text, muted accents darkened for a light background. |

`outl theme list` prints them on a terminal.
`outl theme show <name>` dumps every style in that preset (`ref_link = Style { fg: ..., ... }`).

## Semantic surfaces

Every preset fills every field, directly or by inheriting it.
A field with nothing to inherit from is a load error naming the field, never a default: a theme is not a surface that can be half-painted, and a missing `fg` is invisible text.

For a **built-in**, that error is a `cargo test` failure, so an incomplete preset never ships. For **your own**, it is what `outl theme list` and `outl doctor` report.

### Outline

| Field | Used for |
|-------|----------|
| `bullet` | The `- ` glyph on a regular block |
| `selected_bullet` | The `- ` glyph on the focused block |
| `cursor_block` | Vim-style block cursor (char under cursor in Normal) |
| `cursor_caret` | Insert-mode caret: underlines the character it precedes, drawn as a thin `▏` past end-of-line |
| `property_key` / `property_value` | `key:: value` lines — **TUI only**, see below |
| `heading` | Page title in the header |

**`property_value_fg` is the one token the clients read differently, on purpose.**
The TUI paints every property value with it, and the brand pair sets it to `accent_soft`, so every `status:: waiting` is lavender.
The GUI clients draw their property line with `fg_dimmer` / `fg_dim` instead, because accent there means *this will interrupt you* and only a live `remind::` earns it — a colour spent on every value is a colour that signals nothing.
`property_key_fg` does not diverge: every preset already sets it to `fg_dimmer`, which is what the GUI uses.

Converging them is a change to the **presets**, not to a client: set `property_value_fg` to `fg_dim` and the TUI stops over-signalling too.
That restyles every theme for every terminal user, which is why it is written down here instead of done quietly.
Until then the token's name is wider than its meaning (root [`CLAUDE.md`](../CLAUDE.md) invariant 13), and that is the recorded gap rather than a discovered one.

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
| `status_message` | Transient status messages (the palette's `warn` — "look at this") |
| `destructive` | Something waiting will not fix: the "this page isn't syncing" banner, delete confirmations (the palette's `destructive`) |
| `help_title` | Section titles in the help popup, overlay titles |
| `popup_bg` | Background color for overlays |
| `list_selected` | Highlighted entry in popups (quick switcher, search) |

## Defining a new preset

A theme is one `.toml` file per family, with `[dark]` and `[light]` inside it.
Built-in themes live in `crates/outl-theme/themes/` and are compiled into the binary; your own live in `~/.config/outl/themes/` and are read at startup.
**Same format either way.**

```toml
# crates/outl-theme/themes/my-theme.toml
[dark]
bg = "#141420"
fg = "#e8e8f0"
# … every Palette field

[light]
bg = "#ffffff"      # inherits [dark], so only the delta is written
fg = "#1a1a2e"
accent = "#6d28d9"
```

Names are derived, never written:

| File + section | Resolves as |
|---|---|
| `my-theme.toml` `[dark]` | `my-theme` (canonical), `my-theme-dark` |
| `my-theme.toml` `[light]` | `my-theme-light` |
| a file with only `[light]` | `my-theme-light`, plus `my-theme` |

Writing `name = "…"` in a section is an error, not an override — the filename is the name.

For a built-in, add one line to `BUILTIN` in `crates/outl-theme/src/registry.rs` (its position is its position in the picker), then update the preset table above and `docs/config.md`'s list, and regenerate the browser mock's fixture:

```sh
OUTL_UPDATE_FIXTURES=1 cargo test -p outl-theme --test presets_fixture
```

Nothing else needs wiring. The TUI, the desktop, and mobile all read the same registry.

### Your own theme

Drop a `.toml` in `~/.config/outl/themes/` (`$XDG_CONFIG_HOME/outl/themes/` when that is set; `%APPDATA%\outl\themes\` on Windows). No restart of anything but the app you are using.

The shortest useful theme is three lines — `extends` takes any theme name, built-in or your own:

```toml
# ~/.config/outl/themes/mine.toml
[dark]
extends = "outl"
accent = "#ff6ac1"
```

That resolves as `mine`, appears in every picker, and is usable anywhere a preset name goes:

```toml
[theme]
preset = "mine-light"
preset_dark = "mine"
mode = "auto"
```

A file that names a built-in it shadows (`dracula.toml`) replaces that built-in rather than adding a second entry.

**Inheritance is field by field, and `accent` is not a role.**
This is the one thing that surprises people writing their first theme.
In `outl`, five fields hold the same violet — `accent`, `ref_link_fg`, `selected_bullet_bg`, `status_normal_bg`, `list_selected_bg` — because that is what the palette was authored with, not because four of them follow the fifth.
So the file above repaints the accent *surfaces* and leaves `[[refs]]` the original violet.

Nothing derives a colour from another colour here, on purpose: a runtime-lightened shade looks different on an OLED than on a CRT, and a hand-tuned one does not.
If you want one hue everywhere it appears, name every field:

```toml
[dark]
extends = "outl"
accent = "#ff6ac1"
ref_link_fg = "#ff6ac1"
selected_bullet_bg = "#ff6ac1"
status_normal_bg = "#ff6ac1"
list_selected_bg = "#ff6ac1"
```

`outl theme show mine` prints every resolved surface, which is the fastest way to find the ones you missed.
The full field list, with what each one paints, is under [Semantic surfaces](#semantic-surfaces).

**What is refused, and where you hear about it.**
A missing field with nothing to inherit, a value that is not `#rrggbb`, a misspelled field, a written `name`, an `extends` pointing at nothing, and a file that could not be read at all are each an error naming the thing that is wrong. The theme does not load, and the rest of your themes are unaffected.

```
$ outl theme list
warning: theme `mine` field `accent` is not a hex colour: `rebeccapurple`
```

`outl doctor` warns about the same files. The TUI and the GUI clients write it to their log only — the palette is resolved before there is a status line or a toast to put it in, so when a theme is missing from the picker, those two commands are where the reason is.

### Validating a theme

```sh
outl theme list          # names any file that failed to load, on stderr
outl theme show mine     # dumps every resolved surface
outl doctor              # warns about each broken file
```

## Tips

- **Don't overlap modifiers on the same field across themes.** Solarized's `bold` is `fg(orange) + BOLD`; Dracula's is similar but on orange too.
  Keep modifiers semantic (BOLD for bold, etc.) and let the color carry the personality.
- **Backgrounds**: the RGB presets (every one but the `ansi` pair) paint `bg` across the whole TUI canvas and use `fg` as the base text color, so a light theme stays readable on a dark terminal (and vice versa).
  Only `ansi` and `ansi-light` keep `Color::Reset` and inherit the terminal's own background/foreground — that's their point.
- **Underline on `ref_link` and `tag_link` is intentional.** They're the only "clickable" things in pretty-render mode, and the underline is the visual affordance.
  The Insert-mode caret underlines too (it marks the character it precedes rather than printing a glyph — [#320](https://github.com/outlmd/outl/issues/320)), and on a link character it also swaps the hue to `cursor_caret_fg` and adds `BOLD`, so the two never read as the same thing.
  The underline is what keeps the caret visible on a space, where a foreground colour alone paints nothing; past the end of the line there is no character to mark, so a thin `▏` is drawn there instead.
- **Pick a `cursor_caret_fg` that differs from `fg`.**
  Four shipped presets (`light`, `dracula`, `nord`, `monokai`) set them equal, which leaves the underline carrying the caret on its own.
  That is legible, but a distinct hue is what makes the caret findable at a glance in a wall of text.
- **Contrast matters more than tone.** Test your theme against a workspace with lots of refs, tags, code, and TODOs.

## How each client consumes the palette

| Client | What it does with the hex |
|---|---|
| **`outl-tui`** | `crates/outl-tui/src/theme.rs::theme_from_palette` converts each `#rrggbb` to `ratatui::Color::Rgb(r, g, b)` and re-applies the consistent modifiers (`BOLD` on `bold`, `UNDERLINED` on links, `ITALIC` on `italic`, `CROSSED_OUT` on `strike`). Every preset resolves through `outl_theme::Registry` and renders via `theme::from_palette`; `ansi` and `ansi-light` are the two it hands to hand-built themes on ANSI named colors instead. A theme from `~/.config/outl/themes/` goes through the same path. |
| **`outl-desktop`** | The Tauri commands `list_themes()` and `get_theme(name)` return the `Palette` as JSON. The frontend writes each field as a CSS custom property on `<html>` (`--color-outl-accent`, `--color-outl-ref-link-fg`, …) so Tailwind class utilities like `text-(--color-outl-accent)` resolve at runtime, and flips `color-scheme` (light/dark) from the palette's `bg` luminance so native controls and scrollbars follow. Settings modal exposes the dropdown. Chrome surfaces never hardcode a hue — translucent layers derive from `--color-outl-fg` (`bg-(--color-outl-fg)/10`) so they adapt to light and dark presets alike. |
| **`outl-mobile`** | Shared `@outl/shared/theme::installTheme` fetches both sides of the configured pair and holds both `Palette` objects in memory, then calls `applyPaletteToRoot` (RFC 0022, issue #22). A `prefers-color-scheme` media-query listener swaps tokens on an OS appearance flip without a second backend round-trip; a fresh default config uses `outl-light` / `outl`. |

### Desktop CSS custom-property namespaces

`applyPaletteToRoot` (`@outl/shared/theme`, RFC 0022 — moved out of the desktop crate's own `src/lib/palette.ts`) writes one CSS custom-property namespace on every theme switch: the canonical **`--color-outl-*`** set (`bg-(--color-outl-bg-elev)`, `border-(--color-outl-fg)/15`, etc.).

The legacy `--color-ios-*` / `--color-iosd-*` namespace it used to also write is gone.
`@outl/shared/markdown` (`MarkdownInline`, `EmbeddedSubtree`) no longer reads it either — see [`outl-frontend-shared/CLAUDE.md`](../crates/outl-frontend-shared/CLAUDE.md#theming-note).
`src/styles.css` no longer declares the legacy tokens in its `@theme` block — they were removed, and `the_theme_tokens_match_the_palette` fails if any come back.

`src/styles.css` provides boot-default values for `--color-outl-*` so the page isn't flash-unstyled before `applyPaletteToRoot` runs.
`color-scheme` is set from the palette's `bg` luminance so native controls (scrollbars, `<select>`) follow the active preset.

Mermaid diagrams map Pele's `--pele-*` properties onto that same `--color-outl-*` set in `@outl/shared/diagram/styles`.
Background, foreground, border, accent, destructive (`--pele-error` and `--pele-critical`), and the UI fonts come from the palette.
`--pele-surface` is the elevated background.
`--pele-surface-alt` is `--color-outl-accent-soft`, so a second fill is distinct from the first.
`--pele-series-1` through `--pele-series-8` are accent, accent-alt, bold, tag-link, markdown-link, warn, todo-done, and destructive, in that order.
`ref-link` is not one of them: on the brand presets it is the same colour as accent, so it would repeat series 1.
`bold` differs from accent in every shipped preset.
Both GUI clients import the stylesheet.
A theme switch repaints an already mounted diagram. Nothing re-renders it.

## Future
- **Theme hot-reload on config change** — listen on `~/.config/outl/config.toml` and per-workspace `.outl/config.toml` via `notify`.
- **`outl theme preview`** — render every preset side-by-side on the same fixture so you can pick by feel.

None of these are wired today; they're tracked when there's user demand.
