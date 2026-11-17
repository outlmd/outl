/**
 * The About window replaced AppKit's standard panel, which showed a
 * version number and no way to act on it. These pin the two halves
 * that replacement bought: the version is read and shown, and every
 * link in the shared table is a real control that opens its own URL.
 *
 * The labels and URLs are not asserted literally here — `links.rs`'s
 * `the_frontend_mirror_matches_this_table` owns that fact, and a
 * second copy in a test is the drift it exists to prevent.
 */

import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const getVersion = vi.fn();
const openUrl = vi.fn();
const close = vi.fn();
const getThemeConfig = vi.fn();

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: () => getVersion(),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ close: () => close() }),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: (...a: unknown[]) => openUrl(...a),
}));
vi.mock("@outl/shared/api/commands", () => ({
  getThemeConfig: () => getThemeConfig(),
}));
vi.mock("@outl/shared/theme", () => ({
  installTheme: () => Promise.resolve(() => {}),
}));

import { AboutWindow } from "./AboutWindow";
import { HELP_LINKS } from "../lib/links";

let dispose: (() => void) | undefined;

function mount(): HTMLElement {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <AboutWindow />, host);
  return host;
}

const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  getVersion.mockResolvedValue("0.13.0");
  openUrl.mockResolvedValue(undefined);
  getThemeConfig.mockRejectedValue(new Error("no backend"));
  close.mockReset();
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("AboutWindow", () => {
  it("shows the running version once it is read", async () => {
    const host = mount();
    await flush();
    expect(host.textContent).toContain("Version 0.13.0");
  });

  it("says nothing about the version when it cannot be read", async () => {
    getVersion.mockRejectedValue(new Error("not in a webview"));
    const host = mount();
    await flush();
    // A placeholder would be worse than silence: the build number is
    // the one fact this window exists to state correctly.
    expect(host.textContent).not.toContain("Version");
  });

  it("renders one button per link in the shared table", async () => {
    const host = mount();
    await flush();
    const buttons = [...host.querySelectorAll("button")];
    expect(buttons).toHaveLength(HELP_LINKS.length);
    for (const link of HELP_LINKS) {
      expect(buttons.some((b) => b.textContent?.trim() === link.label)).toBe(
        true,
      );
    }
  });

  it("opens the link the button carries", async () => {
    const host = mount();
    await flush();
    const target = HELP_LINKS[1];
    const button = [...host.querySelectorAll("button")].find(
      (b) => b.textContent?.trim() === target.label,
    );
    button?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await flush();
    expect(openUrl).toHaveBeenCalledWith(target.url);
  });

  it("stays open when the browser refuses the link", async () => {
    openUrl.mockRejectedValue(new Error("no browser"));
    const host = mount();
    await flush();
    host.querySelector("button")?.dispatchEvent(
      new MouseEvent("click", { bubbles: true }),
    );
    await flush();
    expect(close).not.toHaveBeenCalled();
  });

  it("closes on Escape", async () => {
    mount();
    await flush();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(close).toHaveBeenCalledTimes(1);
  });
});
