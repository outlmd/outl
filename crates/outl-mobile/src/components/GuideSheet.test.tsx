/**
 * The chrome walkthrough (#574).
 *
 * Two things are pinned, and both are decisions rather than markup:
 * **any** way out counts as read (a user who swiped it away has read
 * as much of it as they intend to), and it names the controls a new
 * user actually sees rather than teaching the outline again — which
 * `firstRunTips` already does, on a surface this one does not share.
 */

import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CHROME_TOUR_KEY } from "@outl/shared/polish";

import { CHROME_TOUR } from "../lib/chrome-tour";
import { GuideSheet } from "./GuideSheet";

let dispose: (() => void) | undefined;

/**
 * This runtime ships no `localStorage`, and the component reads it
 * through `@outl/shared/polish`'s defensive accessors rather than
 * taking a store. So the suite installs one: without it both
 * dismissal tests pass on a sheet that never remembered anything.
 */
const store = new Map<string, string>();
beforeEach(() => {
  store.clear();
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
    removeItem: (k: string) => void store.delete(k),
  });
});

function mount() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const onClose = vi.fn();
  dispose = render(() => <GuideSheet open onClose={onClose} />, host);
  return { host, onClose };
}

const settle = () => new Promise((r) => setTimeout(r, 0));

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

describe("GuideSheet", () => {
  it("names every control the tour declares", () => {
    const { host } = mount();
    for (const row of CHROME_TOUR) {
      expect(host.querySelector(`[data-tour="${row.id}"]`), row.id).not.toBeNull();
    }
  });

  it("marks itself read when the button is tapped", async () => {
    const { host, onClose } = mount();
    const got = [...host.querySelectorAll("button")].find(
      (b) => b.textContent?.trim() === "Got it",
    );
    got?.click();
    await settle();

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(store.get(CHROME_TOUR_KEY)).toBe("done");
  });

  /**
   * The half that is easy to leave out: dismissing by the backdrop
   * reads as "I am done with this", and re-showing it next launch
   * would be the app not listening.
   */
  it("marks itself read when it is dismissed instead of finished", async () => {
    const { onClose } = mount();
    const backdrop = document.querySelector(".outl-fade-in") as HTMLElement;
    backdrop.click();
    await settle();

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(store.get(CHROME_TOUR_KEY)).toBe("done");
  });

  /** It explains the chrome; the outline already has its own tour, and
   *  a second copy of that is how the two start disagreeing. */
  it("talks about the chrome, not about blocks and links", () => {
    const { host } = mount();
    const text = host.textContent ?? "";
    expect(text).toContain("Reminders");
    expect(text).toContain("calendar");
    expect(text).not.toContain("[[");
  });
});
