import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  createDayClock,
  MAX_DAY_CHECK_MS,
  nextDayCheckDelay,
  shouldWakeToToday,
  WAKE_AFTER_MS,
} from "./day-clock";

const base = {
  lastSeenToday: "2026-10-12",
  today: "2026-10-12",
  currentSlug: "inbox",
  hiddenForMs: 60_000,
  editing: false,
};

describe("shouldWakeToToday", () => {
  it("stays put after a short trip away", () => {
    expect(shouldWakeToToday(base)).toBe(false);
  });

  it("returns to today after a long one", () => {
    expect(shouldWakeToToday({ ...base, hiddenForMs: WAKE_AFTER_MS })).toBe(true);
  });

  it("returns to today when the date rolled over, however short the trip", () => {
    expect(
      shouldWakeToToday({ ...base, today: "2026-10-13", currentSlug: "2026-10-12" }),
    ).toBe(true);
  });

  it("never moves an open edit", () => {
    expect(
      shouldWakeToToday({ ...base, hiddenForMs: WAKE_AFTER_MS * 10, editing: true }),
    ).toBe(false);
  });

  it("does nothing when today is already on screen", () => {
    expect(
      shouldWakeToToday({ ...base, currentSlug: "2026-10-12", hiddenForMs: WAKE_AFTER_MS }),
    ).toBe(false);
  });
});

describe("nextDayCheckDelay", () => {
  it("waits until local midnight when that is sooner than the cap", () => {
    const now = new Date(2026, 10, 13, 23, 30, 0, 0);
    expect(nextDayCheckDelay(now)).toBe(30 * 60 * 1000);
  });

  it("never waits longer than the cap, so a configured timezone is still noticed", () => {
    const now = new Date(2026, 10, 13, 8, 29, 0, 0);
    expect(nextDayCheckDelay(now)).toBe(MAX_DAY_CHECK_MS);
  });

  it("is always positive, including at midnight exactly", () => {
    const now = new Date(2026, 10, 13, 0, 0, 0, 0);
    expect(nextDayCheckDelay(now)).toBeGreaterThan(0);
  });
});

describe("createDayClock", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 10, 13, 23, 30, 0, 0));
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  async function flush() {
    await vi.advanceTimersByTimeAsync(0);
  }

  it("relabels when the day rolls over under an app nobody touched", async () => {
    const days = ["2026-11-13", "2026-11-14"];
    let call = 0;
    await createRoot(async (dispose) => {
      const clock = createDayClock({ resolve: async () => days[Math.min(call++, 1)] });
      await flush();
      expect(clock.today()).toBe("2026-11-13");

      // 23:30 → midnight, with no keystroke, no focus change, nothing.
      await vi.advanceTimersByTimeAsync(30 * 60 * 1000);
      expect(clock.today()).toBe("2026-11-14");
      dispose();
    });
  });

  it("keeps the day it has when the backend call fails", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    let call = 0;
    await createRoot(async (dispose) => {
      const clock = createDayClock({
        resolve: async () => {
          if (call++ === 0) return "2026-11-13";
          throw new Error("backend is busy");
        },
      });
      await flush();
      await vi.advanceTimersByTimeAsync(30 * 60 * 1000);
      expect(clock.today()).toBe("2026-11-13");
      dispose();
    });
  });

  it("stops asking once its owner is disposed", async () => {
    const resolve = vi.fn(async () => "2026-11-13");
    await createRoot(async (dispose) => {
      createDayClock({ resolve });
      await flush();
      dispose();
    });
    const calls = resolve.mock.calls.length;
    await vi.advanceTimersByTimeAsync(MAX_DAY_CHECK_MS * 3);
    expect(resolve.mock.calls.length).toBe(calls);
  });

  it("returns to today when the window comes back on a new day", async () => {
    const days = ["2026-11-13", "2026-11-14"];
    let call = 0;
    const jumpToday = vi.fn();
    await createRoot(async (dispose) => {
      createDayClock({
        resolve: async () => days[Math.min(call++, 1)],
        wake: { currentSlug: () => "2026-11-13", editing: () => false, jumpToday },
      });
      await flush();

      window.dispatchEvent(new Event("blur"));
      vi.setSystemTime(new Date(2026, 10, 14, 8, 29, 0, 0));
      window.dispatchEvent(new Event("focus"));
      // Twice: the refresh resolves on the first, the wake decision
      // it chains onto resolves on the second.
      await flush();
      await flush();

      expect(jumpToday).toHaveBeenCalledTimes(1);
      dispose();
    });
  });

  it("never moves a window the user is typing in", async () => {
    const days = ["2026-11-13", "2026-11-14"];
    let call = 0;
    const jumpToday = vi.fn();
    await createRoot(async (dispose) => {
      createDayClock({
        resolve: async () => days[Math.min(call++, 1)],
        wake: { currentSlug: () => "2026-11-13", editing: () => true, jumpToday },
      });
      await flush();

      window.dispatchEvent(new Event("blur"));
      vi.setSystemTime(new Date(2026, 10, 14, 8, 29, 0, 0));
      window.dispatchEvent(new Event("focus"));
      // Twice: the refresh resolves on the first, the wake decision
      // it chains onto resolves on the second.
      await flush();
      await flush();

      expect(jumpToday).not.toHaveBeenCalled();
      dispose();
    });
  });

  it("does not navigate on the midnight tick itself", async () => {
    const days = ["2026-11-13", "2026-11-14"];
    let call = 0;
    const jumpToday = vi.fn();
    await createRoot(async (dispose) => {
      const clock = createDayClock({
        resolve: async () => days[Math.min(call++, 1)],
        wake: { currentSlug: () => "2026-11-13", editing: () => false, jumpToday },
      });
      await flush();
      await vi.advanceTimersByTimeAsync(30 * 60 * 1000);

      // The label is honest now, and the page the user left open is
      // still the page they left open.
      expect(clock.today()).toBe("2026-11-14");
      expect(jumpToday).not.toHaveBeenCalled();
      dispose();
    });
  });
});
