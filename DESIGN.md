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
  accent-soft: "#c4b5fd"
  accent-alt: "#d6ff47"
  warn: "#fbbf24"
  destructive: "#fb7185"
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
    padding: 4px
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

Design reasoning that this file does not own:

- [`docs/theming.md`](docs/theming.md) — how a user picks a theme, all ten presets, per-client consumption.
- [`docs/rfcs/0022-unified-design-tokens.md`](docs/rfcs/0022-unified-design-tokens.md) — why one namespace, rejected alternatives.
- [`docs/shortcuts.md`](docs/shortcuts.md) — every chord.
- [`docs/client-parity.md`](docs/client-parity.md) — generated per-client verdict per action.

## Colors

### The contract, end to end

```
crates/outl-theme/src/palette.rs      Palette { accent: String, … }   ← the only definition
crates/outl-theme/src/presets.rs      pub fn outl() -> Palette        ← ten presets fill every field
        ↓ serde JSON over the Tauri wire (get_theme)
@outl/shared/theme::applyPaletteToRoot  --color-outl-accent: #a78bfa  ← the only writer
        ↓
BlockRow.tsx                          class="bg-(--color-outl-accent)" ← Tailwind v4 shorthand
```

The TUI skips the CSS half entirely: `theme_from_palette` (`crates/outl-tui/src/theme.rs`) turns each
`#rrggbb` into `ratatui::Color::Rgb(r, g, b)` and re-applies a **fixed** modifier formula — hard-coded once,
never per-preset, so only the hues vary between themes:

- `UNDERLINED` on exactly the three link roles (`ref_link`, `tag_link`, `md_link`). They are the only
  "clickable" things in pretty-render mode, and the underline is the affordance.
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

**Accent rail** — the four hues that carry meaning.

| Field | Hex | What it means |
|---|---|---|
| `accent` | `#a78bfa` | Primary. Selection, active state, the ref-link hue, the caret. |
| `accent_soft` | `#c4b5fd` | Lighter accent: caret, property values, italic, help titles. |
| `accent_alt` | `#d6ff47` | Secondary hue, deliberately *not* a shade of `accent`. Code and DONE live here so they never disappear against an accent-painted selection. |
| `warn` | `#fbbf24` | "Look at this" — open TODOs, transient status messages, the highlighter fill. |
| `destructive` | `#fb7185` | "This cannot be undone" — delete confirmations, remove-peer, error toasts. Added by RFC 0022; before it the TUI and the desktop each picked an ad-hoc red and mobile had a token no other client could see. |

`warn` and `destructive` are separate on purpose. Collapsing them is how a "3 blocks archived" toast
ends up the same colour as "delete this page permanently".

**Inline markdown** — `ref_link_fg`, `tag_link_fg`, `md_link_fg`, `bold_fg`, `italic_fg`, `strike_fg`,
`highlight_bg` / `highlight_fg`, `code_fg`.
**TODO prefixes** — `todo_open_fg`, `todo_done_fg`, `todo_done_body_fg`.
**Structural** — `property_key_fg`, `property_value_fg`, `heading_fg`, `dim_fg`.
**Selection / cursor** — `selected_bullet_bg` / `_fg`, `cursor_block_bg` / `_fg`, `cursor_caret_fg`.
**Chrome** — `status_normal_bg` / `_fg`, `status_insert_bg` / `_fg`, `status_visual_bg` / `_fg`,
`status_message_fg`, `list_selected_bg` / `_fg`, `help_title_fg`.

Two pairings worth stating because a preset author will get them wrong:

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

`font-synthesis: none` is load-bearing: `MarkdownInline` renders `**bold**` as `font-semibold` (600),
and a synthesised 600 on a family that has a real one looks smeared next to the real one on the next line.

The scale, as it actually appears in the code:

