---
version: alpha
name: outl
description: Local-first outliner. One Rust-owned palette, three clients — terminal, desktop, mobile.
colors:
  bg: "#0c0814"
  bg-elev: "#15101f"
  fg: "#f4f1fa"
  fg-dim: "#b4adc7"
  fg-dimmer: "#7b7390"
  border: "#382c54"
  hint: "#b4adc7"
  accent: "#a78bfa"
  accent-fg: "#0c0814"
  accent-soft: "#c4b5fd"
  accent-alt: "#d6ff47"
  warn: "#fbbf24"
  destructive: "#fb7185"
  destructive-fg: "#0c0814"
  ref-link-fg: "#a78bfa"
  tag-link-fg: "#f0abfc"
  md-link-fg: "#7dd3fc"
  bold-fg: "#f4f1fa"
  italic-fg: "#c4b5fd"
  strike-fg: "#7b7390"
  highlight-bg: "#fbbf24"
  highlight-fg: "#0c0814"
  code-fg: "#d6ff47"
  todo-open-fg: "#fbbf24"
  todo-done-fg: "#d6ff47"
  todo-done-body-fg: "#7b7390"
  property-key-fg: "#7b7390"
  property-value-fg: "#c4b5fd"
  heading-fg: "#f4f1fa"
  dim-fg: "#7b7390"
  selected-bullet-bg: "#a78bfa"
  selected-bullet-fg: "#0c0814"
  cursor-block-bg: "#f4f1fa"
  cursor-block-fg: "#0c0814"
  cursor-caret-fg: "#c4b5fd"
  status-normal-bg: "#a78bfa"
  status-normal-fg: "#0c0814"
  status-insert-bg: "#d6ff47"
  status-insert-fg: "#0c0814"
  status-visual-bg: "#f0abfc"
  status-visual-fg: "#0c0814"
  status-message-fg: "#fbbf24"
  list-selected-bg: "#a78bfa"
  list-selected-fg: "#0c0814"
  help-title-fg: "#c4b5fd"
typography:
  body:
    fontFamily: -apple-system, BlinkMacSystemFont, "SF Pro Text", "SF Pro Display", system-ui, "Helvetica Neue", Helvetica, Arial, sans-serif
    fontSize: 17px
    lineHeight: 1.4
  block-inline:
    fontFamily: "{typography.body.fontFamily}"
    fontSize: 14px
  ref-chip:
    fontFamily: "{typography.body.fontFamily}"
    fontSize: 15px
    fontWeight: 500
  popover:
    fontFamily: "{typography.body.fontFamily}"
    fontSize: 13px
  code:
    fontFamily: ui-monospace, monospace
    fontSize: 14px
  meta:
    fontFamily: ui-monospace, monospace
    fontSize: 12px
  chrome-label:
    fontFamily: ui-monospace, monospace
    fontSize: 10px
  gutter:
    fontFamily: ui-monospace, monospace
    fontSize: 9px
rounded:
  sm: 0.25rem
  md: 0.375rem
  lg: 0.5rem
  mark: 0.2em
  scan-frame: 28px
  capsule: 9999px
spacing:
  hairline: 3px
  xs: 4px
  sm: 8px
  md: 16px
  indent: 22px
  lg: 24px
components:
  block-row:
    padding: 3px
    textColor: "{colors.fg}"
    typography: "{typography.body}"
  block-row-selected-rail:
    backgroundColor: "{colors.accent}"
    width: 3px
    rounded: "{rounded.capsule}"
  indent-guide:
    backgroundColor: "{colors.border}"
    width: 22px
  bullet-selected:
    backgroundColor: "{colors.selected-bullet-bg}"
    textColor: "{colors.selected-bullet-fg}"
  ref-chip:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent}"
    rounded: "{rounded.md}"
    padding: 6px
    typography: "{typography.ref-chip}"
  code-inline:
    backgroundColor: "{colors.border}"
    rounded: "{rounded.sm}"
    typography: "{typography.code}"
  highlight-mark:
    backgroundColor: "{colors.highlight-bg}"
    textColor: "{colors.highlight-fg}"
    rounded: "{rounded.mark}"
  popover:
    backgroundColor: "{colors.bg-elev}"
    textColor: "{colors.fg}"
    rounded: "{rounded.md}"
    typography: "{typography.popover}"
    width: 288px
    height: 224px
  error-toast:
    backgroundColor: "{colors.bg-elev}"
    textColor: "{colors.status-message-fg}"
    rounded: "{rounded.lg}"
    padding: 8px
  status-badge-normal:
    backgroundColor: "{colors.status-normal-bg}"
    textColor: "{colors.status-normal-fg}"
  status-badge-insert:
    backgroundColor: "{colors.status-insert-bg}"
    textColor: "{colors.status-insert-fg}"
  status-badge-visual:
    backgroundColor: "{colors.status-visual-bg}"
    textColor: "{colors.status-visual-fg}"
  sheet:
    backgroundColor: "{colors.bg-elev}"
    rounded: "{rounded.lg}"
    padding: 16px
---

# outl — design specification

## Overview

outl is a local-first outliner: markdown files on disk, a tree CRDT underneath, and three clients that
paint the same graph — a terminal TUI, a Tauri desktop app, and a Tauri mobile app.

The visual identity is **deep-purple canvas, lavender accent, lemon signal**.
Dark by default (`outl`), with a purple-tinted light counterpart (`outl-light`).
The accent violet `#a78bfa` is the brand; the lemon `#d6ff47` is deliberately a different hue
so code, success and DONE never vanish against a selection painted in the accent.

The one rule that outranks every aesthetic preference:

> **`outl_theme::Palette` is the single owner of every colour on every client.**
> Root [`CLAUDE.md`](CLAUDE.md) invariant 13.

A colour is not a hex you type into a stylesheet.
It is a **field on a Rust struct** (`crates/outl-theme/src/palette.rs`) that every preset must fill,
that the compiler enforces, that rides the Tauri wire as JSON, and that becomes a CSS custom property
named `--color-outl-<kebab(field)>` at runtime.

Fields are named for **what the surface is**, never what it looks like.
`ref_link_fg` means "the foreground of a `[[ref]]`" on all three clients — the TUI underlines it,
the desktop paints it as an underlined link with a 40 %-opacity decoration, mobile renders the same
token as a filled chip. The hue is one fact; the treatment is per-client.

This file owns **what it looks like**.
Its counterpart [`UX.md`](UX.md) owns **what happens** — who the user is, what they are in the middle of,
how an interaction behaves, and what we say when a client cannot deliver it.
An affordance has both halves; when they meet, the visual half is specified here and the behavioural half
links back to it.

Design reasoning that this file does not own:

- [`UX.md`](UX.md) — user and world models, interaction patterns, voice, the domain glossary.
- [`docs/theming.md`](docs/theming.md) — how a user picks a theme, all ten presets, per-client consumption.
- [`docs/rfcs/0022-unified-design-tokens.md`](docs/rfcs/0022-unified-design-tokens.md) — why one namespace, rejected alternatives.
- [`docs/shortcuts.md`](docs/shortcuts.md) — every chord.
- [`docs/client-parity.md`](docs/client-parity.md) — generated per-client verdict per action.

## Colors

### The contract, end to end

```
crates/outl-theme/src/palette.rs        Palette { accent: String, … }  ← the only definition
crates/outl-theme/src/presets.rs        pub fn outl() -> Palette       ← outl's own four presets
crates/outl-theme/src/presets/ports.rs  pub fn dracula() -> Palette    ← the six ports, re-exported
        ↓ serde JSON over the Tauri wire (get_theme)
@outl/shared/theme::applyPaletteToRoot  --color-outl-accent: #a78bfa   ← the only writer
        ↓
BlockRow.tsx                            class="bg-(--color-outl-accent)" ← Tailwind v4 shorthand
```

