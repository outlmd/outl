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

export interface ThemeConfig {
  mode: ThemeMode;
  preset: string;
  presetDark: string;
}

let latestRequest = 0;
let activeInstallation = 0;
let removeActiveListener: (() => void) | undefined;

export async function installTheme(cfg: ThemeConfig): Promise<() => void> {
  const request = ++latestRequest;
  // Unknown preset names already fall back in the Rust command. Transport
  // failures must escape so Settings can tell the user that its preview or
  // saved palette was not installed; boot callers own their static-frame
  // fallback.
  const [light, dark] = await Promise.all([
    getTheme(cfg.preset),
    getTheme(cfg.presetDark),
  ]);
  const sides: Record<"light" | "dark", Palette> = { light, dark };

  if (request !== latestRequest) return () => {};

  removeActiveListener?.();
  removeActiveListener = undefined;
  const installation = ++activeInstallation;
  const mq = window.matchMedia("(prefers-color-scheme: dark)");
  const paint = () => applyPaletteToRoot(sides[pickSide(cfg.mode, !mq.matches)]);
  const remove = () => mq.removeEventListener("change", paint);

  paint();
  mq.addEventListener("change", paint);
  removeActiveListener = remove;

  return () => {
    if (installation !== activeInstallation) return;
    remove();
    removeActiveListener = undefined;
  };
}
