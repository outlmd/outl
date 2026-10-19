import { describe, expect, it, vi } from "vitest";
import { createRoot, createSignal } from "solid-js";

import presets from "../mock/presets.json";
import { contrastRatio } from "./contrast";
import { TIPS_KEY, dismissOnce, dismissedOnce, firstRunTips, isBlankOutline } from "./firstRun";
import { createFocusMode, FOCUS_KEY } from "./focus";
import { createSettle } from "./settle";

const memStore = () => {
  const m = new Map<string, string>();
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
};

describe("reading contrast", () => {
  it("covers all ten presets", () => {
    expect(presets).toHaveLength(10);
  });
  it.each(presets.map((p) => [p.name, p] as const))("%s body text clears 4.5:1", (_n, p) => {
    expect(contrastRatio(p.fg, p.bg)).toBeGreaterThanOrEqual(4.5);
    // Popovers sit on bg_elev. Solarized's base0 on base02 is 4.1 by
    // design, so the floor there is 4:1 rather than repainting a classic.
    expect(contrastRatio(p.fg, p.bg_elev)).toBeGreaterThanOrEqual(4);
  });
  it.each(presets.map((p) => [p.name, p] as const))("%s dim text clears 4.5:1", (_n, p) => {
    // fg_dim carries hints, dates and metadata people still have to read.
    expect(contrastRatio(p.fg_dim, p.bg)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(p.fg_dim, p.bg_elev)).toBeGreaterThanOrEqual(4);
  });
  it("is symmetric and 21 for black on white", () => {
    expect(contrastRatio("#000000", "#ffffff")).toBeCloseTo(21, 5);
    expect(contrastRatio("#ffffff", "#000000")).toBeCloseTo(21, 5);
  });
});

describe("focus mode", () => {
  it("starts off, mirrors on <html> and persists", () => {
    const store = memStore();
    createRoot((dispose) => {
      const f = createFocusMode(store);
      expect(f.on()).toBe(false);
      expect(document.documentElement.getAttribute("data-outl-focus")).toBe("off");
      f.toggle();
      expect(f.on()).toBe(true);
      expect(document.documentElement.getAttribute("data-outl-focus")).toBe("on");
      expect(store.getItem(FOCUS_KEY)).toBe("on");
      dispose();
    });
  });
  it("restores the saved preference", () => {
    const store = memStore();
    store.setItem(FOCUS_KEY, "on");
    createRoot((dispose) => {
      expect(createFocusMode(store).on()).toBe(true);
      dispose();
    });
  });
  it("keeps working when storage throws", () => {
    const bad = { getItem: () => null, setItem: () => { throw new Error("full"); } };
    createRoot((dispose) => {
      const f = createFocusMode(bad);
      expect(() => f.toggle()).not.toThrow();
      expect(f.on()).toBe(true);
      dispose();
    });
  });
});

describe("first-run tips", () => {
  it("teaches blocks, links and daily in that order on both clients", () => {
    for (const c of ["desktop", "mobile"] as const) {
      expect(firstRunTips(c).map((t) => t.id)).toEqual(["blocks", "links", "daily"]);
    }
  });
  it("uses the client's own gesture", () => {
    expect(firstRunTips("desktop")[0].body).toContain("Tab");
    expect(firstRunTips("mobile")[0].body).toContain("indent keys");
    // Swiping a row deletes it on mobile; the tip must never teach that.
    expect(firstRunTips("mobile")[0].body).not.toContain("swipe");
    expect(firstRunTips("desktop")[2].body).toContain("⌘J");
  });
  it("treats a fresh day with one empty block as blank, and nothing else", () => {
    const b = (text: string, children: unknown[] = []) => ({ text, children });
    expect(isBlankOutline([])).toBe(false);
    expect(isBlankOutline([b("  ")])).toBe(true);
    expect(isBlankOutline([b(""), b("")])).toBe(true);
    expect(isBlankOutline([b("hi")])).toBe(false);
    expect(isBlankOutline([b("", [b("x")])])).toBe(false);
  });
  it("stays dismissed", () => {
    const s = memStore();
    expect(dismissedOnce(TIPS_KEY, s)).toBe(false);
    dismissOnce(TIPS_KEY, s);
    expect(dismissedOnce(TIPS_KEY, s)).toBe(true);
  });
});

describe("sync settle", () => {
  it("pulses once when a pass ends, not when one starts", () => {
    vi.useFakeTimers();
    const [busy, setBusy] = createSignal(false);
    const { settling, dispose } = createRoot((dispose) => ({ settling: createSettle(busy, 100), dispose }));
    {
      setBusy(true);
      expect(settling()).toBe(false);
      setBusy(false);
      expect(settling()).toBe(true);
      vi.advanceTimersByTime(100);
      expect(settling()).toBe(false);
      dispose();
    }
    vi.useRealTimers();
  });
});
