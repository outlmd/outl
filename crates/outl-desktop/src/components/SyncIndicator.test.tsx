/**
 * The sync dot is the desktop's only "sync now" control (the separate
 * refresh affordance went away in #362). These pin that a click runs one
 * pass, that clicks during a pass are dropped instead of stacking dials,
 * that the dot shows the pass while it runs, and that right-click still
 * reaches Settings → Sync.
 */

import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const peerStatus = vi.fn();
const syncNow = vi.fn();
const reloadWorkspace = vi.fn();

vi.mock("@outl/shared/api/commands", () => ({
  peerStatus: (...a: unknown[]) => peerStatus(...a),
  syncNow: (...a: unknown[]) => syncNow(...a),
  reloadWorkspace: (...a: unknown[]) => reloadWorkspace(...a),
}));
vi.mock("../lib/events", () => ({
  onPeerOpsChanged: () => Promise.resolve(() => {}),
}));

import { SyncIndicator } from "./SyncIndicator";
import { appState, setAppState } from "../lib/store";

let dispose: (() => void) | undefined;

function mount(): HTMLButtonElement {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <SyncIndicator />, host);
  return host.querySelector("button")!;
}

const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  peerStatus.mockResolvedValue([]);
  reloadWorkspace.mockResolvedValue(undefined);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  vi.clearAllMocks();
  setAppState("settingsOpen", false);
  setAppState("lastError", null);
  document.body.innerHTML = "";
});

describe("SyncIndicator", () => {
  it("syncs now on click and reloads the workspace", async () => {
    syncNow.mockResolvedValue(undefined);
    const btn = mount();
    expect(btn.getAttribute("aria-label")).toMatch(/^Sync now/);
    btn.click();
    await flush();
    expect(syncNow).toHaveBeenCalledTimes(1);
    expect(reloadWorkspace).toHaveBeenCalledTimes(1);
    expect(appState.settingsOpen).toBe(false);
  });

  it("shows the pass on the dot and ignores clicks while it runs", async () => {
    let finish!: () => void;
    syncNow.mockReturnValue(new Promise<void>((r) => (finish = r)));
    const btn = mount();
    btn.click();
    await flush();
    expect(btn.querySelector("[data-testid=sync-spinner]")).not.toBeNull();
    expect(btn.getAttribute("aria-busy")).toBe("true");
    btn.click();
    btn.click();
    expect(syncNow).toHaveBeenCalledTimes(1);
    finish();
    await flush();
    await flush();
    expect(btn.querySelector("[data-testid=sync-spinner]")).toBeNull();
    btn.click();
    expect(syncNow).toHaveBeenCalledTimes(2);
  });

  it("surfaces a failed pass and lets the next click retry", async () => {
    syncNow.mockRejectedValueOnce(new Error("no route"));
    const btn = mount();
    btn.click();
    await flush();
    await flush();
    expect(appState.lastError).toBe("no route");
    syncNow.mockResolvedValue(undefined);
    btn.click();
    expect(syncNow).toHaveBeenCalledTimes(2);
  });

  it("shows state and last sync in the hover tooltip", async () => {
    syncNow.mockResolvedValue(undefined);
    const btn = mount();
    await flush();
    expect(btn.title).toContain("Not synced yet this session");
    expect(btn.title).toContain("right-click for Sync settings");
    btn.click();
    await flush();
    await flush();
    expect(btn.title).toContain("Last synced just now");
  });

  it("is dim before the first probe and green once a device answers", async () => {
    // Held by `@outl/shared/peers`, asserted here because this is the client
    // that reads it: a launch must not claim offline over a measurement
    // nobody has taken (#575).
    let answer!: (v: unknown) => void;
    peerStatus.mockReturnValue(new Promise((r) => (answer = r)));
    const btn = mount();
    await flush();
    const dot = () =>
      btn.querySelector("[data-testid=sync-dot]") as HTMLElement | null;
    expect(dot()!.style.background).toContain("--color-outl-fg-dim");
    expect(btn.title).toContain("checking");

    answer([
      {
        node_id: "n1",
        alias: "iPhone",
        online: true,
        rtt_ms: 12,
        pending_ops: 0,
      },
    ]);
    await flush();
    expect(dot()!.style.background).toContain("--color-outl-accent-alt");
    expect(btn.title).toContain("a device is reachable");
  });

  it("opens Sync settings on right-click without syncing", () => {
    const btn = mount();
    btn.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    expect(appState.settingsOpen).toBe(true);
    expect(syncNow).not.toHaveBeenCalled();
  });
});
