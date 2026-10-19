/**
 * Reporting is on by default, so this strip is what carries prior
 * disclosure to a user who never opens Settings. That makes two
 * properties load-bearing rather than cosmetic, and they pull in
 * opposite directions: it has to be impossible to miss, and it has to
 * be impossible for it to get in the way.
 *
 * `UX.md` principle 2 (never block a keystroke) and principle 6 (no
 * onboarding dismissed before the first bullet) are the second half.
 * These pin it, because the way that regresses is a well-meant
 * `.focus()` on the dismiss button — which reads as "make the way out
 * reachable" in review and reads as "my first keystroke went nowhere"
 * on a device.
 */

import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { CRASH_REPORTS_NOTICE } from "../onboarding";
import { CrashReportsNotice } from "./CrashReportsNotice";
import { CRASH_NOTICE_KEY } from "./firstRun";

let dispose: (() => void) | undefined;

function mount() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <CrashReportsNotice />, host);
  return host;
}

/** The strip's card, or `undefined` when nothing rendered. */
function card(): HTMLElement | undefined {
  return (document.body.querySelector(".outl-crash-notice__card") ??
    undefined) as HTMLElement | undefined;
}

function dismissButton(): HTMLButtonElement | undefined {
  return (document.body.querySelector(".outl-crash-notice__dismiss") ??
    undefined) as HTMLButtonElement | undefined;
}

/** A stand-in for the block the user is typing in. */
function firstBullet(): HTMLTextAreaElement {
  const ta = document.createElement("textarea");
  document.body.appendChild(ta);
  ta.focus();
  return ta;
}

/**
 * happy-dom does not install `localStorage` here, and `safeStore()`
 * reads the global by name — so stub the global rather than giving the
 * component a `store` prop. A test seam in the production signature
 * would mean the path the clients actually run is the one path nothing
 * covers.
 */
function installStore(): Map<string, string> {
  const m = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (k: string) => m.get(k) ?? null,
      setItem: (k: string, v: string) => void m.set(k, v),
      removeItem: (k: string) => void m.delete(k),
      clear: () => m.clear(),
    },
  });
  return m;
}

let store: Map<string, string>;

beforeEach(() => {
  store = installStore();
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  store.clear();
});

describe("the crash-reporting strip", () => {
  it("shows the shared copy on a device that has not dismissed it", () => {
    mount();
    expect(card()?.textContent).toContain(CRASH_REPORTS_NOTICE.body);
    expect(dismissButton()?.textContent).toContain(CRASH_REPORTS_NOTICE.dismiss);
  });

  it("does not take focus from the block the user is typing in", () => {
    const bullet = firstBullet();
    expect(document.activeElement).toBe(bullet);

    mount();

    // Mounting moved nothing. No `autofocus`, no `.focus()`, no trap.
    expect(document.activeElement).toBe(bullet);
    expect(dismissButton()?.hasAttribute("autofocus")).toBe(false);
  });

  it("does not stop the first bullet taking a keystroke", () => {
    const bullet = firstBullet();
    const seen: string[] = [];
    bullet.addEventListener("keydown", (e) => seen.push(e.key));

    mount();
    bullet.dispatchEvent(new KeyboardEvent("keydown", { key: "a", bubbles: true }));
    bullet.value = "a";
    bullet.dispatchEvent(new Event("input", { bubbles: true }));

    expect(seen).toEqual(["a"]);
    expect(bullet.value).toBe("a");
    // Still the caret's, after the keystroke as before it.
    expect(document.activeElement).toBe(bullet);
  });

  it("announces politely instead of interrupting, and is not a dialog", () => {
    // `alert` is assertive and would cut off whatever a screen reader
    // is mid-sentence on; `dialog` would claim a modal this is not.
    mount();
    const strip = document.body.querySelector(".outl-crash-notice");
    expect(strip?.getAttribute("role")).toBe("status");
  });

  it("stays dismissed, including for the next launch", () => {
    mount();
    dismissButton()?.click();
    expect(card()).toBeUndefined();

    // A second mount stands in for the next process: the flag is read
    // back from the store, not from a signal this render still holds.
    dispose?.();
    dispose = undefined;
    mount();
    expect(card()).toBeUndefined();
    expect(store.get(CRASH_NOTICE_KEY)).toBe("done");
  });

  it("renders nothing at all for a device that already dismissed it", () => {
    store.set(CRASH_NOTICE_KEY, "done");
    const host = mount();
    expect(host.querySelector(".outl-crash-notice")).toBeNull();
  });

  it("still shows for someone who dismissed the 30-second tour", () => {
    // The two share a mechanism, not a key. An upgrading user has
    // long since dismissed the tour, and is exactly the person this
    // disclosure has to reach.
    store.set("outl.first-run-tips", "done");
    mount();
    expect(card()).toBeDefined();
  });
});
