import { describe, expect, it, beforeEach } from "vitest";
import { applyPaletteToRoot } from "./palette";
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
});