| Role | Size | Where |
|---|---|---|
| Page title | 28px, `font-mono`, semibold, `leading-[1.15] tracking-tight` | `OutlineView.tsx` — the one place monospace is used at display size, on both the journal and page branches |
| Body | 17px / 1.4 | mobile `body` and `BlockRow` (`text-[17px] leading-[1.42]`); desktop has no CSS base and takes `DesktopSettings.font_size`, default `15` |
| Inline chip, inline code | 14px | `MarkdownInline.tsx` — block-ref chip, `` `code` `` |
| Ref / tag chip (mobile) | 15px, weight 500 | `MarkdownInline.tsx` chip variant |
| Popover / autocomplete row | 13px | `BlockRow.tsx` (desktop) — the four `text-[13px]` dropdowns |
| Secondary meta | 12px | truncated paths, mono |
| Chrome label | 10px, uppercase, mono | code-fence language label |
| Gutter | 9px, mono | `BlockRow.tsx` left gutter |

Monospace is a **role**, not a decoration. It marks "this is a literal": inline code, code-fence language
labels, block ids, file paths, keyboard chords. Prose never uses it.

Weight carries only two values: normal, and `font-semibold` for `**bold**`.
Emphasis beyond that is carried by hue (`italic_fg`, `strike_fg`), which is what lets the TUI express
the same distinctions with `Modifier::ITALIC` and a colour and get an identical reading.

## Layout

