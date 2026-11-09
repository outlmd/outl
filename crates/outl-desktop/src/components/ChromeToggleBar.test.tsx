/**
 * The desktop top bar mirrors the mobile one (#362): same controls, same
 * order, sync dot right before settings in the top-right capsule, and the
 * sidebar toggle alone in the top-left. These pin the order and that each
 * button drives the same store flag / handler as its chord.
 */

import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@outl/shared/api/commands", () => ({
  pluginToolbar: () => Promise.resolve([]),
  pluginRun: vi.fn(),
  peerStatus: () => Promise.resolve([]),
  syncNow: vi.fn(),
  reloadWorkspace: vi.fn(),
}));
vi.mock("../lib/events", () => ({
  onPeerOpsChanged: () => Promise.resolve(() => {}),
}));
vi.mock("../lib/plugin-views", () => ({ playPluginViews: () => {} }));

import { ChromeToggleBar } from "./ChromeToggleBar";
import { ago } from "./SyncIndicator";
import { appState, setAppState } from "../lib/store";

let dispose: (() => void) | undefined;

function mount(handlers = {}) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <ChromeToggleBar handlers={handlers} />, host);
  return host;
}

const labels = (el: Element) =>
  [...el.querySelectorAll("button")].map((b) => b.getAttribute("aria-label"));

afterEach(() => {
  dispose?.();
  dispose = undefined;
  setAppState({
    settingsOpen: false,
    remindersOpen: false,
    marketplaceOpen: false,
    trashOpen: false,
  });
  document.body.innerHTML = "";
});

describe("ChromeToggleBar", () => {
  it("lays the right capsule out in the mobile order", () => {
    const host = mount();
    const right = host.querySelector("[data-testid=chrome-right]")!;
    expect(right.className).toContain("top-3");
    expect(right.className).toContain("right-3");
    const l = labels(right);
    expect(l[0]).toBe("Fold all");
    expect(l[1]).toBe("Unfold all");
    // Trash is desktop-only, so it sits with the other desktop-only
    // controls ahead of the tail the phone also has — the order this
    // test exists to pin is that tail.
    expect(l[2]).toBe("Trash");
    expect(l[3]).toBe("Reminders");
    expect(l[4]).toBe("Plugins");
    expect(l[5]).toMatch(/^Sync/);
    expect(l[6]).toBe("Focus mode");
    expect(l[7]).toBe("Settings");
    expect(l).toHaveLength(8);
  });

  it("flips focus mode on <html> from its capsule button", () => {
    const host = mount();
    const btn = [...host.querySelectorAll("button")].find(
      (b) => b.getAttribute("aria-label") === "Focus mode",
    )!;
    btn.click();
    expect(document.documentElement.getAttribute("data-outl-focus")).toBe("on");
    const back = [...host.querySelectorAll("button")].find(
      (b) => b.getAttribute("aria-label") === "Leave focus mode",
    )!;
    back.click();
    expect(document.documentElement.getAttribute("data-outl-focus")).toBe("off");
  });

  it("keeps only the sidebar toggle on the left and drops the help toggle", () => {
    const host = mount();
    const left = host.querySelector("[data-testid=chrome-left]")!;
    expect(labels(left)).toEqual([
      appState.sidebarOpen ? "Hide sidebar" : "Show sidebar",
    ]);
    expect(labels(host).some((l) => /keyboard shortcuts/i.test(l ?? ""))).toBe(false);
  });

  it("routes each button to its chord's handler or flag", () => {
    const FoldAll = vi.fn();
    const UnfoldAll = vi.fn();
    const host = mount({ FoldAll, UnfoldAll });
    const btn = (l: string) =>
      host.querySelector<HTMLButtonElement>(`button[aria-label="${l}"]`)!;
    btn("Fold all").click();
    btn("Unfold all").click();
    expect(FoldAll).toHaveBeenCalledTimes(1);
    expect(UnfoldAll).toHaveBeenCalledTimes(1);
    btn("Reminders").click();
    expect(appState.remindersOpen).toBe(true);
    btn("Plugins").click();
    expect(appState.marketplaceOpen).toBe(true);
    btn("Settings").click();
    expect(appState.settingsOpen).toBe(true);
  });
});

describe("ago", () => {
  it("reads coarse and never negative", () => {
    expect(ago(-5000)).toBe("just now");
    expect(ago(10_000)).toBe("just now");
    expect(ago(3 * 60_000)).toBe("3 min ago");
    expect(ago(2 * 3_600_000)).toBe("2 h ago");
    expect(ago(3 * 86_400_000)).toBe("3 d ago");
  });
});
