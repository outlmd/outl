/**
 * Install a [`Palette`] returned by the `get_theme` Tauri command
 * as `--color-outl-*` CSS custom properties on `<html>`.
 *
 * This is the single writer of theme tokens for every GUI client
 * (`outl-desktop`, `outl-mobile`). RFC 0022 deleted the legacy
 * `--color-ios-*` / `--color-iosd-*` namespace this used to also
 * write: the desktop mapped `iosd` to "elevated" while mobile mapped
 * it to "dark", and `MarkdownInline` read both through Tailwind's
 * `dark:` variant — so the OS appearance setting was silently
 * deciding block elevation on the desktop, with no relation to the
 * theme the user picked. See `docs/theming.md`.
 */

import type { Palette } from "../api/types";

/**
 * Convert `selected_bullet_bg` → `selected-bullet-bg`. Vite / Tailwind
 * surface custom properties hyphen-delimited; the backend uses snake
 * because Rust + Serde do.
 */
function kebab(snake: string): string {
  return snake.replace(/_/g, "-");
}

export function applyPaletteToRoot(palette: Palette) {
  const root = document.documentElement;
  const set = (prop: string, value: string) =>
    root.style.setProperty(prop, value);

  // Canonical --color-outl-* tokens. Walk every field so new keys
  // added to Palette propagate without extra wiring here.
  for (const [field, value] of Object.entries(palette)) {
    if (field === "name") continue;
    if (typeof value !== "string") continue;
    set(`--color-outl-${kebab(field)}`, value);
  }

  // Body background + foreground — Tailwind utilities like
  // `bg-(--color-outl-bg)` reference the canonical tokens, but the
  // bare `<body>` should pick the palette up too so the boot frame
  // matches the theme before any class hydrates.
  document.body.style.backgroundColor = palette.bg;
  document.body.style.color = palette.fg;

  // Native controls (select dropdowns, scrollbars, checkboxes)
  // follow `color-scheme`, not our CSS variables. styles.css boots
  // with `dark` (brand default); flip it per-palette so a light
  // preset doesn't ship dark scrollbars and a dark select popup.
  root.style.colorScheme = isLightHex(palette.bg) ? "light" : "dark";
}

/**
 * Perceived-luminance check on a `#rrggbb` string (ITU-R BT.601
 * weights). Mirrors `outl_theme::Palette::is_light` exactly — that
 * Rust method is the single owner of "is this a light theme", but it
 * is a computed method, not a wire field, so nothing crosses the
 * Tauri bridge for it. This is the client-side copy kept in sync by
 * hand; used only to pick `color-scheme`. A malformed hex is treated
 * as dark, matching the boot default.
 */
function isLightHex(hex: string): boolean {
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})/i.exec(hex);
  if (!m) return false;
  const [r, g, b] = [m[1], m[2], m[3]].map((c) => parseInt(c, 16));
  return 0.299 * r + 0.587 * g + 0.114 * b > 128;
}