The base unit is **4px** (Tailwind's default scale, unmodified). Every spacing value in the clients is a
multiple of it except two deliberate odd numbers, both listed below.

### Outline indentation — 22px per level, on both GUI clients

This is the single most load-bearing measurement in the product, and the two clients arrive at it differently
while agreeing on the number.

Mobile computes it (`crates/outl-mobile/src/components/BlockRow.tsx`):

```ts
const INDENT_PX = 22;
const padLeft = () => 16 + props.depth * INDENT_PX;
```

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

### The two odd numbers

- **3px** — the selected-block rail: `absolute top-[4px] bottom-[4px] -left-[2px] w-[3px] rounded-full bg-(--color-outl-accent)`.
  At 4px it reads as a border; at 2px it disappears on a non-Retina display.
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
// BlockRow.tsx (desktop) — every autocomplete popover
class="absolute top-full left-0 z-30 mt-1 max-h-56 w-72 overflow-y-auto rounded-md
       border border-(--color-outl-border) bg-(--color-outl-bg-elev) py-1 text-[13px] shadow-lg"
```

Translucent chrome derives from `fg`, not from a dedicated token — `bg-(--color-outl-fg)/10`,
`border-(--color-outl-fg)/15`, `hover:bg-(--color-outl-fg)/20`. That is why chrome inverts correctly on a
light preset with no `dark:` variant anywhere.

Mobile adds three non-colour chrome tokens in its `@theme` block, all derived from the palette:

```css
--radius-capsule: 9999px;
--shadow-capsule: 0 4px 16px color-mix(in srgb, var(--color-outl-fg) 10%, transparent);
--blur-chrome: 24px;
```

`--shadow-capsule` composes the active `fg` rather than a frozen `rgba(0,0,0,…)`. A hardcoded black shadow
would be invisible on `outl` (dark canvas) and heavy-handed on `outl-light`.

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
| `rounded` | 0.25rem | inline code, small chips, ghost buttons |
| `rounded-md` | 0.375rem | popovers, ref chips, autocomplete panels |
| `rounded-lg` | 0.5rem | toasts, sheets |
| `rounded-[0.2em]` | em-relative | the `<mark>` highlight — scales with the text it wraps, so a highlight in a 13px popover is not visually rounder than one in 17px body |
| `--radius-capsule` | 9999px | mobile floating capsules (header actions, edit toolbar), the selection rail |
| 28px | absolute | the barcode scan frame, an overlay drawn on a live camera feed and not on a themed surface |

Borders are **hairlines**: `1px` at reduced opacity against a palette token
(`border-(--color-outl-border)/20` for indent guides, full `border-(--color-outl-border)` for popovers).
outl never draws a heavy divider; separation comes from whitespace first, hairline second, shadow third.

### Motion

Two ease curves, defined once in `crates/outl-mobile/src/styles.css`:

```css
--ease-spring-out: cubic-bezier(0.16, 1.08, 0.38, 1);  /* sheets arriving — overshoots a hair */
--ease-spring-in:  cubic-bezier(0.32, 0.72, 0, 1);     /* settles to rest, no bounce */
```

Durations: fade-in 200ms, toast 320ms, sheet 360ms, press 80ms, skeleton loop 1400ms.
Press feedback is `scale(0.96)` + `opacity 0.7` — the smallest transform that still reads as a tap.

## Components

The component surface is deliberately unequal across clients, and that inequality is a recorded fact,
not an accident. `outl_shortcuts::support` and `outl_shortcuts::capability_support`
(`crates/outl-shortcuts/src/`) are two exhaustive `match`es, so **a new `Action` or `Capability` does not
compile until all three clients have declared their verdict**:

```rust
pub enum Support {
    Full,                        // the client performs the action
    Native(&'static str),        // the platform performs it — Backspace on an empty textarea
    Partial(&'static str),       // reachable, not with the full semantics
    Missing(&'static str),       // should exist here, doesn't yet — says what to do instead
    NotApplicable(&'static str), // cannot exist here by construction — says why
}
```

`Native` exists because "reachable" and "has a handler" are different questions, and a boolean would have
forced that row to lie in one direction or the other.
The reason string lives in the catalog, never in the client — a client that writes its own wording is a
fourth copy of the fact. [`docs/client-parity.md`](docs/client-parity.md) is **generated** from those
matches — 78 `Action` rows and 7 `Capability` rows across `Tui` / `Desktop` / `Mobile` — and pinned by
`the_parity_doc_matches_the_code`:

```sh
OUTL_UPDATE_PARITY_DOC=1 cargo test -p outl-shortcuts
```

Two tests police the wording rather than the coverage: `every_degraded_state_explains_itself` rejects an
empty reason, and `nudges_are_written_for_the_user_not_the_developer` bans "unimplemented", "no handler",
"dispatcher" and four more, and requires more than 20 characters. A gap the user can see must be explained
in the user's vocabulary.

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
| `((block-ref))`, orphaned | `rounded bg-(--color-outl-border)/30 px-1 font-mono text-[13px] text-(--color-outl-fg-dim)` | identical |

Two things this table admits. Inline code is tinted by its background only — `code_fg` exists in `Palette`
and in both `@theme` blocks, and `MarkdownInline` does not read it (the TUI does).
And TODO state is **not** painted here: `MarkdownInline` prefixes a bare glyph (`✓ `, `◐ `, `☐ `), while the
hue comes from each client's own `BlockRow` — desktop maps `▣` / `▨` / `▢` to `todo_done_fg` / `todo_open_fg`
and strikes the DONE body with `line-through opacity-60`.

Tailwind only emits these classes because both clients declare
`@source "../../outl-frontend-shared/src/**/*.{ts,tsx}"` in `styles.css`. Without that glob the scanner never
reads the shared component and silently drops every one of its utilities.

Also shared: `ParseWarningsBanner` / `PageAheadOfLogBanner` (invariant 8's refusal must reach the user —
a client that swallows it into a log line ships a page that silently stopped syncing), `PairingQR`,
`PeerList`, and the toolbar action catalog with its most-frequently-used ordering.

### Desktop (`crates/outl-desktop/src/components/`)

Three-pane chrome, mouse + keyboard. `Sidebar` (page list + month calendar), `OutlineView`
(zoom path, page history), `BlockRow` (gutter, fold chevron, indent guides, four autocomplete popovers,
property editor, code fences), `StatusBar` (vim mode badge, transient message), `ErrorToast`,
`SyncPanel` / `SyncIndicator`, `PropertyEditor`, `InlineBacklinks`, `ChromeToggleBar`, `SettingsModal`.

`SettingsModal` owns the entire `[theme]` triple — mode selector plus a picker per side — and each change
reinstalls the draft pair through `installTheme`, so the preview is the real thing rather than an approximation.
Cancel reinstalls the configuration captured when the modal opened.

### Mobile (`crates/outl-mobile/src/components/`)

Single pane, touch. `Journal` (the primary surface — journal-first is the product), `JournalChrome`,
`BlockRow`, `SelectionToolbar`, `KeyboardToolbar`, `Calendar`, `Onboarding`, and a family of bottom sheets
(`DevicesSheet`, `RemindersSheet`, `PropertiesSheet`, `PluginSheet`), each reserving the safe-area inset.

## Do's and Don'ts

**Do** add the field to `Palette` and give all ten presets a value.
**Don't** write a hex literal into a client stylesheet — that is a second definition of a colour.

The one exception is a client's `@theme` boot block, which exists so the first painted frame is branded
before the palette arrives over the wire. Both clients declare the same 19 boot tokens, byte-identical.
Every name in it must reverse-map to a real `Palette` field; `the_theme_tokens_match_the_palette` enforces it.

**Do** derive translucency from a token with `color-mix()` or a Tailwind opacity suffix.
**Don't** put an rgba in `Palette`. The terminal reads that struct and has no answer for an alpha.

**Do** let one token mean one fact everywhere it appears.
**Don't** let a client-local prefix acquire a second meaning — that is the `ios` / `iosd` bug, and the OS
appearance setting is what ended up casting the deciding vote.

**Do** name a field for the surface it paints (`ref_link_fg`).
**Don't** invent compound names (`inner_bold_in_quote`). If two surfaces genuinely share a style, share the field.

**Do** record which clients lack a capability, in `support.rs` / `capability_support.rs`.
**Don't** ship a chord with no handler. `y r` and `:` were listed as desktop chords for months; both were
dead keys that logged to a console the user never opens.

**Do** honour `prefers-reduced-motion` — mobile kills every keyframe animation under it and keeps only the
press feedback, so a tap still confirms itself.
**Don't** reintroduce a `dark:` variant to express a colour. The OS selects *which preset*; it never
selects *which token name*.

### The live exceptions, named

The rule above is the design. These are the places the shipped code does not yet meet it. They are listed
so nobody has to rediscover them, and so nobody cites one as precedent.

| Where | What | Standing |
|---|---|---|
| `@outl/shared/highlight/styles.css` | 11 hex literals — a complete single-theme syntax palette | **Deliberate.** Code blocks read against the brand-dark canvas on every preset; the file says so. A syntax theme is its own vocabulary, not a `Palette` role. |
| `@outl/shared/peers/styles.css` | `#fff` on the pairing QR | **Deliberate.** The quiet zone must stay white or the code stops scanning. |
| mobile `styles.css` `.scan-*`, both `index.html` files | `#fff`, `#000`, raw `rgba()`, `#0c0814` / `#f6f4fb` | **Deliberate.** The scan overlay sits on a live camera feed, not a themed surface; `index.html` values are the pre-JS boot frame, and the desktop's are `var()` *fallbacks* rather than overrides. |
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

## Platform divergence, and why each one exists

| Divergence | Why |
|---|---|
| `default-dark` and `light` bypass the RGB path and build on ANSI named colours (`Color::Reset`, `Color::DarkGray`) | So the user's own terminal palette shows through. That is the point of those two presets, not a gap. |
| The desktop has no character cursor inside the selected block | Its vim mode has only a selected block id. `x`/`X`/`D`/`C`/`s`/`r`/`f`/`F`/`~`/`e` surface one shared status-line nudge, written once in the catalog. |
| Mobile binds no chords | Touch plus an on-screen keyboard. `docs/shortcuts.md` leaves its column blank rather than inventing a row. |
| GUI clients read 28 of the 43 colour fields | The other 15 — `bold_fg`, `italic_fg`, `strike_fg`, `heading_fg`, `dim_fg`, `property_key_fg`, `property_value_fg`, `cursor_block_bg` / `_fg`, `cursor_caret_fg`, `list_selected_bg` / `_fg`, `hint`, `todo_done_body_fg`, `selected_bullet_fg` — are consumed by the TUI only. The GUI expresses those distinctions with weight (`font-semibold`), style (`italic`), opacity (`line-through opacity-70`) and the native caret. The fields stay in `Palette` because the TUI is a first-class client, not a fallback. |
| Only the desktop resolves keystrokes through `outl_shortcuts::lookup()` | The TUI still dispatches Normal mode from its own `match` in `input/normal.rs`. Finishing that migration is open work, not a settled decision. |

## Accessibility

- **Reduced motion** is honoured on mobile: `@media (prefers-reduced-motion: reduce)` sets `animation: none`
  on `.outl-fade-in`, `.outl-sheet-up`, `.outl-toast-in` and `.outl-skeleton`, drops the press `transform`,
  and keeps a 80ms opacity transition so a tap still confirms itself.
- **Every icon-only control carries an `aria-label`** — 159 accessibility attributes across the two GUI
  clients today. Labels are specific, not generic: `Delete page "{name}"`, `Delete ${noun()} ${chip.key}`,
  `Expand` / `Collapse` computed from the block's actual state.
- **Decorative structure is hidden**: indent guides are `aria-hidden="true"`, so a screen reader gets the
  outline's nesting from the DOM tree, not from 22px-wide spacers.
- **Contrast is a preset-author obligation.** `outl-light` darkens the brand violet from `#a78bfa` to
  `#7c3aed` and replaces the lemon `#d6ff47` with `#65a30d` (lime-600) — the comment in
  `crates/outl-theme/src/presets.rs` says the lemon is *"unreadable on light bg"*. Reusing the dark
  palette's hues on a light canvas is the most common preset mistake.
- **Selection is never colour-only.** The selected block gets both an accent rail and a background change;
  vim mode is both a hue and the word (`NORMAL` / `INSERT` / `VISUAL`) in `StatusBar`.
- **Caret and selection are branded**, which is also a legibility choice: `caret-color: var(--color-outl-accent)`
  and `::selection { background: color-mix(in srgb, var(--color-outl-accent) 25%, transparent) }` — the
  default iOS blue caret on a deep-purple canvas is low contrast.

## Adding a token or a preset, end to end

**A new colour role:**

1. Add the field to `Palette` (`crates/outl-theme/src/palette.rs`) with a doc comment saying what the
   surface *is*.
2. Add it to `Palette::fields()` — the installer and the hex test both walk that list.
3. Fill it in all ten presets (`crates/outl-theme/src/presets.rs`). The compiler forces the field to exist;
   `every_preset_defines_destructive` is the pattern for forcing it to *mean* something rather than be `""`.
4. Map it in `crates/outl-tui/src/theme.rs` if the TUI paints it.
5. GUI clients need **no change** — `applyPaletteToRoot` walks every field. Reference it as
   `bg-(--color-outl-<kebab-name>)`.
6. Add a boot value to both `@theme` blocks only if the token must be right on the very first painted frame.
7. Run `/check`. The Rust half and the TypeScript half are both required: invariants 12 and 13 are enforced
   partly by TS parity tests.

**A new preset:**

1. Write the constructor in `presets.rs`; fill every field with `#rrggbb`.
2. Add the name to `PRESETS` in `crates/outl-theme/src/lib.rs` and a match arm to `by_name`
   (case- and separator-insensitive: `Solarized Dark`, `solarized_dark`, `SOLARIZED-DARK` all resolve).
3. Add the one-line TUI delegate in `crates/outl-tui/src/theme.rs`, unless it is deliberately ANSI-based.
4. The CLI, the desktop Settings picker and mobile pick it up automatically — there is no second list.
   There used to be: the TUI kept its own `PRESETS`, so `outl theme list` advertised eight presets while the
   desktop offered nine and told users `outl-light` did not exist while `outl --theme outl-light` resolved it
   anyway. That const was deleted; `outl-tui` now re-exports `outl_theme::PRESETS`.
5. If the preset is meant as one half of a pair, check `Palette::is_light()` agrees —
   `outl doctor` validates that a configured pair has one light side and one dark side.
