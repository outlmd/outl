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

import { cacheBootStyle } from "../boot";
import type { Palette } from "../api/types";

/**
 * Convert `selected_bullet_bg` → `selected-bullet-bg`. Vite / Tailwind
 * surface custom properties hyphen-delimited; the backend uses snake
 * because Rust + Serde do.
 */
function kebab(snake: string): string {
  return snake.replace(/_/g, "-");
}

/**
 * The `<html>` inline declarations a palette installs, as a CSS string.
 *
 * Pure, so it is both testable and reusable as the boot cache: the
 * inline `<head>` script in each client assigns this exact string to
 * `document.documentElement.style`, which is what paints the user's own
 * palette on the **first** frame instead of the brand default.
 *
 * Built explicitly rather than read back off `root.style.cssText`,
 * because that would also capture whatever else a client has put on
 * `<html>` (`--outl-notice-offset`, `data-`driven layout values) and
 * replay a stale copy of it on the next launch. This is the theme's
 * declarations and nothing else.
 */
export function paletteDeclarations(palette: Palette): string {
  const decls: string[] = [];
  // Canonical --color-outl-* tokens. Walk every field so new keys
  // added to Palette propagate without extra wiring here.
  for (const [field, value] of Object.entries(palette)) {
    if (field === "name") continue;
    if (typeof value !== "string") continue;
    decls.push(`--color-outl-${kebab(field)}:${value}`);
  }
  // Native controls (select dropdowns, scrollbars, checkboxes)
  // follow `color-scheme`, not our CSS variables. styles.css boots
  // with `dark` (brand default); flip it per-palette so a light
  // preset doesn't ship dark scrollbars and a dark select popup.
  decls.push(`color-scheme:${isLightHex(palette.bg) ? "light" : "dark"}`);
  // The boot frame needs a background before any class hydrates, and
  // the inline script has only `<html>` to put it on (`<body>` may not
  // be parsed yet). `applyPaletteToRoot` still paints `<body>` too.
  decls.push(`background-color:${palette.bg}`);
  decls.push(`color:${palette.fg}`);
  return `${decls.join(";")};`;
}

export function applyPaletteToRoot(palette: Palette) {
  const root = document.documentElement;
  const declarations = paletteDeclarations(palette);
  root.style.cssText = mergeDeclarations(root.style.cssText, declarations);

  // Body background + foreground — Tailwind utilities like
  // `bg-(--color-outl-bg)` reference the canonical tokens, but the
  // bare `<body>` should pick the palette up too so the boot frame
  // matches the theme before any class hydrates.
  document.body.style.backgroundColor = palette.bg;
  document.body.style.color = palette.fg;

  // Remember it for the next launch's first frame. The single writer of
  // that cache is here, beside the single writer of the tokens, so the
  // two cannot disagree about what the theme is.
  cacheBootStyle(declarations);
}

/**
 * Overlay `declarations` onto `existing`, keeping whatever `existing`
 * holds that the theme does not own.
 *
 * `root.style.cssText = …` wholesale would drop a client's own
 * `<html>`-level custom properties on every repaint (an OS appearance
 * flip repaints), and `setProperty` in a loop cannot remove a token a
 * narrower palette no longer sets. This keeps both: theme declarations
 * are replaced as a set, everything else survives.
 */
function mergeDeclarations(existing: string, declarations: string): string {
  const ours = new Set(
    declarations
      .split(";")
      .map((d) => d.split(":")[0]?.trim())
      .filter((name): name is string => Boolean(name)),
  );
  const kept = existing
    .split(";")
    .map((d) => d.trim())
    .filter((d) => d.length > 0)
    .filter((d) => {
      const name = d.split(":")[0]?.trim();
      return name ? !ours.has(name) : false;
    });
  return kept.length > 0 ? `${kept.join(";")};${declarations}` : declarations;
}

/**
 * Perceived-luminance check on a `#rrggbb` string (ITU-R BT.601
 * weights). Mirrors `outl_theme::Palette::is_light` exactly — that
 * Rust method is the single owner of "is this a light theme", but it
 * is a computed method, not a wire field, so nothing crosses the
 * Tauri bridge for it. This is the client-side copy kept in sync by
 * hand; used only to pick `color-scheme`. A malformed hex is treated
 * as dark, matching the boot default.
 *
 * "Exactly" is load-bearing, so the regex has to answer the same
 * question `outl_theme::parse_hex` does: the whole string after `#`,
 * 6 or 8 hex digits, nothing else. Hence the end anchor (without it
 * `#ffffffgh` matched its own prefix and came back light, where Rust
 * reads a malformed 8-digit hex and falls to dark) and the optional
 * fourth pair (`parse_hex` accepts `#rrggbbaa` and ignores alpha, so
 * anchoring to 6 alone would drift the opposite way).
 */
function isLightHex(hex: string): boolean {
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})(?:[0-9a-f]{2})?$/i.exec(
    hex,
  );
  if (!m) return false;
  const [r, g, b] = [m[1], m[2], m[3]].map((c) => parseInt(c, 16));
  return 0.299 * r + 0.587 * g + 0.114 * b > 128;
}
