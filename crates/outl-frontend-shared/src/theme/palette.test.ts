import { describe, expect, it, beforeEach } from "vitest";
import { applyPaletteToRoot, paletteDeclarations } from "./palette";
import { BOOT_STYLE_KEY } from "../boot";
import type { Palette } from "../api/types";

const p = (over: Partial<Palette> = {}): Palette =>
  ({
    name: "test",
    bg: "#0c0814",
    fg: "#f4f1fa",
    accent: "#a78bfa",
    border: "#382c54",
    destructive: "#fb7185",
    ...over,
  }) as Palette;

describe("applyPaletteToRoot", () => {
  beforeEach(() => {
    document.documentElement.removeAttribute("style");
  });

  it("writes every field as a --color-outl-* custom property", () => {
    applyPaletteToRoot(p());
    const root = document.documentElement;
    expect(root.style.getPropertyValue("--color-outl-bg")).toBe("#0c0814");
    expect(root.style.getPropertyValue("--color-outl-destructive")).toBe(
      "#fb7185",
    );
  });

  it("kebab-cases snake_case field names", () => {
    applyPaletteToRoot(p({ selected_bullet_bg: "#abcdef" } as Partial<Palette>));
    expect(
      document.documentElement.style.getPropertyValue(
        "--color-outl-selected-bullet-bg",
      ),
    ).toBe("#abcdef");
  });

  it("never writes the legacy ios namespace", () => {
    // RFC 0022 deleted --color-ios-* / --color-iosd-*. On the desktop
    // `iosd` meant "elevated"; on mobile it meant "dark". One shared
    // component read both, and the OS appearance setting decided which
    // — so the OS was silently changing block elevation.
    applyPaletteToRoot(p());
    const style = document.documentElement.getAttribute("style") ?? "";
    expect(style).not.toContain("--color-ios");
  });

  it("sets color-scheme from the palette, not from the OS", () => {
    applyPaletteToRoot(p({ bg: "#ffffff" }));
    expect(document.documentElement.style.colorScheme).toBe("light");
    applyPaletteToRoot(p({ bg: "#0c0814" }));
    expect(document.documentElement.style.colorScheme).toBe("dark");
  });
  it("mirrors Palette::is_light on a malformed hex", () => {
    // `outl_theme::Palette::is_light` (crates/outl-theme/src/palette.rs)
    // is the owner; this is the hand-synced client copy, and the
    // comment above it claims to mirror it *exactly*. Rust's
    // `parse_hex` takes the whole string after `#` and requires it to
    // be 6 or 8 hex digits, so `#ffffffgh` is malformed and falls to
    // the "treat as dark" branch. An unanchored regex here matched
    // the `#ffffff` prefix instead and answered "light" — the exact
    // shape of false assurance invariant 13 is about.
    applyPaletteToRoot(p({ bg: "#ffffffgh" }));
    expect(document.documentElement.style.colorScheme).toBe("dark");
    applyPaletteToRoot(p({ bg: "#fffffff" }));
    expect(document.documentElement.style.colorScheme).toBe("dark");
    applyPaletteToRoot(p({ bg: "not-a-colour" }));
    expect(document.documentElement.style.colorScheme).toBe("dark");
  });

  it("mirrors Palette::is_light on an 8-digit hex", () => {
    // `parse_hex` accepts `#rrggbbaa` and ignores the alpha, so an
    // end anchor that only allowed 6 digits would drift the other
    // way: a light theme shipping dark scrollbars.
    applyPaletteToRoot(p({ bg: "#ffffff00" }));
    expect(document.documentElement.style.colorScheme).toBe("light");
    applyPaletteToRoot(p({ bg: "#0c0814ff" }));
    expect(document.documentElement.style.colorScheme).toBe("dark");
  });
});

describe("the first-frame style cache", () => {
  beforeEach(() => {
    document.documentElement.removeAttribute("style");
  });

  /**
   * The cached string is what the inline `<head>` script assigns before
   * the first pixel, so it has to carry a background on `<html>` — the
   * script runs before `<body>` is parsed and has nowhere else to put
   * one.
   */
  it("carries the background and the tokens on <html>", () => {
    const decls = paletteDeclarations(p({ bg: "#ffffff", fg: "#101010" }));
    expect(decls).toContain("--color-outl-bg:#ffffff");
    expect(decls).toContain("background-color:#ffffff");
    expect(decls).toContain("color:#101010");
    expect(decls).toContain("color-scheme:light");
  });

  it("is what applying a palette stores", () => {
    const store = new Map<string, string>();
    Object.defineProperty(globalThis, "localStorage", {
      value: {
        getItem: (k: string) => store.get(k) ?? null,
        setItem: (k: string, v: string) => void store.set(k, v),
      },
      configurable: true,
      writable: true,
    });

    const palette = p({ bg: "#ffffff" });
    applyPaletteToRoot(palette);
    expect(store.get(BOOT_STYLE_KEY)).toBe(paletteDeclarations(palette));
  });

  /**
   * An OS appearance flip repaints, and a client keeps its own
   * `<html>`-level values there (`--outl-notice-offset`). Replacing
   * `cssText` wholesale used to be safe only because nothing else wrote
   * to it; this keeps it safe by construction.
   */
  it("keeps a non-theme declaration across a repaint", () => {
    document.documentElement.style.setProperty("--outl-notice-offset", "12px");
    applyPaletteToRoot(p());
    expect(
      document.documentElement.style.getPropertyValue("--outl-notice-offset"),
    ).toBe("12px");
    expect(
      document.documentElement.style.getPropertyValue("--color-outl-bg"),
    ).toBe("#0c0814");
  });

  it("replaces a token a later palette changes rather than keeping both", () => {
    applyPaletteToRoot(p({ bg: "#0c0814" }));
    applyPaletteToRoot(p({ bg: "#ffffff" }));
    expect(
      document.documentElement.style.getPropertyValue("--color-outl-bg"),
    ).toBe("#ffffff");
  });
});
