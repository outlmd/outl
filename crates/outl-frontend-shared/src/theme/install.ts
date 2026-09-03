/**
 * Theme installation shared by every GUI client (`outl-desktop`,
 * `outl-mobile`).
 *
 * Both sides of the pair are fetched once at boot and held in
 * memory; an OS appearance change swaps tokens locally. Do NOT
 * turn this into a `getTheme()` call inside the media-query
 * listener — that is a backend round-trip at the exact moment the
 * user is watching the screen repaint (RFC 0022, "The opposite
 * direction").
 *
 * Moved out of `outl-mobile/src/lib/theme.ts` (RFC 0022 follow-up)
 * so the desktop can resolve the same `[theme]` pair instead of
 * rendering `theme.preset` unconditionally — a second copy of this
 * logic per client is exactly the drift `@outl/shared` exists to
 * remove.
 */
import { getTheme } from "../api/commands";
import { applyPaletteToRoot } from "./palette";
import type { Palette } from "../api/types";

export type ThemeMode = "light" | "dark" | "auto";

/** Which side of the pair to paint. Pure, so it is testable. */
export function pickSide(mode: ThemeMode, osIsLight: boolean): "light" | "dark" {
  if (mode === "light") return "light";
  if (mode === "dark") return "dark";
  return osIsLight ? "light" : "dark";
}

export async function installTheme(cfg: {
  mode: ThemeMode;
  preset: string;
  presetDark: string;
}): Promise<() => void> {
  // A rejection here must not escape: `onMount` in the caller awaits
  // this before registering `onCleanup(unsubscribe)`, so an uncaught
  // throw would skip that registration entirely (mirrors the
  // desktop's `hydrateTheme` fallback in App.tsx).
  let sides: Record<"light" | "dark", Palette>;
  try {
    const [light, dark] = await Promise.all([
      getTheme(cfg.preset),
      getTheme(cfg.presetDark),
    ]);
    sides = { light, dark };
  } catch {
    try {
      const fallback = await getTheme(null);
      sides = { light: fallback, dark: fallback };
    } catch {
      // No backend at all (shouldn't happen) — keep the static boot
      // frame and skip wiring the OS-appearance listener.
      return () => {};
    }
  }

  const mq = window.matchMedia("(prefers-color-scheme: dark)");
  const paint = () => applyPaletteToRoot(sides[pickSide(cfg.mode, !mq.matches)]);

  paint();
  mq.addEventListener("change", paint);
  return () => mq.removeEventListener("change", paint);
}