Forty-five colour fields, ten presets, and the compiler is what makes "every preset fills every field"
a fact rather than a convention: `Palette::fields()` lists all forty-five by name, and three Rust guards
walk that list (`every_palette_field_is_hex`, plus the two token tests below).

The TUI skips the CSS half entirely: `theme_from_palette` (`crates/outl-tui/src/theme.rs`) turns each
`#rrggbb` into `ratatui::Color::Rgb(r, g, b)` and re-applies a **fixed** modifier formula — hard-coded once,
never per-preset, so only the hues vary between themes:

- `UNDERLINED` on the three link roles (`ref_link`, `tag_link`, `md_link`) — the only "clickable" things
  in pretty-render mode, and the underline is the affordance — plus the Insert-mode caret, which reaches
  it through `Theme::cursor_caret_on_char` rather than a palette field. The caret is the one modifier
  applied outside `theme_from_palette`, and it still lives in `theme.rs` so the formula keeps one owner.
  It underlines because it paints the character it sits *before* instead of printing a glyph of its own
  ([#320](https://github.com/outlmd/outl/issues/320)), and a foreground colour paints nothing on a space.
  The two roles never collide on the same cell: a caret on a link char also swaps the hue to
  `cursor_caret_fg` and adds `BOLD`.
- `CROSSED_OUT` on `strike` **and** `todo_done_body`.
- `ITALIC` on `italic` alone.
- `BOLD` on emphasis, cursors and every reverse-video chip: `bold`, `selected_bullet`, `cursor_block`,
  `cursor_caret`, `todo_open`, `todo_done`, `heading`, `status_normal` / `_insert` / `_visual`,
  `help_title`, `list_selected`.

A malformed hex degrades to `Color::Reset` rather than panicking, so a bad config never blocks boot —
`every_palette_field_is_hex` is what catches the typo before it gets that far.

`applyPaletteToRoot` (`crates/outl-frontend-shared/src/theme/palette.ts`) walks `Object.entries(palette)`
rather than naming fields, so a new `Palette` field propagates to both GUI clients with no extra wiring:

```ts
for (const [field, value] of Object.entries(palette)) {
  if (field === "name") continue;
  set(`--color-outl-${kebab(field)}`, value);
}
```

### Roles

Every role below is a real field. Grep it in `crates/outl-theme/src/palette.rs`.
Hex values shown are the `outl` (brand, dark) preset.

**Canvas** — the reading surface.

| Field | Hex | What it means |
|---|---|---|
| `bg` | `#0c0814` | The main canvas. In the two ANSI TUI presets this is `Color::Reset` instead, so the terminal's own background shows through. |
| `bg_elev` | `#15101f` | Anything that floats: popovers, modals, sheets, the picker, toasts. The *only* elevation colour — there is no second step. |
| `fg` | `#f4f1fa` | Body text. Also the source of every translucent chrome layer (`bg-(--color-outl-fg)/10`), so chrome adapts to light and dark presets without a second hue. |
| `fg_dim` | `#b4adc7` | Secondary metadata — timestamps, counts, breadcrumb tails. |
| `fg_dimmer` | `#7b7390` | Placeholders, separators, struck-through DONE bodies, property keys. |
| `border` | `#382c54` | Panel and popover borders, indent guides, the skeleton shimmer's base. |
| `hint` | `#b4adc7` | Footer hint text. Aliases `fg_dim` in the brand presets, but stays a separate field so a preset can pull them apart. |

**Accent rail** — the four hues that carry meaning, plus the two foregrounds that pair with them.

| Field | Hex | What it means |
|---|---|---|
| `accent` | `#a78bfa` | Primary. Selection, active state, the ref-link hue, the caret. |
| `accent_fg` | `#0c0814` | The foreground that reads **on** `accent` — primary buttons, the week strip's selected day, the calendar's selected cell, the mark inside a filled checkbox. Pairs with `accent` exactly as `destructive_fg` pairs with `destructive`. |
| `accent_soft` | `#c4b5fd` | Lighter accent: caret, property values, italic, help titles. |
| `accent_alt` | `#d6ff47` | Secondary hue, deliberately *not* a shade of `accent`. Code and DONE live here so they never disappear against an accent-painted selection. |
| `warn` | `#fbbf24` | "Look at this" — open TODOs, transient status messages, the highlighter fill. |
| `destructive` | `#fb7185` | "This cannot be undone" — delete confirmations, remove-peer, error toasts. Added by RFC 0022; before it the TUI and the desktop each picked an ad-hoc red and mobile had a token no other client could see. |
| `destructive_fg` | `#0c0814` | The foreground **on** `destructive`, where it is a solid fill: the mobile error toast, swipe-to-delete. Neither `#ffffff` nor `bg` could have served as a hardcoded pair — measured over the ten presets, white fails AA on six and the canvas fails on five, and `nord`, `monokai` and `gruvbox` have no passing option between them. |

`warn` and `destructive` are separate on purpose. Collapsing them is how a "3 blocks archived" toast
ends up the same colour as "delete this page permanently".

`destructive_fg` came first and `accent_fg` copied the pattern, which is worth recording because the
pattern is now the default: a hue painted as a solid fill gets a paired foreground, and `highlight_*`,
`selected_bullet_*`, `status_*_*` and `list_selected_*` were all already shaped that way.

`accent_fg` exists because `accent` is authored as **ink** — a hue picked to read against `bg` — and
being a background is a different job. Nine of ten presets resolve `accent_fg` to the canvas, which
is why the clients got away without the field for as long as they did. The tenth is `monokai`, whose
canvas on its pink accent is 3.93:1, under AA; with a real field it picks `#000000` and reaches 5.55:1.
**Never paint `bg` as a text colour on an accent fill** — that is a coincidence, not a pairing, and it
inverts the moment a preset's `bg` stops being the opposite of its `accent`.
`every_fill_pair_passes_contrast` checks both this pair and `accent`-on-`bg`, because the TUI's
`selected_bullet` / `status_normal` / `list_selected` fills do reuse `accent` with the canvas on top.

Twenty-one surfaces were painting that coincidence, and **only eighteen of them were greppable.**
Eighteen spelled it `text-(--color-outl-bg)`, so one search found them. The other three reached the
same place by other spellings and would have survived a search-and-replace: `stroke="white"` on the
checkmark in mobile's `BacklinksSection`, `bg-white` on `Calendar`'s today dot, and
`color: var(--color-outl-bg)` on `.outl-tips-primary` in `@outl/shared/polish`. All twenty-one read
`--color-outl-accent-fg` now, across twenty-four call sites — twenty-two foregrounds on an accent fill
plus the two inverted day-marker dots that paint `accent_fg` as a *background* on an accent tile.
The lesson is in the counting: a role used by coincidence has no single spelling to search for, which
is the argument for the field and not just for the fix.

**Inline markdown** — `ref_link_fg`, `tag_link_fg`, `md_link_fg`, `bold_fg`, `italic_fg`, `strike_fg`,
`highlight_bg` / `highlight_fg`, `code_fg`.
**TODO prefixes** — `todo_open_fg`, `todo_done_fg`, `todo_done_body_fg`.
**Structural** — `property_key_fg`, `property_value_fg`, `heading_fg`, `dim_fg`.
**Selection / cursor** — `selected_bullet_bg` / `_fg`, `cursor_block_bg` / `_fg`, `cursor_caret_fg`.
**Chrome** — `status_normal_bg` / `_fg`, `status_insert_bg` / `_fg`, `status_visual_bg` / `_fg`,
`status_message_fg`, `list_selected_bg` / `_fg`, `help_title_fg`.

Three pairings worth stating because a preset author will get them wrong:

- `accent` / `accent_fg` must read against **each other**. Reaching for the canvas because it usually
  works is the specific mistake this field was added to stop.
- `highlight_bg` / `highlight_fg` must read against **each other**, not against `bg`.
  `outl-light` proves the point: it keeps a bright `#fde68a` fill on a light canvas, with the comment
  *"a real highlighter pen doesn't get darker on light paper"* — it does **not** mirror `outl()`'s
  `warn`-on-`bg` pairing.
- Every `status_*_bg` is paired with a `status_*_fg` equal to `bg`, so mode badges are reverse-video
  chips. Insert mode is lemon, Normal is violet, Visual is magenta — three hues a user reads at a
  glance without reading the word.

### Case: the namespace collision that let the OS pick a colour

Before RFC 0022 the desktop wrote two namespaces:

```ts
set("--color-ios-bg",  palette.bg);
set("--color-iosd-bg", palette.bg_elev);   // desktop: "iosd" means ELEVATED
```

Mobile's stylesheet used the same prefix for something else:

```css
--color-ios-bg:  #f6f4fb;   /* light */
--color-iosd-bg: #0c0814;   /* mobile: "iosd" means DARK */
```

`MarkdownInline.tsx` — one shared component — read both namespaces (18 `ios-`, 17 `iosd-`) and reached
the `iosd` set through Tailwind's `dark:` variant, which resolves off `prefers-color-scheme`.

So on the desktop, **the operating system appearance setting changed the elevation of markdown blocks**,
with no relationship to the theme the user picked. One token name, two meanings, and the wrong one
selected by a signal that had no business voting.

The guard: `no_client_references_the_legacy_ios_namespace` (`crates/outl-theme/tests/tokens.rs`).
It checks *functional use*, not mention — `var(--color-ios…)`, Tailwind's `(--color-ios…)`,
`setProperty("--color-ios…`, and a `--color-ios…:` declaration — because a test that fails on a clean
tree gets deleted by the first person who hits it, taking the real guard with it.

### Case: the boot value that cost the first painted frame

The desktop's `@theme` block declared `--color-outl-ref-link` where the `Palette` field is `ref_link_fg`.

What that did **not** cost: the utility still resolved. Tailwind v4's `text-(--var)` shorthand reads a
CSS custom property at paint time whether or not `@theme` declares it — `--color-outl-status-normal-bg`,
`--color-outl-help-title-fg` and `--color-outl-status-insert-bg` are absent from `@theme` today and
render correctly in the built bundle.

What it actually cost: **the first painted frame**. Before `applyPaletteToRoot()` runs over the wire,
`@theme` is the only source of a value. Refs, tags, markdown links, inline code and TODO markers
rendered with that one property unset for one frame, then repainted correctly.

The guard is `the_theme_tokens_match_the_palette` (same file), which reverse-maps every
`--color-outl-*` name in each client's `@theme` block to a `Palette` field and fails on a name with no owner.
It caught this on its first run.

`every_token_a_component_uses_has_a_palette_field` (same file) runs the same check one layer out, over
every `.ts` / `.tsx` / `.css` in the three client crates rather than the `@theme` blocks, and it carries
an `assert!(scanned > 50)` so a scanner that stops reading the right shape fails loudly instead of
passing on an empty sweep. Both are needed: `@theme` is where a *boot value* goes missing, a component
is where an *invented name* goes in, and Tailwind resolves an invented name to an unset property rather
than a compile error.

### Case: the unlayered reset that outranked every text colour

Mobile's `styles.css` carried this, outside any `@layer`:

```css
button, input, textarea { font-family: inherit; font-size: inherit; color: inherit; }
```

**An unlayered rule beats any layered rule, regardless of specificity.** Tailwind v4's utilities live in
`@layer utilities`. So `color: inherit` here outranked every `text-*` class on a `<button>`, and every
text colour set directly on a button in the mobile client did nothing — not one surface, all of them.
In the built CSS, by byte offset: `@layer utilities{` starts at 7206 and
`.text-\(--color-outl-bg\){color:var(--color-outl-bg)}` sits at 26512 inside it, while
`button,input,textarea{…color:inherit}` sat at 57773, outside every layer.

What a user reported was one tile: the week strip's selected day rendering near-black text on the purple
accent fill. The tell that cracked it is in the same `selected()` branch — the day-marker dot uses
`bg-(--color-outl-accent-fg)` and painted correctly, while the label using
`text-(--color-outl-accent-fg)` did not. The reset touches `color`, never `background-color`.

Two hypotheses were tested and **refuted**, and both are the obvious guess:

- *Tailwind never generated the utility.* It did. A fresh build emits it; a stale `dist/` was the red
  herring, and `dist/` is gitignored, so the artifact a reader inspects is whatever they last built.
- *Solid's `classList` does not split a multi-class key.* It does. The purple fill came from the same
  key as the dead text colour, so the key applied.

The fix was to delete the `color: inherit` line and nothing else. Tailwind's preflight already declares
`button, input, select, optgroup, textarea { font: inherit; color: inherit }` **inside `@layer base`**,
which is the correct position: an author layer still beats the UA sheet's `buttontext`, and a utility
still beats the layer. `font-family` and `font-size` were deliberately left unlayered. They defeat
`text-[16px]` on a button in exactly the same way, and changing that is a typography decision across the
whole app rather than part of this fix — it is a live exception, not an oversight nobody noticed.

Unlayered is not the bug, though, which is why this case sits next to a place the codebase does it on
purpose: `.outl-row-chrome` is unlayered so it can own `opacity` against a Tailwind utility, and that is
correct. What separates the two is the selector, not the layer — see
[Layout → unlayered is a sharp tool](#outline-indentation--22px-per-level-on-both-gui-clients).

## Typography

One family, system-native, declared once as `--font-sans` in each client's `@theme` block and
identical in both:

```css
--font-sans:
  -apple-system, BlinkMacSystemFont, "SF Pro Text", "SF Pro Display",
  system-ui, "Helvetica Neue", Helvetica, Arial, sans-serif;
```

No web fonts. Nothing is fetched at boot, so there is no FOUT and no network dependency in an app whose
whole premise is local-first.

Mobile sets the base in `crates/outl-mobile/src/styles.css`:

```css
body {
  font-family: var(--font-sans);
  font-size: 17px;          /* iOS body size, not 16 */
  line-height: 1.4;
  font-synthesis: none;     /* never fake a weight the family doesn't have */
  text-rendering: optimizeLegibility;
  -webkit-font-smoothing: antialiased;
}
```

`font-synthesis: none` is load-bearing: `MarkdownInline` renders `**bold**` as `font-semibold` (600), and a synthesised 600 on a family that has a real one looks smeared next to the real one on the next line.

The scale, as it actually appears in the code:

| Role | Size | Where |
|---|---|---|
| Page title (desktop) | 24px, semibold, `leading-[1.2] tracking-[-0.011em]` | the `<h1>` is in `OutlineView.tsx`, the words in `PageTitle.tsx`. A journal reads as a day (`Today`, `Yesterday`, `Thursday`) with the full date beside it at 15px in `fg-dim`, from `journalHeading()` in `@outl/shared/journal`; `Today` takes `accent`, the one place the header uses it. The ISO slug moves to the eyebrow, in mono, because it is the file name |
| Journal title (mobile) | 17px, semibold | `JournalHeader.tsx`, same `journalHeading()`, date line at 12px under it on every day so the header height never changes |
| Body | 17px / 1.5 | mobile `BlockBody` and `BlockTextarea`; desktop has no CSS base and takes `DesktopSettings.font_size`, default `15`. The ratio is one shared token, `--outl-block-leading` in `@outl/shared/polish` — the same value on a row and on its edit textarea, so a row does not re-flow when it enters edit mode, and pinned in both clients by `polish/parity.test.ts`. Mobile's `body` still declares 17px / 1.4 as the page default |
| Ref / tag chip, resolved block ref (mobile) | 15px, weight 500 | `MarkdownInline.tsx` `pill` variant |
| Inline code, file / asset chip | 14px | `MarkdownInline.tsx` — `` `code` `` and `FileChip` |
| Popover row, orphaned block ref, embed | 13px | one `SuggestPopup` shell in `SuggestPopups.tsx` serves all four desktop dropdowns; an unresolved `((ref))` is 13px mono |
| Secondary meta | 12px, mono | truncated paths, the mobile date line |
| Desktop secondary chrome | 12.5px | the page eyebrow, the zoom path, sidebar page rows and buttons. Half a pixel, deliberately: at 13px a sidebar row competes with a block, at 12px it reads as disabled |
| Chrome label | 11px | status bar body, popover group headings, `kbd` chips, `.outl-trust-heading`, mobile's `PageHeader` eyebrow (uppercase) |
| Chrome label, mono | 10px, uppercase | the code-fence language label and the vim mode badge |
| Gutter | 9px, mono | `BlockRow.tsx` left gutter. The sidebar calendar's weekday header is 9.5px uppercase, the only other fractional size |

Chrome icons are line SVGs from `@outl/shared/icons`, hand-drawn on a 24px grid with a 1.75 stroke, rendered at 14px by default and stroked in `currentColor` so they take the palette token of the text next to them.
Sixteen of them, in the spirit of Lucide (ISC) rather than copied from it.
`PageGlyph` is the one place that picks a page's icon: its own `icon::` when set, else the calendar or page line icon.
Emoji in chrome are out: they bring their own colours, render differently per OS, and read as content.
That holds for every surface, not just the ones a screenshot caught: the desktop's attach button (`PaperclipIcon`), the link and asset chips in `MarkdownInline` (`PageIcon` / `ImageIcon`), and the namespace rows in `NestedPages`, which asks `PageGlyph` rather than spelling its own `icon || "📄"` fallback.

A journal heading is written in English unless the caller asks for a locale.
`journalHeading()` writes `Today` / `Yesterday` / `Tomorrow` itself, so handing the date to the host's locale put two languages in one line — a macOS pt-BR machine read `Today 12 de outubro de 2026`, and a day further out read `quinta-feira` where the accent test expects `Today`.
The default now formats from the module's own `MONTH_NAMES` / `WEEKDAY_NAMES`, which is also what makes the test deterministic: under `LC_ALL=pt_BR.UTF-8` the old code passed every assertion that named a locale and failed the one that didn't.

The page header carries no rule under it.
A full-width border read as two stacked panes, with the title belonging to the chrome instead of to the outline it names; spacing separates them now.
Page chrome that is not the title — attach a file, page history — sits in the eyebrow, so nothing with a label floats between the title and the first block.

Monospace is a **role**, not a decoration. It marks "this is a literal": inline code, code-fence language
labels, block ids, file paths, keyboard chords, a journal's ISO slug in the eyebrow, and a `((ref))` that
could not resolve. Prose never uses it.

Weight carries only two values: normal, and `font-semibold` for `**bold**`.
Emphasis beyond that is carried by hue (`italic_fg`, `strike_fg`), which is what lets the TUI express
the same distinctions with `Modifier::ITALIC` and a colour and get an identical reading.

## Layout

The base unit is **4px** (Tailwind's default scale, unmodified), and layout reaches for `p-2` / `gap-3` /
`mb-4` before it reaches for a bracket. Twenty-nine arbitrary pixel values survive across the two clients,
almost all of them 1–6px nudges aligning a glyph against text it sits beside — a bullet's `mt-[5px]`, a
hairline's `-left-[2px]`. Two carry a design decision rather than an alignment, and those two are below.

### Outline indentation — 22px per level, on both GUI clients

This is the single most load-bearing measurement in the product, and the two clients arrive at it differently
while agreeing on the number.

Mobile computes it (`crates/outl-mobile/src/components/BlockBody.tsx`):

```ts
export const INDENT_PX = 22;
export function rowPadLeft(depth: number): number {
  return 16 + depth * INDENT_PX;
}
```

It is a function and not an inline expression because `BlockRow` positions the parent-to-children guide
line against the same number, and two copies of `16 + depth * 22` in two files is two chances to move one.

Desktop renders it structurally — one hairline guide element per ancestor level
(`crates/outl-desktop/src/components/BlockRow.tsx`):

```tsx
<For each={Array.from({ length: props.depth })}>
  {() => (
    <span
      aria-hidden="true"
      class="ml-[10px] w-3 shrink-0 self-stretch border-l border-(--color-outl-border)/20"
    />
  )}
</For>
```

`ml-[10px]` + `w-3` (12px) = 22px per level. Mobile's hairline sits at `16 + depth * 22 + 5` px, at 35 % border
opacity; the desktop's at 20 %. Both are `aria-hidden`, and both are deliberately **not** `.outl-row-chrome`.
A guide is a *column* cue, read as a line down the whole page, so revealing it per row makes it flicker under
the pointer and never shows the structure it exists to show. The fold chevron is per-row chrome and does hide.
That is the layout principle in one element: **structure visible on demand, prose visible always.**

`.outl-row-chrome` is defined in the desktop's `styles.css`, unlayered so it wins over Tailwind's utilities:
`opacity: 0` at rest, `1` on `:hover`, `:focus-within`, and the row's `data-selected` / `data-visual` /
`data-editing` states. `focus-within` is load-bearing rather than decorative, since without it the chevron is
unreachable by keyboard and "quieter at rest" becomes "fold is mouse-only". The fade is dropped under
`prefers-reduced-motion`.

It marks the fold chevron and nothing else. The bullet carried it once, which would have hidden the primary
affordance of an outliner; the class had been applied to three elements and only one of them was chrome.

**Unlayered is a sharp tool, and which end you are holding depends on the selector.** An `@layer` rule
always loses to an unlayered one regardless of specificity, so leaving a rule outside a layer is the only
way to own a property that a Tailwind utility also sets. On a class *you* wrote and only put where you
meant it — `.outl-row-chrome` owning `opacity` — that is a deliberate, local override, and the
element it applies to is the element you were thinking about. On an **element selector** it is a silent
killer: it reaches every button, input and textarea in the app, including the ones that do not exist yet,
and the utility it defeats fails by rendering nothing rather than by erroring. That is exactly how
`button { color: inherit }` killed every button text colour on mobile — same mechanism, opposite outcome,
one selector apart. The test is not "is unlayered allowed here" but **"how many elements does this
selector reach, and did I choose all of them?"**

### The two odd numbers

- **3px** — the selected-block rail: `absolute top-[4px] bottom-[4px] -left-[2px] w-[3px] rounded-full bg-(--color-outl-accent)`.
  At 4px it reads as a border; at 2px it disappears on a non-Retina display.
  The same 3px is the desktop row's own vertical padding (`py-[3px]`), which is what keeps a dense outline
  from reading as a list of cards; `.outl-reading` adds 2px more per side when reading rhythm is on.
- **16px** — mobile's base left padding before indentation starts, so depth 0 is not flush to the safe-area edge.

### Viewport and scroll ownership

The desktop pins the height chain to the viewport rather than letting it grow
(`crates/outl-desktop/src/styles.css`, inside `@layer base`):

```css
html, body, #root { height: 100%; }
body, #root { overflow: hidden; }
```

`height: 100%`, not `min-height: 100vh`. With `min-height`, every descendant `h-full` resolves against an
unbounded parent, the outline's inner `overflow-y-auto` concludes it already fits, and the whole document
scrolls instead of the column. **Exactly one element opts into scrolling, explicitly.**

`@layer base` is required so Tailwind's preflight does not win the cascade and paint a white window.

### Safe areas (mobile only)

Every bottom-anchored surface reserves the home indicator, with a floor so it is never flush on a device
that has no inset:

```tsx
style={{ bottom: "max(env(safe-area-inset-bottom), 16px)" }}   // SelectionToolbar.tsx
style="padding-top: max(env(safe-area-inset-top), 12px);"      // JournalChrome.tsx
```

## Elevation & Depth

**There is exactly one elevation step.** `bg` is the canvas; `bg_elev` is everything that floats.
A third level would need a field every one of the ten presets — including `nord`, `monokai`,
`solarized-dark` — has to answer honestly, and a terminal has no answer for "two steps above the canvas".

Depth is carried by **border + shadow + translucency**, never by a second background hue:

```tsx
// SuggestPopups.tsx (desktop) — one shell for all four autocomplete dropdowns
class={`absolute top-full left-0 z-30 mt-1 max-h-56 overflow-y-auto rounded-pop
        border border-(--color-outl-border)/60 bg-(--color-outl-bg-elev) p-1 text-[13px]
        shadow-pop ${props.widthClass}`}
```

Width is the only thing the four differ on, passed as a Tailwind literal so the JIT scanner still sees it:
`w-72` (288px) for page refs, emoji and slash commands, `w-96` (384px) for block search, which shows a
block's text and needs the room. `max-h-56` (224px) is shared. Everything else about the surface — radius,
border, elevation, row size, active-row treatment — is declared once, because four near-identical copies
inside `BlockRow.tsx` is what it used to be, and a fix applied to one had to be re-applied to the other
three by hand.

Translucent chrome derives from `fg`, not from a dedicated token — `bg-(--color-outl-fg)/10`,
`border-(--color-outl-fg)/15`, `hover:bg-(--color-outl-fg)/20`. That is why chrome inverts correctly on a
light preset with no `dark:` variant anywhere.

Desktop has one more, for the popover:

```css
--shadow-pop:
  0 1px 2px color-mix(in srgb, var(--color-outl-fg) 6%, transparent),
  0 8px 24px color-mix(in srgb, var(--color-outl-fg) 10%, transparent);
```

Mobile declares three non-colour chrome tokens in its `@theme` block:

```css
--radius-capsule: 9999px;
--shadow-capsule: 0 4px 16px color-mix(in srgb, var(--color-outl-fg) 10%, transparent);
--blur-chrome: 24px;
```

`--shadow-capsule` composes the active `fg` rather than a frozen `rgba(0,0,0,…)`. A hardcoded black shadow
would be invisible on `outl` (dark canvas) and heavy-handed on `outl-light`. Eleven mobile components read it.

The other two are **declared and consumed by nobody.** `--radius-capsule` has no reference outside its
own declaration — every capsule spells `rounded-full`, which is the same 9999px, so the visual result is
right and the token is not the owner. `--blur-chrome: 24px` likewise: all 41 blurred surfaces use
Tailwind's own `backdrop-blur-sm` / `-md` / `-xl` / `-2xl`. Neither is enforced by anything, because
`the_theme_tokens_match_the_palette` only reverse-maps `--color-outl-*` names. Listed here so the next
person to read the `@theme` block does not infer a system that is not wired up.

Mobile's bottom sheets come in three shapes, all **one** step above the canvas — the difference is how
much they admit about what is under them, not how high they float:

| Shape | Treatment | Who |
|---|---|---|
| Edge-to-edge, translucent | `rounded-t-2xl bg-(--color-outl-bg)/85` (or `/90`), `backdrop-blur-2xl backdrop-saturate-150`, `shadow-2xl` | `Calendar`, `DevicesSheet`, `HistorySheet`, `PageSwitcher`, `TasksSheet` — full-height browsing surfaces, where the blurred outline behind says "you are still on that page" |
| Edge-to-edge, opaque | `rounded-t-2xl bg-(--color-outl-bg-elev)` | `CaptureSheet`, `PluginSheet` — you are typing into this, so nothing behind it should move |
| Inset card | `mx-3 mb-2 rounded-2xl bg-(--color-outl-bg-elev)/95 shadow-[var(--shadow-capsule)] backdrop-blur-2xl` | `SettingsSheet`, `TemplateSheet`, `RemindersSheet`, `PropertiesSheet` — short lists; the 12px side gutter is what makes a card read as a card instead of a truncated sheet |

The inset-card container string is byte-identical across its four, and each pairs with a detached
`mx-3 rounded-2xl` action row below it, iOS action-sheet style.

**Every shape is bounded by the screen, and its rows scroll.**
A sheet is `position: fixed` against `bottom-0`, so its height is its content's and nothing scrolls it.
Content taller than the viewport therefore puts the top of the card above the top edge, unreachable, and
takes the grab handle with it, so the sheet cannot even be dragged shut.
*How* it is bounded follows from where the shape puts the card.
An inset card sits inside a transparent positioned wrapper, so the wrapper takes `max-h-dvh` plus a top
safe-area padding, which costs nothing while the sheet is short.
An edge-to-edge sheet *is* the card, so it takes `max-height: calc(100dvh - max(env(safe-area-inset-top),
12px))` instead: a padding there would open a gap above the handle on every portrait phone, where nothing
was wrong.
A sheet pinned by both `top` and `bottom-0` is already bounded by the gap between them.
`crates/outl-mobile/src/components/sheets.bounded.test.ts` holds the per-sheet table and fails on a new
sheet that declares none of the three.

Landscape is what makes this certain rather than marginal, since both platforms allow it, and it is also
where a width-driven height bites.
`Calendar`'s day cells are `aspect-square` in seven columns, so width drives height: a landscape iPhone
made the columns ~101pt and the grid alone 627pt inside a 402pt viewport.
Hence `max-w-[28rem]` on the grid, which is above every portrait phone's width, so a date cell stays a
date cell without portrait changing at all.

### Case: deleting a hardcoded colour is not the same as restoring a working one

RFC 0022 deleted a `prefers-color-scheme` block that carried the skeleton shimmer's two raw rgba values.
That left the shimmer with **no colour at all** — the shimmer had to be re-derived:

```css
.outl-skeleton {
  background: linear-gradient(90deg,
    color-mix(in srgb, var(--color-outl-border) 40%, transparent) 0%,
    color-mix(in srgb, var(--color-outl-border) 70%, transparent) 50%,
    color-mix(in srgb, var(--color-outl-border) 40%, transparent) 100%);
  background-size: 200% 100%;
  animation: outl-skeleton-shimmer 1400ms ease-in-out infinite;
}
```

`color-mix()` over a palette token is the sanctioned way to express an alpha. `Palette` deliberately does
**not** carry rgba values: an alpha is a webview concept, and the terminal that also reads this struct has
no answer for it.

## Shapes

| Token | Value | Applied to |
|---|---|---|
| `rounded` / `rounded-sm` | 0.25rem | inline code, small chips, ghost buttons, the desktop block row itself (both spellings are 0.25rem in Tailwind v4) |
| `rounded-md` | 0.375rem | ref chips, rows inside a popover |
| `rounded-pop` | 0.5rem | desktop popovers: autocomplete, the property editor, the quick switcher (`Picker`), quick capture. Paired with `shadow-pop`, both in `crates/outl-desktop/src/styles.css`. The active row is `bg-(--color-outl-accent)/15` on `fg` text, not a solid accent fill, so the accent stays a highlight and never becomes a background |
| `rounded-lg` | 0.5rem | the desktop `ErrorToast`, and rows inside a mobile sheet |
| `rounded-2xl` / `rounded-t-2xl` | 1rem | mobile sheets — all four corners on an inset card, top two on an edge-to-edge sheet |
| `rounded-[0.2em]` | em-relative | the `<mark>` highlight — scales with the text it wraps, so a highlight in a 13px popover is not visually rounder than one in 17px body |
| `rounded-full` | 9999px | mobile floating capsules (header actions, edit toolbar), the `Toast` pill, the selection rail. `--radius-capsule` declares the same value and nothing reads it |
| 28px | absolute | the barcode scan frame, an overlay drawn on a live camera feed and not on a themed surface |

Borders are **hairlines**: `1px` at reduced opacity against a palette token
(`border-(--color-outl-border)/20` for indent guides, full `border-(--color-outl-border)` for popovers).
outl never draws a heavy divider; separation comes from whitespace first, hairline second, shadow third.

### Motion

Three ease curves. Two are mobile's own sheet physics, in `crates/outl-mobile/src/styles.css`:

```css
--ease-spring-out: cubic-bezier(0.16, 1.08, 0.38, 1); /* sheets arriving — overshoots a hair */
--ease-spring-in: cubic-bezier(0.32, 0.72, 0, 1);     /* settles to rest, no bounce */
```

The third is shared by both GUI clients, in `@outl/shared/polish/styles.css`, with its two durations:

```css
--outl-motion-fast: 120ms;
--outl-motion-base: 180ms;
--outl-ease-out: cubic-bezier(0.2, 0.8, 0.2, 1);
```

Everything in that file is under 200ms and touches only `opacity` and `transform`, so no reveal, unfold or
drop-target highlight can reflow the outline. That is the rule the shared curve exists to encode; mobile's
two springs are allowed to overshoot because a sheet is a surface arriving, not text moving.

Durations, measured: press 80ms transform / 120ms opacity, row-chrome fade and drop-target 120ms, unfold
and focus-mode fade 180ms, fade-in 200ms, sheet drag-release 220ms, toast 320ms, sheet 360ms, saved-mark
400ms, sync-dot settle 420ms, skeleton loop 1400ms.
Press feedback is `scale(0.96)` + `opacity 0.7` — the smallest transform that still reads as a tap.

## Components

The component surface is deliberately unequal across clients, and that inequality is a recorded fact,
not an accident: `outl_shortcuts::support` and `outl_shortcuts::capability_support` are two exhaustive
`match`es, so a new `Action` or `Capability` does not compile until all three clients declare a verdict,
and [`docs/client-parity.md`](docs/client-parity.md) is generated from them.

What each verdict promises the user, how the nudge is worded, and why the wording lives in the catalog
rather than in a client are [`UX.md` → When a client cannot do the thing](UX.md#when-a-client-cannot-do-the-thing).
What follows here is only what those components look like.

### Shared — `@outl/shared` (`crates/outl-frontend-shared/src/`)

Pure, stateless, identical on both GUI clients. Chrome stays in the client.

`MarkdownInline.tsx` is the single owner of inline token painting. It takes a `variant` prop with two
values — `"inline"` (desktop: underlined text, mouse-hover affordances) and `"pill"` (mobile: filled chips
sized for a fingertip). Same tokens, two treatments, one component:

| Token | `inline` | `pill` |
|---|---|---|
| `[[ref]]` | `text-(--color-outl-ref-link-fg) underline decoration-(--color-outl-ref-link-fg)/40 underline-offset-2 hover:decoration-(--color-outl-ref-link-fg)` — the 40 % decoration makes the underline read as an affordance, not as emphasis, and hover resolves it to full | `rounded-md bg-(--color-outl-accent)/12 px-1.5 py-0.5 text-[15px] font-medium text-(--color-outl-accent) active:opacity-60` |
| `#tag` | same pattern on `--color-outl-tag-link-fg` | `text-(--color-outl-accent) active:opacity-60` |
| `[text](url)` | same pattern on `--color-outl-md-link-fg` | `text-(--color-outl-accent) underline active:opacity-60` |
| `` `code` `` | `rounded bg-(--color-outl-border)/30 px-1 py-0.5 font-mono text-[14px]` | identical |
| `==highlight==` | `<mark class="rounded-[0.2em] bg-(--color-outl-highlight-bg) px-[0.15em] text-(--color-outl-highlight-fg)">` | identical |
| `**bold**` | `font-semibold` — weight only, no colour override | identical |
| `*italic*` | `italic` | identical |
| `~~strike~~` | `line-through opacity-70` | identical |
| `((block-ref))`, resolved | `text-(--color-outl-ref-link-fg) underline decoration-(--color-outl-ref-link-fg)/40 underline-offset-2` — the `[[ref]]` treatment without the `hover:` and without a click handler, since there is nowhere to go | `rounded-md bg-(--color-outl-accent)/12 px-1.5 py-0.5 text-[15px] font-medium text-(--color-outl-accent)`, the ref chip minus `active:opacity-60` for the same reason |
| `((block-ref))`, orphaned | `rounded bg-(--color-outl-border)/30 px-1 font-mono text-[13px] text-(--color-outl-fg-dim)` | identical |
| `!((embed))`, unresolved | `rounded bg-(--color-outl-accent)/12 px-1 font-mono text-[13px] text-(--color-outl-accent)` | identical |

An unresolvable reference drops to **mono** in both variants, which is the one place the monospace role
doubles as a state: a `((uid))` that resolved reads as prose, and one that did not reads as the literal it
has become.

Two more things this table admits. Inline code is tinted by its background only — `code_fg` exists in `Palette`
and in both `@theme` blocks, and `MarkdownInline` does not read it (the TUI does).
And TODO state is **not** painted here: `MarkdownInline` prefixes a bare glyph (`✓ `, `◐ `, `☐ `), while the
hue comes from each client's own `BlockRow` — desktop maps `▣` / `▨` / `▢` to `todo_done_fg` / `todo_open_fg`
and strikes the DONE body with `line-through opacity-60`.

Tailwind only emits these classes because both clients declare
`@source "../../outl-frontend-shared/src/**/*.{ts,tsx}"` in `styles.css`. Without that glob the scanner never
reads the shared component and silently drops every one of its utilities.

Also shared: `ParseWarningsBanner` / `PageAheadOfLogBanner` (invariant 8's refusal must reach the user —
a client that swallows it into a log line ships a page that silently stopped syncing), `PairingQR`,
`PeerList`, `SyncProgressView`, the namespace rows (`NestedPages`, `NamespaceMentions`), the trust surfaces
(`DeviceActivityList`, `MergedEditsList`, `DiffText`, `ExportCard`), `PluginSettings`, the line-icon set, and
the toolbar action catalog with its most-frequently-used ordering.

Six of those directories ship their own `styles.css` — `highlight`, `peers`, `plugins`, `polish`, `trust`,
`warnings` — because a shared component cannot rely on a client's stylesheet existing. `polish` is the one
that carries cross-client **design** rather than one component's chrome: the motion curve, the reading
rhythm (`--outl-read-leading: 1.6`, `--outl-read-measure: 68ch`), the block leading, the focus-mode
treatment, and the first-run tips card. Treat a `--outl-*` name there the way you treat a `--color-outl-*`
name: one owner, one meaning, both clients.

### Desktop (`crates/outl-desktop/src/components/`)

Three-pane chrome, mouse + keyboard. `AppShell` (the grid), `Sidebar` (page list + month calendar),
`OutlineView` (eyebrow, zoom path, title) with `PageTitle`, `BlockRow` / `BlockBody` / `BlockEditor`
(gutter, fold chevron, indent guides, autocomplete via `SuggestPopups`, property editor, code fences via
`CodeFenceView`), `PageSections` (namespace rows), `InlineBacklinks`, `PropertyEditor`, `BatchToolbar`.

Floating and full-pane surfaces: `Picker` (the quick switcher), `QuickCapture`, `HelpOverlay`,
`SettingsModal`, `TimelinePanel` (page history), `RemindersPanel`, `TasksPanel` (the open-task
list beside it), `PluginMarketplace`, `PluginEffectLayer`, `WorkspacePicker`, `Onboarding`,
`TrustSection`.
Status and transient: `StatusBar` (vim mode badge, transient message), `ErrorToast`,
`SyncPanel` / `SyncIndicator`, `ChromeToggleBar`.

`SettingsModal` owns the entire `[theme]` triple — mode selector plus a picker per side — and each change
reinstalls the draft pair through `installTheme`, so the preview is the real thing rather than an approximation.
Cancel reinstalls the configuration captured when the modal opened.

### Mobile (`crates/outl-mobile/src/components/`)

Single pane, touch. `Journal` is the primary surface — journal-first is the product — with `JournalChrome`,
`JournalHeader`, `WeekStrip` (the seven-day scrubber under the title), `JournalZoomBar`, `JournalEmpty`,
`BottomBar` (an accent capsule for the primary action), and `PageSections` for namespace rows.
A row is `BlockRow` wrapping `SwipeRow` around `BlockBody`, which owns the glyphs (`BlockMarkers`) and the
edit textarea (`BlockTextarea`); `BlockContextMenu`, `PullToRefresh` and `SkeletonOutline` sit around them.

Keyboard-adjacent: `KeyboardToolbar`, `KeyboardAccessory`, `SuggesterStrip`, `SelectionToolbar`.
Bottom sheets, each reserving the safe-area inset: `CaptureSheet`, `TasksSheet`, `PageSwitcher`,
`HistorySheet`, `DevicesSheet`, `RemindersSheet`, `PropertiesSheet`, `TemplateSheet`, `PluginSheet`, and
`SettingsSheet` — the newest, and the only one holding preferences rather than workspace content, which is
why it is the one with a navigation row (`Devices and sync`) handing off to `DevicesSheet`.
Plus `Calendar`, `Onboarding`, `ConfirmDialog` / `JournalDeleteDialogs`, `Toast`, `SyncDot`,
`BacklinksSection`, `PluginFence` / `PluginViewOverlay`.

The `Journal.*.ts` siblings beside `Journal.tsx` are extracted behaviour, not components — what they do
belongs to [`UX.md`](UX.md), and nothing in them paints.

## Do's and Don'ts

**Do** add the field to `Palette` and give all ten presets a value.
**Don't** write a hex literal into a client stylesheet — that is a second definition of a colour.

The one exception is a client's `@theme` boot block, which exists so the first painted frame is branded
before the palette arrives over the wire. Both clients declare the same 21 boot tokens, byte-identical —
the same twenty-one names, the same twenty-one hexes, in the same order.
Every name in it must reverse-map to a real `Palette` field; `the_theme_tokens_match_the_palette` enforces it.

**Do** derive translucency from a token with `color-mix()` or a Tailwind opacity suffix.
**Don't** put an rgba in `Palette`. The terminal reads that struct and has no answer for an alpha.

**Do** let one token mean one fact everywhere it appears.
**Don't** let a client-local prefix acquire a second meaning — that is the `ios` / `iosd` bug, and the OS
appearance setting is what ended up casting the deciding vote.

**Do** name a field for the surface it paints (`ref_link_fg`).
**Don't** invent compound names (`inner_bold_in_quote`). If two surfaces genuinely share a style, share the field.

**Do** honour `prefers-reduced-motion` — mobile kills every keyframe animation under it and keeps only the
press feedback, so a tap still confirms itself.
**Don't** reintroduce a `dark:` variant to express a colour. The OS selects *which preset*; it never
selects *which token name*.

**Do** count the elements a selector reaches before leaving a rule outside an `@layer`.
**Don't** put a property an element selector would hand to every button in the app there — a layered rule
loses to an unlayered one regardless of specificity, and the utility it beats fails by painting nothing.

Behavioural do's and don'ts — declaring which clients lack a capability, never shipping a chord with no
handler, where a refusal has to land — are [`UX.md` → Do's and don'ts](UX.md#dos-and-donts).

### The live exceptions, named

The rule above is the design. These are the places the shipped code does not yet meet it. They are listed
so nobody has to rediscover them, and so nobody cites one as precedent.

| Where | What | Standing |
|---|---|---|
| `@outl/shared/highlight/styles.css` | 11 hex literals plus 2 raw `rgba()` — a complete single-theme syntax palette | **Deliberate.** Code blocks read against the brand-dark canvas on every preset; the file says so. A syntax theme is its own vocabulary, not a `Palette` role. |
| `@outl/shared/peers/styles.css` | `#fff` on the pairing QR | **Deliberate.** The quiet zone must stay white or the code stops scanning. |
| mobile `styles.css` `.scan-*`, both `index.html` files | `#fff`, `#000`, raw `rgba()`, `#0c0814` / `#ffffff`, plus `theme-color` metas | **Deliberate.** The scan overlay sits on a live camera feed, not a themed surface; `index.html` values are the pre-JS boot frame, and the desktop's are `var()` *fallbacks* rather than overrides. |
| mobile `styles.css` `button, input, textarea` | `font-family` / `font-size: inherit`, unlayered | **Deliberate, with a cost.** They defeat `text-[16px]` on a button the same way the deleted `color: inherit` defeated every text colour. Changing them is a typography decision across the whole app; see the case above before "fixing" them. |
| `@outl/shared/trust/styles.css` | 7 hex literals, all as `var()` fallbacks (`var(--color-outl-accent, #3b82f6)`, `var(--color-outl-destructive, #c0392b)`) | Same shape as `index.html`, so the same standing — the fallback never wins while a client is themed. Worth noting the hexes are off-brand (Tailwind blue-500, a flat-UI red), so a surface that *did* fall through would not look like outl. |
| `@outl/shared/trust/styles.css` `.outl-merge-undo` | `var(--outl-border, #ccc)` — `--outl-border` is defined nowhere | **A defect, not an exception.** The merge-list Undo button draws a `#ccc` border on every preset. It escapes both token guards because they only reverse-map `--color-outl-*`, and `--outl-border` is missing the `color-` segment. The real token is `--color-outl-border`. |
| mobile `bg-white` | the two toggle knobs (`SettingsSheet`, `RemindersSheet`), the QR card in `DevicesSheet`, the `Toast` action button's `bg-white/20` | The QR card is the same quiet-zone rule as above. The knobs and the toast overlay are a named colour standing in for a role — `accent_fg` is what a knob on an accent track wants, and the toast already paints `destructive_fg` next to its white button. |
| `ErrorToast.tsx` | uses `--color-outl-status-message-fg`, absent from both `@theme` blocks | The utility resolves; only the first painted frame is unstyled. Same shape as the `ref-link` incident above. |

## Theming — light, dark, and who resolves it

`[theme]` takes three keys, owned by [`docs/theming.md`](docs/theming.md):

```toml
[theme]
preset = "outl-light"    # the light side
preset_dark = "outl"     # the dark side; falls back to `preset` when absent
mode = "auto"            # light | dark | auto (default)
```

`mode` names **which side of the pair to use**, not a colour.
Backwards compatibility comes from `preset_dark` defaulting to `preset`, not from the `mode` default:
a config carrying only `preset = "dracula"` resolves dracula on both sides, so `auto` alternates between
dracula and dracula — today's behaviour byte for byte.

Both GUI clients call the shared `get_theme_config` command, hold **both** `Palette` objects in memory, and
repaint locally on a `prefers-color-scheme` flip. The in-memory requirement is part of the design, not an
optimisation: a backend round-trip mid-repaint is a visible stall at the exact moment the user is watching.

`applyPaletteToRoot` also flips `color-scheme` from the palette's BT.601 luminance over `bg`, so native
scrollbars and `<select>` popups follow the preset.

**`mode = "auto"` resolves to the dark side on the TUI, always.** A terminal has no API for the OS
appearance setting; probing (OSC 11, `COLORFGBG`) is wrong under tmux and unimplemented in several emulators.
The TUI declares the gap rather than guessing — a permanent behaviour, recorded in `docs/client-parity.md`.

## App icon

One master, `assets/source-icon.png`, and one generator, `scripts/gen-icons.py`.
Everything else under `assets/` is generated from it: the inset macOS master, the opaque square iOS and
the App Store take, and the monochrome variant.

The platforms do not want the same picture, which is the reason a generator exists rather than a copy.
macOS draws app icons on a 1024 canvas where the tile occupies 824 and the rest is transparent margin, so
every icon in the Dock shares a silhouette and an optical size.
An icon that fills its canvas edge to edge is not bigger by mistake, it is drawn at the size its
neighbours reserve for margin, and outl shipped exactly that until a user said so.
Windows and Linux apply no mask and reserve no margin, where the same padding would make outl *smaller*
than its neighbours, so the margin goes on the `.icns` alone and the `.ico` and loose PNGs stay full
bleed.
iOS and the App Store take an opaque square with no alpha channel at all, because the OS applies its own
mask and an icon that arrives pre-rounded gets rounded twice.

**The generated sets stay inside the client that ships them**, and the duplication is deliberate rather
than unresolved.
The two Tauri sets are byte-identical and `scripts/check-icons.py` fails in CI if they stop being, which
is the guard the drift actually needed: before the generator, both crates carried the same sixteen
filenames from different runs and all sixteen differed byte for byte, with nothing able to notice.
A shared crate was considered and is the wrong scope — `outl-tauri-shared` is the *Tauri* backend, while
these files are consumed by Xcode (`Assets.xcassets`), Gradle (`mipmap-*`), a web manifest
(`outl-mobile/public/`) and the README, none of which can reference a Rust crate's directory.
The masters already live one level broader than that, at the repo root, which is the scope that covers
every consumer.
Two copies of a generated artifact with a check on them is not two sources of truth; the master is, and
there is one.

## Platform divergence, and why each one exists

Visual divergence only.
Divergence in *what a client does* — no chords on mobile, no character cursor on the desktop, no automatic
backups on iOS — is [`UX.md` → What each client can do](UX.md#what-each-client-can-do).

| Divergence | Why |
|---|---|
| `default-dark` and `light` bypass the RGB path and build on ANSI named colours (`Color::Reset`, `Color::DarkGray`) | So the user's own terminal palette shows through. That is the point of those two presets, not a gap. |
| GUI clients reference 29 of the 45 colour fields | The other 16 — `hint`, `bold_fg`, `italic_fg`, `strike_fg`, `todo_done_body_fg`, `property_key_fg`, `property_value_fg`, `heading_fg`, `dim_fg`, `selected_bullet_bg` / `_fg`, `cursor_block_bg` / `_fg`, `cursor_caret_fg`, `list_selected_bg` / `_fg` — are consumed by the TUI only. The GUI expresses those distinctions with weight (`font-semibold`), style (`italic`), opacity (`line-through opacity-70`) and the native caret. The fields stay in `Palette` because the TUI is a first-class client, not a fallback. Measured as the distinct `--color-outl-*` names in the three client crates' non-test source; one of the 29, `code_fg`, appears only as a boot value in the two `@theme` blocks and is painted by nothing. |
| Only the desktop resolves keystrokes through `outl_shortcuts::lookup()` | The TUI still dispatches Normal mode from its own `match` in `input/normal.rs`. Finishing that migration is open work, not a settled decision. |

## Accessibility

- **Reduced motion** is honoured in all three stylesheets that animate, not just mobile's. Mobile sets
  `animation: none` on `.outl-fade-in`, `.outl-sheet-up`, `.outl-toast-in` and `.outl-skeleton`, drops the
  press `transform`, and keeps an 80ms opacity transition so a tap still confirms itself. The desktop drops
  the `.outl-row-chrome` fade but keeps the reveal, because the chevron still has to appear. `@outl/shared/polish`
  drops `.outl-unfold`, `.outl-sync-settle` and the tips-card entrance, plus every focus-mode transition.
  The pattern across all three: **the end state is never the thing removed.**
- **A keyboard focus ring exists and is branded**, on the desktop:
  `:where(button, a[href], [role="button"], [role="toolbar"] *, [tabindex]):focus-visible` gets
  `outline: 2px solid var(--color-outl-accent)` with a 2px offset. `:focus-visible` rather than `:focus`,
  so a mouse click never paints it and Tab, the arrow keys and F6 always do.
- **Whether an affordance can be reached at all** — labelling, hidden-but-keyboard-reachable chrome, touch
  targets — is the behavioural half: [`UX.md` → Accessibility is a reach question](UX.md#accessibility-is-a-reach-question).
  176 `aria-*` attributes across the two GUI clients' non-test components today.
- **Contrast is a preset-author obligation.** `outl-light` darkens the brand violet from `#a78bfa` to
  `#6d28d9` (violet-700, 7.1:1 on its white canvas) and replaces the lemon `#d6ff47` with `#4d7c0f`
  (lime-700, 4.99:1 on the canvas), dark enough that insert mode's white label clears AA. Most hues in that
  preset carry their measured ratio in a trailing comment, which is the form a preset's contrast argument
  should take: a number next to the hex, not a claim in a doc. Reusing the dark palette's hues on a light
  canvas is the most common preset mistake.
- **Selection is never colour-only.** The selected block gets both an accent rail and a background change;
  vim mode is both a hue and the word (`NORMAL` / `INSERT` / `VISUAL`) in `StatusBar`.
- **Caret and selection are branded**, which is also a legibility choice: `caret-color: var(--color-outl-accent)`
  and `::selection { background: color-mix(in srgb, var(--color-outl-accent) 25%, transparent) }` — the
  default iOS blue caret on a deep-purple canvas is low contrast.

## Adding a token or a preset, end to end

**A new colour role:**

1. Add the field to `Palette` (`crates/outl-theme/src/palette.rs`) with a doc comment saying what the
   surface *is*.
2. Add it to `Palette::fields()`. Three Rust guards walk that list — `every_palette_field_is_hex`,
   `the_theme_tokens_match_the_palette`, `every_token_a_component_uses_has_a_palette_field` — so a field
   left out of it is a field nothing checks. The GUI installer does *not* read it; `applyPaletteToRoot`
   walks `Object.entries(palette)` on the TypeScript side instead.
3. Fill it in all ten presets — `crates/outl-theme/src/presets.rs` for outl's own four,
   `crates/outl-theme/src/presets/ports.rs` for the six ports. The compiler forces the field to exist;
   `every_preset_defines_destructive` is the pattern for forcing it to *mean* something rather than be `""`.
   If the field is a foreground meant to sit on a fill, add the pair to `fill_pairs()` in
   `crates/outl-theme/tests/contrast.rs` — that is the only test that knows which two fields land on top
   of each other, and a field can be a valid hex with a real token and still be unreadable.
4. Map it in `crates/outl-tui/src/theme.rs` if the TUI paints it.
5. GUI clients need **no change** — `applyPaletteToRoot` walks every field. Reference it as
   `bg-(--color-outl-<kebab-name>)`.
6. Add a boot value to both `@theme` blocks only if the token must be right on the very first painted frame.
7. Run `/check`. The Rust half and the TypeScript half are both required: invariants 12 and 13 are enforced
   partly by TS parity tests.

**A new preset:**

1. Write the constructor in `presets.rs` (or `presets/ports.rs`, if it is a port of another editor's
   scheme — those are re-exported, so `presets::dracula()` resolves either way); fill every field with
   `#rrggbb`.
2. Add the name to `PRESETS` in `crates/outl-theme/src/lib.rs` and a match arm to `by_name`
   (case- and separator-insensitive: `Solarized Dark`, `solarized_dark`, `SOLARIZED-DARK` all resolve).
3. Add the one-line TUI delegate in `crates/outl-tui/src/theme.rs`, unless it is deliberately ANSI-based.
4. The CLI, the desktop Settings picker and mobile pick it up automatically — there is no second list.
   There used to be: the TUI kept its own `PRESETS`, so `outl theme list` advertised eight presets while the
   desktop offered nine and told users `outl-light` did not exist while `outl --theme outl-light` resolved it
   anyway. That const was deleted; `outl-tui` now re-exports `outl_theme::PRESETS`.
5. If the preset is meant as one half of a pair, check `Palette::is_light()` agrees —
   `outl doctor` validates that a configured pair has one light side and one dark side.
