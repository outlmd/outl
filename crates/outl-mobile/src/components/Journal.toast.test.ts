/**
 * The toast's two doors, and the trap that made them doors rather than a
 * `tone` signal sitting beside the message.
 *
 * There were already forty-odd `setError(…)` call sites across
 * `Journal.tsx` and its siblings. A separate tone signal means every one
 * of them has to remember to reset it, and the first confirmation leaves
 * `success` behind for the next failure to wear — a green pill saying
 * "could not open page". So the tone is set by whichever door the text
 * came through, and that is what these pin.
 */

import { describe, expect, it, vi } from "vitest";

const haptic = vi.fn();
vi.mock("../lib/haptics", () => ({ haptic: (s: string) => haptic(s) }));

import { createJournalToast } from "./Journal.toast";

describe("createJournalToast", () => {
  it("starts silent, on the error tone", () => {
    const t = createJournalToast();

    expect(t.error()).toBeNull();
    expect(t.toastTone()).toBe("error");
  });

  it("a notice is a success, and buzzes like one", () => {
    const t = createJournalToast();
    t.notify("Renamed. 2 links updated.");

    expect(t.error()).toBe("Renamed. 2 links updated.");
    expect(t.toastTone()).toBe("success");
    expect(haptic).toHaveBeenCalledWith("success");
  });

  /** The whole reason the tone is not a signal of its own. */
  it("a failure after a notice is red again", () => {
    const t = createJournalToast();
    t.notify("Renamed.");
    t.setError("could not open page");

    expect(t.toastTone()).toBe("error");
    expect(t.error()).toBe("could not open page");
  });

  /** A retry belongs to the error it came with: retrying "Renamed." would
   *  re-run something the user cannot see the name of. */
  it("a notice drops a retry left over from an error", () => {
    const t = createJournalToast();
    const retry = vi.fn();
    t.setError("sync failed");
    // Solid reads a bare function as an updater, so the handler is
    // returned from one rather than passed as one.
    t.setErrorRetry(() => retry);
    expect(t.errorRetry()).toBe(retry);
    t.notify("Renamed.");

    expect(t.errorRetry()).toBeNull();
  });

  it("dismissing clears both the message and the retry", () => {
    const t = createJournalToast();
    const retry = vi.fn();
    t.setError("sync failed");
    t.setErrorRetry(() => retry);
    t.dismissToast();

    expect(t.error()).toBeNull();
    expect(t.errorRetry()).toBeNull();
  });

  /** `withError` clears the toast before each attempt; that must not be
   *  mistaken for a confirmation. */
  it("clearing through setError keeps the error tone", () => {
    const t = createJournalToast();
    t.notify("Renamed.");
    t.setError(null);

    expect(t.error()).toBeNull();
    expect(t.toastTone()).toBe("error");
  });
});
