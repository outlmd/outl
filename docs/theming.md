# Theming

outl ships eleven built-in palettes and reads your own from `~/.config/outl/themes/`.
Every client paints from the same palette file, so one theme looks the same in the terminal, on the desktop and on the phone.

This page is for people who want to **change how outl looks**.
The first half is the task list: pick a theme, pair a light and a dark one, write your own.
The second half is the reference: every colour a theme can set, and what each client does with it.

- [Pick a theme](#picking-a-theme)
- [Light / dark pair and `mode`](#light--dark-pair-and-mode)
- [Built-in presets](#built-in-presets)
- [Your own theme](#your-own-theme)
- [Palette reference](#palette-reference)
- [Troubleshooting](#troubleshooting)
- [How each client consumes the palette](#how-each-client-consumes-the-palette)

## Picking a theme

The fastest way, per client:

| Client | How |
|---|---|
| **TUI** | Type `:theme nord` in Normal mode. The status line answers `theme: nord`, or `unknown theme: nord`. The switch lasts until you quit. |
| **Desktop** | `Cmd/Ctrl+,` opens Settings. Pick a preset for the light side, one for the dark side, and a mode. Choosing previews live, **Save** keeps it, **Cancel** puts back what you had. |
| **Mobile** | No picker today. The app follows `[theme]` from the config and your phone's appearance setting (light or dark). |

To make a choice stick for the TUI, or to set it by hand, use the config.
There are four sources, and the first one that names a theme you have wins:

1. **CLI flag**, this run only:
   ```bash
   outl --workspace ~/notes --theme dracula
   ```
2. **Per-workspace config**, this workspace only (TUI):
   ```toml
   # ~/notes/.outl/config.toml
   [theme]
   preset = "dracula"
   ```
   Only `preset` is read here.
   `preset_dark` and `mode` belong to the global file.
3. **Global config**, read by every client:
   ```toml
   # ~/.config/outl/config.toml
   [theme]
   preset = "outl-light"
   preset_dark = "outl"
   mode = "auto"
   ```
   The desktop's Settings modal writes this file, so a change there reaches the next `outl-tui` launch.
   The GUI clients read the global file only.
4. **Default**: the `outl-light` / `outl` brand pair.
   On the TUI, if the terminal does not report 24-bit colour, the default becomes the ANSI-based `ansi` preset instead ([#453](https://github.com/outlmd/outl/issues/453)).

**An unknown name does not fail.**
On the TUI a name that matches nothing is skipped and the next source is tried, with no message.
If your theme "does nothing", run `outl theme list` and check the spelling.
The GUI clients fall back to the default palette.

**The 24-bit check applies to level 4 only.**
It looks for `COLORTERM=truecolor` or `24bit`, or a `TERM` containing `-direct`.
A terminal that supports 24-bit colour and sets neither gets `ansi`, which renders correctly everywhere; guessing the other way renders wrong.
A theme you named in the flag or a config is honoured as written, even on a 256-colour terminal.

**Names are forgiving.**
Case does not matter, and a space, `_` and `-` are the same character: `Solarized Dark`, `solarized_dark` and `solarized-dark` are one theme.
Every family also answers to `<name>-dark` and `<name>-light` for the sides it has.
Four pre-0.14 names still work: `default-dark` and `dark` mean `ansi`, `light` means `ansi-light`, `default` means `outl`.

## Light / dark pair and `mode`

`[theme]` takes a light preset, a dark preset and a mode, so one config follows the time of day:

```toml
[theme]
preset = "logseq-light"
preset_dark = "nord"
mode = "auto"
```

| `mode` | Result |
|---|---|
| `light` | Always the `preset` side. |
| `dark` | Always the `preset_dark` side. |
| `auto` (default) | Follows the OS appearance, **except on the TUI**. |

**On the TUI, `auto` is always dark.**
A terminal has no API for the OS appearance, and guessing from `COLORFGBG` or an OSC 11 query is wrong often enough (under tmux, in terminals that skip the query) that outl does not try.
If you want a light TUI, set `mode = "light"`.
This is permanent behaviour, not a bug.

A config that only has `preset` keeps working: with no `preset_dark`, both sides use `preset`.
A fresh config gets the brand pair `outl-light` / `outl`.

`outl doctor` warns when the pair is mismatched, for example `mode = "light"` pointing at a dark preset.

Desktop and mobile re-resolve the pair when the OS flips between light and dark, without a restart and without a round-trip to the backend.

## Built-in presets

| Name | Look |
|------|------|
| `outl` | The brand palette, matched to avelino.run. Deep-purple canvas, lavender accent, lemon highlight. The default. |
| `outl-light` | The brand light side. White canvas, cool grays, one violet accent (`#6d28d9`, 7.1:1 contrast). |
| `ansi` | Built from the 16 ANSI colours: cyan refs, magenta tags, green code. On a terminal it uses the named colours, so **your own terminal palette shows through**. Was `default-dark`. |
| `ansi-light` | The ANSI pair's light side, for bright terminals. Blue refs, red tags. Was `light`. |
| `logseq-light` | Logseq's default light theme. White canvas, warm dark-gray text, blue links. Alias: `logseq`. |
| `dracula` | Pink, purple, cyan, yellow on a dark canvas. |
| `solarized` | Ethan Schoonover's classic. Muted base03 background. Alias: `solarized-dark`. |
| `nord` | Arctic blue-greys. Cool, low contrast. |
| `monokai` | Wimer Hazenberg's high-contrast palette. Hot pink highlights. |
| `gruvbox` | morhetz's retro-groove dark. Warm charcoal, cream text. Alias: `gruvbox-dark`. |
| `gruvbox-light` | The light side. Warm cream canvas, dark warm-gray text, accents darkened to read on cream. |

```bash
outl theme list          # every theme, including your own
outl theme show nord     # how the TUI paints that theme (see below)
```

`outl theme list` prints one canonical name per theme, so aliases such as `logseq`, `solarized-dark` and `mine-dark` resolve but are not listed.
`outl theme show <name>` prints the TUI's style for about twenty surfaces (`ref_link`, `status_normal`, ...) as ratatui values such as `Style::new().fg(Color::Rgb(167, 139, 250)).underlined()`.
It is a quick look at the result, not a dump of the file.
The names it prints are TUI style names; the names you write in a theme file are in the [Palette reference](#palette-reference).

## Your own theme

A theme is one `.toml` file.
Put it in:

| OS | Folder |
|---|---|
| macOS, Linux | `~/.config/outl/themes/` |
| Any OS with `XDG_CONFIG_HOME` set (it wins over the rows around it) | `$XDG_CONFIG_HOME/outl/themes/` |
| Windows | `%APPDATA%\outl\themes\` |

The folder does not exist until you make it.
The app you are using needs a restart to see a new file on the TUI.
The desktop reads the folder every time the theme list is requested, so reopening Settings is enough.

### Smallest useful theme

`extends` takes any theme name, built-in or your own, and you write only what differs:

```toml
# ~/.config/outl/themes/mine.toml
[dark]
extends = "outl"
accent = "#ff6ac1"
```

This file defines `mine` and `mine-dark`.
It has no light side, so `mine-light` does not exist.
Use it anywhere a preset name goes:

```toml
[theme]
preset = "mine"
mode = "dark"
```

### A light and dark pair

Give the file both sections.
Extend a light base in `[light]`, because an unmarked `[light]` inherits your `[dark]`, and a dark canvas with light overrides is rarely what you want:

```toml
# ~/.config/outl/themes/mine.toml
[dark]
extends = "outl"
accent = "#ff6ac1"

[light]
extends = "outl-light"
accent = "#c026d3"
```

Now `mine`, `mine-dark` and `mine-light` all resolve:

```toml
[theme]
preset = "mine-light"
preset_dark = "mine"
mode = "auto"
```

Checked against a real build:

```
$ outl theme list
...
  gruvbox-light
  mine
  mine-light
```

### How names are decided

The name comes from the file name and the section.
You never write it.

| File and section | Resolves as |
|---|---|
| `mine.toml` `[dark]` | `mine` and `mine-dark` |
| `mine.toml` `[light]` | `mine-light` |
| a file with only `[light]` | `mine-light` and `mine` |

Writing `name = "..."` in a section is an error, not an override.
A file named like a built-in (`dracula.toml`) replaces that built-in in place and keeps its position in the picker.

### Inheritance is field by field

This surprises people on their first theme, so read it before you write one.
**`accent` is a field, not a role.**
In `outl`, five fields hold the same violet: `accent`, `ref_link_fg`, `selected_bullet_bg`, `status_normal_bg` and `list_selected_bg`.
They match because the palette was authored that way, not because four of them follow the fifth.
So the `mine` file above repaints the accent surfaces and leaves `[[refs]]` the original violet.

Nothing derives a colour from another colour, on purpose.
A shade computed at runtime looks different on an OLED than on a CRT, and a hand-picked one does not.
If you want one hue everywhere, name every field:

```toml
[dark]
extends = "outl"
accent = "#ff6ac1"
ref_link_fg = "#ff6ac1"
selected_bullet_bg = "#ff6ac1"
status_normal_bg = "#ff6ac1"
list_selected_bg = "#ff6ac1"
```

A full file with no `extends` must set every field in the [Palette reference](#palette-reference).
A missing field with nothing to inherit is an error that names the field, never a silent default, because a missing `fg` is invisible text.

### Rules a theme file must meet

- Colours are `#rrggbb` (an `#rrggbbaa` is accepted and the alpha is ignored).
  Three-digit hex and colour names such as `rebeccapurple` are refused.
- Top-level keys are `[dark]` and `[light]` only, and at least one must exist.
- Field names must match exactly.
  A typo is an error listing every valid name.
- `extends` must name a theme that loads.
  Two themes that extend each other are reported as a missing base.

One broken file does not take the others down.
The broken theme is absent from every picker, and the rest load normally.

### Check your theme

```bash
outl theme list     # names any file that failed to load, on stderr
outl theme show mine
outl doctor         # warns about each broken file and a mismatched light/dark pair
```

What the errors look like (real output):

```
warning: theme `bad` field `accent` is not a hex colour: `rebeccapurple`
warning: theme `n` writes `name` in its [dark] section; the name is derived from the file name (`n.toml`) and the section
warning: theme `partial`: missing field `bg_elev`
warning: theme `g` extends `ghost`, which does not exist
warning: theme `typo`: unknown field `acent`, expected one of `name`, `bg`, ...
```

The TUI and the GUI clients write the same message to their log only.
The palette is resolved before there is a status line or toast to show it in, so when a theme is missing from a picker, `outl theme list` and `outl doctor` are where the reason is.

### The TUI's `:theme` command and your themes

`:theme <name>` on the TUI resolves built-in names only.
`:theme mine` answers `unknown theme: mine`, even though `--theme mine` and `[theme] preset = "mine"` both work.
Select your own theme through the flag or the config until that is fixed.

## Palette reference

A palette has **45 colour fields** plus a derived `name`.
Every field is a `#rrggbb` string.
Field names say what the surface **is**, not what it looks like.

### Canvas

| Field | Paints |
|-------|--------|
| `bg` | The main canvas. Whether a theme counts as light or dark is computed from this colour. |
| `bg_elev` | Raised surfaces: popups, modals, the picker. The TUI paints overlays with it. |
| `fg` | Body text. |
| `fg_dim` | Secondary text and metadata. |
| `fg_dimmer` | Placeholders, separators and the plain `-` bullet. |
| `border` | Panel and pane borders. |
| `hint` | The footer hint line. |

### Accent and status colours

| Field | Paints |
|-------|--------|
| `accent` | The primary accent: selection, active state, primary buttons. |
| `accent_fg` | Text that sits on a solid `accent` fill (primary buttons, the selected calendar day). Must be readable on `accent`. |
| `accent_soft` | A lighter accent. |
| `accent_alt` | A second hue, distinct from `accent`, for code and success. |
| `warn` | "Look at this": open TODOs, transient messages. |
| `destructive` | "This cannot be undone": delete confirmations, the "this page isn't syncing" banner, error toasts. |
| `destructive_fg` | Text on a solid `destructive` fill. Must be readable on it. |

### Inline markdown

| Field | Paints |
|-------|--------|
| `ref_link_fg` | `[[page]]` references. Underlined on the TUI. |
| `tag_link_fg` | `#tag` references. Underlined on the TUI. |
| `md_link_fg` | `[text](url)` links. Underlined on the TUI. |
| `bold_fg` | `**bold**` text. |
| `italic_fg` | `_italic_` text. |
| `strike_fg` | `~~strike~~` text. |
| `highlight_bg` / `highlight_fg` | `==highlight==`: the marker fill and the text on it. |
| `code_fg` | `` `code` `` text. |
| `dim_fg` | The delimiters (`**`, `~~`) when a block is shown in raw form. |

### Tasks and properties

| Field | Paints |
|-------|--------|
| `todo_open_fg` | The `TODO` marker. |
| `todo_done_fg` | The `DONE` marker. |
| `todo_done_body_fg` | The text of a finished task, struck through. |
| `property_key_fg` | The key of a `key:: value` line. |
| `property_value_fg` | The value of a `key:: value` line (see the note below). |
| `heading_fg` | The page title in the header. |

### Selection and cursor

| Field | Paints |
|-------|--------|
| `selected_bullet_bg` / `selected_bullet_fg` | The bullet of the focused block. |
| `cursor_block_bg` / `cursor_block_fg` | The vim block cursor in Normal mode (TUI). |
| `cursor_caret_fg` | The Insert-mode caret (TUI). |

### Chrome

| Field | Paints |
|-------|--------|
| `status_normal_bg` / `status_normal_fg` | The Normal-mode badge. |
| `status_insert_bg` / `status_insert_fg` | The Insert-mode badge. |
| `status_visual_bg` / `status_visual_fg` | The Visual-mode badge. |
| `status_message_fg` | Transient status-line messages. |
| `list_selected_bg` / `list_selected_fg` | The highlighted row in pickers and popups. |
| `help_title_fg` | Section titles in the help popup and overlay titles. |

**`property_value_fg` is read differently on purpose.**
The TUI paints every property value with it, and the brand pair sets it to `accent_soft`, so every `status:: waiting` is lavender.
The GUI clients draw property lines with `fg_dimmer` / `fg_dim` instead, because on a GUI the accent means "this will interrupt you" and only a live `remind::` earns it.
Until the presets change, expect the TUI to show more colour on properties than the apps do.

### Tips

- **Contrast beats tone.**
  Check your theme in a workspace with many refs, tags, code blocks and TODOs.
- **Pair fills with their text.**
  Every `_bg` / `_fg` pair, plus `accent` / `accent_fg` and `destructive` / `destructive_fg`, is a solid fill with text on it.
  The built-in themes are tested against WCAG AA for those pairs.
  Yours are not, so check them yourself.
- **Pick a `cursor_caret_fg` that differs from `fg`.**
  `ansi`, `ansi-light`, `dracula`, `monokai` and `nord` set them equal, which leaves the underline alone carrying the caret.
  A distinct hue makes the caret findable in a wall of text.
- **Underlines on links are intentional.**
  `ref_link_fg` and `tag_link_fg` are the clickable things on the TUI, and the underline is the affordance.
  The Insert caret underlines the character it precedes too.
  On a link character it also swaps to `cursor_caret_fg` and goes bold, so the two never read as the same thing.

### What the TUI does with a palette

- Every preset but the ANSI pair paints `bg` across the whole canvas and uses `fg` as the base text colour, so a light theme stays readable on a dark terminal and the reverse.
- `ansi` and `ansi-light` keep the terminal's own background and foreground and use named colours, which is their point.
- The modifiers are fixed, not themeable: bold is always bold, italic italic, strike crossed out, links underlined.

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| My theme is not in the picker | The file failed to load. Run `outl theme list` and read the warning on stderr. |
| `:theme mine` says `unknown theme` | `:theme` knows built-ins only. Use `--theme mine` or set `[theme] preset` in the config. |
| I set `preset = "dracula"` and still get another theme | A higher-priority source names a theme that exists: the `--theme` flag, then the workspace config. Check both. |
| The TUI is dark although I use `mode = "auto"` | Expected. `auto` is dark on the TUI. Set `mode = "light"` for a light terminal. |
| Colours look wrong in tmux or a plain `xterm` | The terminal did not report 24-bit colour and you set a RGB preset. Use `ansi` / `ansi-light`, or set `COLORTERM=truecolor` if the terminal supports it. |
| Only the accent changed, links kept the old colour | Inheritance is per field. See [Inheritance is field by field](#inheritance-is-field-by-field). |
| The desktop picker shows my old colours | Reopen Settings. The folder is read on each request. |
| `outl doctor` warns about the pair | One side of `preset` / `preset_dark` has the wrong brightness for its slot. Swap them or pick another. |

## How each client consumes the palette

| Client | What it does with the hex |
|---|---|
| **TUI** | Each `#rrggbb` becomes a `ratatui::Color::Rgb` and the fixed modifiers are re-applied. `ansi` and `ansi-light` are built by hand on ANSI named colours instead. User themes take the same path as built-ins. |
| **Desktop** | The `list_themes` and `get_theme` commands return the palette as JSON. The frontend writes each field as a CSS custom property on `<html>` (`--color-outl-accent`, `--color-outl-ref-link-fg`, ...) and sets `color-scheme` from the luminance of `bg`, so native controls and scrollbars follow. |
| **Mobile** | The shared `installTheme` fetches both sides of the pair, holds both in memory, and swaps on an OS appearance change. |

A colour change in `crates/outl-theme/themes/*.toml` reaches every client without a coordinated edit.

## For contributors

### Adding a built-in theme

Built-ins live in `crates/outl-theme/themes/` and are compiled into the binary.
The format is the one you write as a user.

1. Add `crates/outl-theme/themes/<family>.toml` with `[dark]` and optionally `[light]`.
2. Add one line to `BUILTIN` in `crates/outl-theme/src/registry.rs`.
   Its position is its position in the picker.
3. Update the preset table above and the list in [`config.md`](config.md).
4. Regenerate the browser mock's fixture:
   ```sh
   OUTL_UPDATE_FIXTURES=1 cargo test -p outl-theme --test presets_fixture
   ```

Nothing else needs wiring.
A malformed built-in fails `cargo test`, so an incomplete theme never ships.

### Adding a palette field

A new field is a coordinated change: the field in `Palette`, a value in every preset, a render rule on each client, and a token on the CSS side.
Colour tokens have one meaning on every client ([invariant 13](../CLAUDE.md)).
`the_theme_tokens_match_the_palette` fails when a `--color-outl-*` token has no `Palette` field behind it.

### Desktop CSS custom-property namespaces

`applyPaletteToRoot` (`@outl/shared/theme`) writes the `--color-outl-*` namespace on every theme switch.
`src/styles.css` carries boot-default values so the first painted frame is branded, and an inline `<head>` script replays the last installed palette to avoid a dark flash for light-theme users.
The legacy `--color-ios-*` / `--color-iosd-*` namespace is gone.

Mermaid diagrams map Pele's `--pele-*` properties onto the same set in `@outl/shared/diagram/styles`.
`--pele-surface` is the elevated background and `--pele-surface-alt` is `--color-outl-accent-soft`.
`--pele-series-1` to `--pele-series-8` are accent, accent-alt, bold, tag-link, markdown-link, warn, todo-done and destructive.
`ref-link` is left out because on the brand presets it equals accent and would repeat series 1.
A theme switch repaints a mounted diagram; nothing re-renders it.

### Not built yet

- Theme hot-reload when the config changes.
- `outl theme preview`, to render every preset side by side.
- `:theme` resolving user themes on the TUI.
