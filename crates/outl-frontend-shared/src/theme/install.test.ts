import { beforeEach, describe, expect, it, vi } from "vitest";

const { getTheme, applyPaletteToRoot } = vi.hoisted(() => ({
  getTheme: vi.fn(async (name: string | null) => ({
    name: name ?? "fallback",
    bg: "#000000",
    fg: "#ffffff",
  })),
  applyPaletteToRoot: vi.fn(),
}));

vi.mock("../api/commands", () => ({ getTheme }));
vi.mock("./palette", () => ({ applyPaletteToRoot }));

import { installTheme, pickSide } from "./install";

describe("installTheme", () => {
  beforeEach(() => {
    getTheme.mockReset();
    getTheme.mockImplementation(async (name: string | null) => ({
      name: name ?? "fallback",
      bg: "#000000",
      fg: "#ffffff",
    }));
    applyPaletteToRoot.mockClear();
  });

  it("replaces the active config and OS listener", async () => {
    const listeners = new Set<() => void>();
    let dark = false;
    vi.spyOn(window, "matchMedia").mockImplementation(
      () =>
        ({
          get matches() {
            return dark;
          },
          addEventListener: (_type: string, listener: () => void) =>
            listeners.add(listener),
          removeEventListener: (_type: string, listener: () => void) =>
            listeners.delete(listener),
        }) as MediaQueryList,
    );

    const disposeOld = await installTheme({
      mode: "auto",
      preset: "old-light",
      presetDark: "old-dark",
    });
    await installTheme({
      mode: "dark",
      preset: "new-light",
      presetDark: "new-dark",
    });
    disposeOld();
    dark = true;
    listeners.forEach((listener) => listener());

    expect(listeners.size).toBe(1);
    expect(applyPaletteToRoot).toHaveBeenLastCalledWith(
      expect.objectContaining({ name: "new-dark" }),
    );
  });

  it("surfaces backend failures to the caller", async () => {
    getTheme.mockRejectedValueOnce(new Error("backend unavailable"));

    await expect(
      installTheme({ mode: "auto", preset: "light", presetDark: "dark" }),
    ).rejects.toThrow("backend unavailable");
    expect(applyPaletteToRoot).not.toHaveBeenCalled();
  });
});

describe("pickSide", () => {
  it("follows the OS in auto", () => {
    expect(pickSide("auto", true)).toBe("light");
    expect(pickSide("auto", false)).toBe("dark");
  });

  it("ignores the OS when the mode is explicit", () => {
    expect(pickSide("light", false)).toBe("light");
    expect(pickSide("dark", true)).toBe("dark");
  });
});
