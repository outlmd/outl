# RFC 0022 — One palette owns every colour, and the OS never picks which token

| | |
|---|---|
| **Status** | Accepted |
| **Issue** | [#22](https://github.com/outlmd/outl/issues/22) (+ one to open for the desktop half) |
| **PR** | none yet |
| **Date** | 2026-09-02 |
| **Reference doc** | [`docs/theming.md`](../theming.md) |
| **Invariant** | root `CLAUDE.md` invariant 13 |
| **Guarded by** | `no_client_references_the_legacy_ios_namespace`, `the_theme_tokens_match_the_palette` (`crates/outl-theme/tests/tokens.rs`); `every_preset_defines_destructive`, `is_light_reads_the_canvas_background` (`crates/outl-theme/src/palette.rs`); `outl_light_reads_as_light` (`crates/outl-theme/src/lib.rs`); `a_config_with_only_preset_behaves_exactly_as_before` (`crates/outl-config/src/schema.rs`); `auto_resolves_to_the_dark_side_on_the_tui` (`crates/outl-tui/src/runtime.rs`); `a_configured_pair_has_a_light_side_and_a_dark_side` (`crates/outl-cli/src/cmd/doctor/theme.rs`); `save_keeps_the_theme_pair_but_lets_the_modal_pick_the_preset` (`crates/outl-desktop/src-tauri/src/settings.rs`); `every_listed_preset_resolves`, `theme_name_matches_preset_id` (`crates/outl-tui/src/theme.rs` — pin `outl-light`'s membership in `outl_tui::PRESETS` after the amendment above) |

Sub-project **A** of the client UI/UX convergence effort (A → B → C → D).
B makes capability gaps a compile error, C closes the mobile gaps B enumerates, D aligns the CLI / MCP vocabulary.
Each gets its own RFC; this one covers A.

## Why

`outl-theme::Palette` is described as the shared palette every renderer consumes.
It is not.

| Client | Consumes `Palette`? | How |
|---|---|---|
| TUI | yes | direct crate dependency |
| Desktop | yes | `get_theme` → `applyPaletteToRoot()` rewrites `--color-*` on `<html>` |
| Mobile | **no** | hex literals in `src/styles.css`, switched by Tailwind `dark:` variants |

[`docs/theming.md`](../theming.md) states the opposite:

> the TUI / desktop / mobile clients each turn those hex strings into whatever their renderer expects

A user who picks `dracula` in the desktop's settings and opens the phone gets the brand purple, with no indication that the setting did not travel.
The theme is the most visible shared state in the product and it is the one that does not sync.

### The part that is a live bug, not drift

The desktop carries two token namespaces: `--color-outl-*` (canonical) and `--color-ios-*` / `--color-iosd-*` (legacy, still read by `@outl/shared`'s `MarkdownInline`).

`applyPaletteToRoot()` maps them like this:

```ts
set("--color-ios-bg",  palette.bg);
set("--color-iosd-bg", palette.bg_elev);   // "iosd" means elevated
```

Mobile uses the same prefix for something else:

```css
--color-ios-bg:  #f6f4fb;   /* light */
--color-iosd-bg: #0c0814;   /* "iosd" means dark */
```

`MarkdownInline.tsx` references both namespaces — 18 `ios-`, 17 `iosd-` — and reaches the `iosd` set through Tailwind's `dark:` variant, which resolves off `prefers-color-scheme`.
That is the **operating system**.

So on the desktop, when the OS is in dark mode, `MarkdownInline` swaps to `iosd`, which points at `bg_elev`.
**The OS appearance setting changes the elevation of markdown blocks**, with no relationship to the theme the user picked.
One shared component, one token name, two meanings, and the wrong one is selected by a signal that should have no vote.

Measured surface: 252 `dark:` occurrences across 20 mobile files; 3 files in `@outl/shared`; the legacy block plus ten `set(...)` lines on the desktop.

## What we chose

**`outl_theme::Palette` is the single owner of every colour on every client, and `--color-outl-*` is the only token namespace.**

### The data

`Palette` gains exactly one field: `destructive`.
Every other token the mobile stylesheet defines already has a home:

| Mobile token | `Palette` field |
|---|---|
| `--color-ios-card` | `bg_elev` |
| `--color-ios-divider` | `border` |
| `--color-ios-text` | `fg` |
| `--color-ios-text-secondary` | `fg_dim` |
| `--color-ios-text-tertiary` | `fg_dimmer` |
| `--color-ios-accent` | `accent` |
| `--color-ios-destructive` | **none — the new field** |

`Palette` has `warn`, but nothing for a destructive action; the TUI's delete confirmation and the desktop's `ErrorToast` each pick an ad-hoc colour today.
Being a field makes the compiler force every preset to answer, which is the existing contract.

`tabbar`, `tabbar-border` and the glass/blur values stay out of `Palette` — they are `bg` plus an alpha, derived in CSS with `color-mix()`.
An rgba-with-alpha in a struct the TUI reads is a webview concept pushed into a datatype a terminal consumes, and the terminal has no answer for it.

**Implementation note: a ninth preset, `outl-light`, landed alongside the one field.**
Mobile's `styles.css` hardcoded its own light palette — soft purple-tinted canvas, brand purple accent — because nothing in `outl_theme` carried a light counterpart to the brand `outl` preset.
That hardcoded palette was itself a second definition of colour living outside `Palette`, exactly the shape this RFC exists to eliminate, so the migration moved it in as `outl-light` rather than deleting it.
Without it, every mobile user whose OS reports light mode would have received the dark `outl` preset instead of the light brand look they had before.
**Amendment (2026-09-02): the paragraph above shipped `outl-light` as desktop/mobile only, and that exclusion has been reversed.**
The original decision kept `outl-light` out of `outl-tui`'s own `PRESETS`, reasoning that a terminal has no equivalent of "the brand's light look" distinct from its existing `light` preset.
That reasoning does not survive a look at the same list: `PRESETS` already carries `light` and `logseq-light`, both light palettes a terminal renders without issue, so nothing about `outl-light` is special to a terminal.
The concrete cost of keeping the exclusion was `outl-cli`'s `outl theme list`, which prints `outl_tui::THEME_PRESETS` — the same `PRESETS` list.
So the CLI advertised eight presets while the desktop's Settings picker offered nine, and told a user `outl-light` did not exist while `outl --theme outl-light` resolved it anyway.
Two lists of one fact, disagreeing, is the defect this RFC exists to remove, and the TUI's own preset list had quietly reproduced it in miniature.
`outl_tui::PRESETS` now carries `outl-light`, in the same position it holds in `outl_theme::PRESETS`: second, right after `outl`.

`Palette` does not gain `is_dark`.
`palette.ts::isLightHex()` already answers it (BT.601 luminance over `bg`, used to pick `color-scheme`); the check moves into `outl-theme` as `Palette::is_light()` so `outl doctor`'s pairing validation has a Rust-side answer to call.
This does **not** collapse to one shared answer — `is_light()` is a computed method, not a wire field, so nothing carries it across the Tauri bridge.
The client keeps its own hand-synced copy, `isLightHex` in `crates/outl-frontend-shared/src/theme/palette.ts`, and its doc comment says so.
Two implementations by design, not one — a deliberate outcome of the method-vs-wire-field split, not an oversight.

### Configuration

```rust
pub struct ThemeCfg {
    pub preset: String,               // light side of the pair
    pub preset_dark: Option<String>,  // None resolves to `preset`
    pub mode: ThemeMode,              // Light | Dark | Auto
}

impl ThemeCfg {
    pub fn dark(&self) -> &str {
        self.preset_dark.as_deref().unwrap_or(&self.preset)
    }
}
```

`mode` names **which side of the pair to use**, not a colour:

| `mode` | Resolves to |
|---|---|
| `light` | `preset` |
| `dark` | `dark()` |
| `auto` | `preset` when the OS reports light, `dark()` when it reports dark |

Nothing stops a user putting a dark preset in `preset`, and then `mode = "light"` returns it and the mode name reads like a lie.
That is a misconfigured pair rather than a resolution bug, and `outl doctor` is what says so.

**Backwards compatibility comes from `preset_dark` defaulting to `preset`, not from the `mode` default.**
`mode` defaults to `auto`.
An existing config carrying only `preset = "dracula"` resolves dracula on both sides, so `auto` alternates between dracula and dracula — today's behaviour byte for byte.
Auto starts doing something only once the user sets a second preset, which is when they asked for it.

### Where the code lives

- `outl-desktop/src-tauri/src/commands/theme.rs` → **`outl-tauri-shared/src/commands/theme.rs`**.
  `list_themes` and `get_theme` are pure functions over `outl_theme`; nothing in them is desktop-specific.
  This one move is what makes mobile theming possible, and it closes #22.
- `outl-mobile/src-tauri/Cargo.toml` gains `outl-theme` and registers both commands.
- `applyPaletteToRoot()` moves from `outl-desktop/src/lib/palette.ts` into `@outl/shared` — it stops being desktop-specific the moment mobile calls it.
- `outl-mobile/package.json` declares `@outl/shared`. It imports from it in 21 files today and resolves only through bun workspace hoisting.

The mobile `dark:` variants are **removed, not rewritten**.
Once the resolved preset already matches the OS appearance there is no second variant to express: `bg-(--color-outl-bg)` is correct in both modes because the token's *value* changed, not the class.

On mobile, `auto` fetches both presets at boot and swaps in memory when `prefers-color-scheme` fires.
It must not round-trip to the backend on an appearance change — that is a visible stall at the exact moment the user is watching the screen repaint.

## Why not the alternatives

**Give `Palette` the full mobile chrome vocabulary (`card`, `tabbar`, `divider`, tertiary text).**
Rejected because the mapping is already 1:1 onto existing fields, so it would add eight synonyms and then require every future preset author to keep synonyms consistent.
The presets include terminal palettes — `nord`, `monokai`, `solarized-dark` — where "translucent tab bar" has no meaning a terminal can render, and a field every preset must fill is a field every preset must have an honest answer for.

**Leave the mobile `dark:` variants in place and derive two token sets from the pair.**
Cheaper — it avoids touching 252 lines. Rejected because it keeps two code paths alive for the thing this RFC exists to unify, and the second path is exactly where the `ios`/`iosd` meaning drifted the first time. The churn is mechanical and one-time; the second path is permanent.

**`Palette` gains `is_dark`, and `auto` picks the first light and first dark preset.**
Less configuration. Rejected because the user loses the choice of *which* light and *which* dark: someone who wants `dracula` at night and `logseq-light` by day cannot express it, and that pairing is the whole reason to want auto.

**Default `mode` to an inert value so `auto` is opt-in.**
Rejected because it needs a fourth mode meaning "ignore the other field", and a reader looking at `mode = "fixed"` next to a populated `preset_dark` line cannot tell which one wins. One default doing the work of two is the smaller surface.

## The opposite direction

**Mobile appearance switching gets more expensive.**
Today both palettes sit in CSS and the OS flip costs nothing but a media query.
After this, `auto` holds two `Palette` objects and rewrites tokens on flip.
Held in memory that is a style recalculation, not a fetch — but it is strictly more work, and if someone later "simplifies" it into a backend call the flip becomes a visible stall.
That is why the in-memory requirement is in the design and not filed as an optimisation.

**A shared component's blast radius grows.**
`applyPaletteToRoot()` in `@outl/shared` means a bug there breaks two apps instead of one.
That is the trade already accepted for `outl-actions` and `MarkdownInline`; it is worth naming, not avoiding.

**The desktop will look changed to anyone who adapted to the bug.**
Markdown block elevation stops responding to the OS appearance setting.
That is the fix, and it will be reported as a visual regression by whoever got used to it.

**Nothing here touches reconciliation, sync, or projection.**
No op is added, no `.md` is written, no sidecar is read.
A wrong colour is a wrong colour; it cannot cost a user content.
Recording that explicitly so the next reader knows the question was asked.

**Deleting a colour-only media block can unstyle something, not just retire drift.**
`.outl-skeleton`'s shimmer was a raw rgba gradient with a `prefers-color-scheme` twin duplicating the same two rgba values.
Deleting both media-query copies (the point of this RFC) left the shimmer with no colour at all until it was re-derived from `--color-outl-border` via `color-mix()` — a hardcoded hex removed is not automatically a working default restored.

**A fence that was never themed came along for the ride.**
Mobile's `PluginFence.tsx` read `bg-(--color-ios-fill)/60` (and a `dark:bg-(--color-iosd-fill)/60` twin) — a token **never defined anywhere in the repo's history**, in either namespace.
The fence has therefore never had a real background; Tailwind silently dropped the class the same way it did for the desktop's `ref-link` typo above.
Giving it `--color-outl-bg-elev` is a user-visible change (the fence now actually has a background) that arrives with this refactor rather than being caused by it — worth naming so it isn't mistaken for a Palette regression.

## Who was standing on the old behaviour

Per [invariant 10](../../CLAUDE.md) — who consumed the winner?

- **`MarkdownInline` consumers** relied on `--color-ios-*` existing. Both clients migrate in the same change; the namespace cannot be removed from one side only.
- **The desktop's `dark:` variants** resolve off the OS today. After this the OS selects *which preset*, never *which token name*.
- **`outl --theme <name>` and `:theme <name>`** set a single preset. Under `mode = "auto"` they must set the side matching the current appearance, not clobber both — otherwise a runtime swap silently disables auto.

## Who does not have this

Per [invariant 12](../../CLAUDE.md) — record the gap when it is created.

**`mode = "auto"` on the TUI resolves to `dark()`.**
A terminal does not expose the OS appearance setting, and probing for it (OSC 11, `COLORFGBG`) is unreliable across emulators and multiplexers.
The TUI treats `auto` as `dark`, and that goes in [`docs/client-parity.md`](../client-parity.md) with the user-facing sentence — not in a code comment.
Detecting terminal background is a follow-up issue, not a blocker.

**Both GUI clients now resolve the `mode` / `preset_dark` pair — this section used to report neither did, which is now stale except for one remaining write-side gap.**
All three clients are wired to `outl_config::ThemeCfg`.
The TUI's `resolve_preset_name` reads `mode` and `preset_dark` directly.
The desktop and mobile both call the shared `get_theme_config` command (`outl-tauri-shared::commands::theme`) and feed the resolved pair to the shared `installTheme` (`@outl/shared/theme`, moved there from `outl-mobile/src/lib/theme.ts`).
`installTheme` follows `prefers-color-scheme` when `mode = "auto"`.
`outl doctor` validates the pair regardless of which client wrote it.

**Closed: the desktop Settings modal used to be able to write only `preset`, never `preset_dark` — this is the last gap this RFC left open, and it is now closed.**
The desktop's flat `Settings` DTO (`crates/outl-desktop/src-tauri/src/settings.rs`) now carries `theme_dark` and `theme_mode` alongside `theme`, mapped to/from `ThemeCfg::preset_dark` / `ThemeCfg::mode` in both `From` impls.
`restore_unmodeled_sections` no longer restores either from disk.
The modal owns all three fields now, and restoring any of them would silently discard the user's pick instead of protecting it.
That is the inverse failure mode from before this DTO change, and the reason that function's `[theme]` restore lines were removed rather than extended.
`SettingsModal.tsx` renders a mode selector plus one preset picker per side.
Each picker's live preview reuses `pickSide` — the same function `installTheme` uses — to decide whether an edit to that side should repaint immediately, so there is still exactly one implementation of "which side is on screen".
`docs/theming.md` and `docs/config.md` record the closed state.

## How it cannot regress

**The rule.**
This RFC proposes a new root `CLAUDE.md` invariant, because nothing today states it and the bug above is what its absence produced:

> **A colour token has exactly one meaning, on every client.**
> A token name that resolves to `bg` in one client and `bg_elev` in another is not a naming inconsistency — it is two definitions of one fact, and the wrong one gets selected by whatever signal happens to be wired to it.
> Colours come from `outl_theme::Palette` and reach a client through `applyPaletteToRoot()`. A hex literal in a client stylesheet is a second definition.

**The tests.**
All four land with the implementation; their doc comments must say they exist to fail if the namespace is re-split, or a future reader will tidy them away.

1. **`the_theme_tokens_match_the_palette`** — generates the `@theme` block from `Palette`, fails if a client `styles.css` hardcodes a hex. Same mechanism as `the_parity_doc_matches_the_code`.
2. **`no_client_references_the_legacy_ios_namespace`** — greps every `src/` for `--color-ios`; the deleted namespace does not come back by accident.
3. **`every_preset_defines_destructive`** — the compiler forces the field; this pins that no preset ships an empty string to satisfy it.
4. **`a_configured_pair_has_a_light_side_and_a_dark_side`** — whenever `preset_dark` is set, `Palette::is_light()` must be `true` for `preset` and `false` for `dark()`. Checked for every `mode`, not just `auto`, because `mode = "light"` resolving to a dark preset is the same misconfiguration wearing another label. Reported by **`outl doctor`**, not by failing the boot: a bad theme should not stop a user reaching their notes. A config with no `preset_dark` is not a pair and is never flagged — that is every pre-RFC config, and they are all still correct.

`docs/theming.md` is corrected in the same PR: its claim about all three clients becomes true rather than deleted.

## Scope

Not covered here:

- **New presets or any change to existing hex values.** This is plumbing; a visual redesign is a separate decision.
- **Per-client chrome layout** — sidebar vs page switcher, modals vs sheets. That is sub-project B's vocabulary problem.
- **Typography and spacing scales.** Worth doing, owned by neither this RFC nor B yet.
- **The mobile capability gaps** in [`docs/client-parity.md`](../client-parity.md) — undo/redo ([#14](https://github.com/outlmd/outl/issues/14)), search ([#19](https://github.com/outlmd/outl/issues/19)), block ref ([#18](https://github.com/outlmd/outl/issues/18)). Sub-project C.
- **Detecting terminal background** so the TUI can honour `auto`. Follow-up issue.
