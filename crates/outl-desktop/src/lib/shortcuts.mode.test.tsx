/**
 * Property: a keystroke typed into a focused editor is typed, never
 * run as a block command, whatever `appState.mode` holds (issue #489).
 *
 * `exitVisual()` leaves `mode` at `"vim-normal"` and nothing writes it
 * back, so for the rest of the session `c` folded the block being
 * typed in and `dd` deleted it. These tests drive the real dispatcher
 * rather than `detectMode`, because the user-visible fact is "the key
 * reached the textarea", not which label the function returned.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Binding } from "./api";

vi.mock("./api", async (importOriginal) => {
  const real = await importOriginal<typeof import("./api")>();
  const ch = (value: string) => ({ mods: 0, key: { kind: "Char", value } });
  const bindings: Binding[] = [
    { chord: [ch("c")], mode: "normal", action: { kind: "ToggleCollapsed" }, description: "" },
    { chord: [ch("d"), ch("d")], mode: "normal", action: { kind: "DeleteBlock" }, description: "" },
    { chord: [ch("x")], mode: "visual", action: { kind: "DeleteRange" }, description: "" },
    { chord: [ch("c")], mode: "overlay", action: { kind: "OpenPicker" }, description: "" },
  ] as Binding[];
  return {
    ...real,
    listShortcutBindings: vi.fn(async () => bindings),
    listActionSupport: vi.fn(async () => []),
    pluginKeybindings: vi.fn(async () => []),
  };
});

vi.mock("@outl/shared/api/commands", () => ({ pluginRun: vi.fn() }));

import { render } from "solid-js/web";

import { StatusBar } from "../components/StatusBar";
import { installShortcuts } from "./shortcuts";
import { setAppState } from "./store";

const fired: string[] = [];
let uninstall: () => void;
let textarea: HTMLTextAreaElement;

/** Returns whether the page was allowed to handle the key itself. */
function press(key: string): boolean {
  const e = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
  (document.activeElement ?? document.body).dispatchEvent(e);
  return !e.defaultPrevented;
}

beforeEach(async () => {
  fired.length = 0;
  vi.spyOn(console, "info").mockImplementation(() => {});
  uninstall = await installShortcuts({
    OpenPicker: () => void fired.push("OpenPicker"),
    ToggleCollapsed: () => void fired.push("ToggleCollapsed"),
    DeleteBlock: () => void fired.push("DeleteBlock"),
    DeleteRange: () => void fired.push("DeleteRange"),
  });
  textarea = document.createElement("textarea");
  document.body.appendChild(textarea);
  setAppState({ mode: "normal", pickerOpen: false, settingsOpen: false, helpOpen: false });
});

afterEach(() => {
  uninstall();
  textarea.remove();
  vi.restoreAllMocks();
});

describe("typing in a focused editor", () => {
  for (const stale of ["vim-normal", "normal", "edit", "vim-insert"] as const) {
    it(`is never a block command when mode is ${stale}`, () => {
      setAppState("mode", stale);
      textarea.focus();
      const reachedTextarea = ["c", "d", "d", "x"].map(press);
      expect(reachedTextarea).toEqual([true, true, true, true]);
      expect(fired).toEqual([]);
    });
  }

  it("is never a block command inside a plain input either", () => {
    const input = document.createElement("input");
    document.body.appendChild(input);
    setAppState("mode", "vim-normal");
    input.focus();
    expect(press("c")).toBe(true);
    expect(fired).toEqual([]);
    input.remove();
  });
});

describe("what the focus rule must not break", () => {
  it("vim-normal with nothing focused still runs Normal-mode commands", () => {
    setAppState("mode", "vim-normal");
    textarea.blur();
    expect(press("c")).toBe(false);
    expect(fired).toEqual(["ToggleCollapsed"]);
  });

  it("vim-normal with nothing focused still buffers a two-key chord", () => {
    setAppState("mode", "vim-normal");
    textarea.blur();
    press("d");
    press("d");
    expect(fired).toEqual(["DeleteBlock"]);
  });

  it("vim-visual outranks focus, since a range has no textarea", () => {
    setAppState("mode", "vim-visual");
    textarea.blur();
    expect(press("x")).toBe(false);
    expect(fired).toEqual(["DeleteRange"]);
  });

  it("an open overlay outranks everything, including a focused input", () => {
    setAppState({ mode: "vim-normal", pickerOpen: true });
    textarea.focus();
    // Only the overlay binding for `c` exists besides Normal's; with the
    // overlay branch deleted the key would be typed and nothing fires.
    expect(press("c")).toBe(false);
    expect(fired).toEqual(["OpenPicker"]);
  });
});

describe("the status badge and the dispatcher agree", () => {
  const cases = [
    { name: "stale vim-normal, editor focused", mode: "vim-normal", focus: true, picker: false, badge: "INSERT", cFires: [] },
    { name: "vim-normal, nothing focused", mode: "vim-normal", focus: false, picker: false, badge: "NORMAL", cFires: ["ToggleCollapsed"] },
    { name: "vim-visual, editor focused", mode: "vim-visual", focus: true, picker: false, badge: "VISUAL", cFires: [] },
    { name: "picker open, editor focused", mode: "vim-normal", focus: true, picker: true, badge: "OVERLAY", cFires: ["OpenPicker"] },
  ] as const;

  for (const c of cases) {
    it(c.name, () => {
      setAppState({ mode: c.mode, pickerOpen: c.picker });
      if (c.focus) textarea.focus();
      else textarea.blur();

      const host = document.createElement("div");
      document.body.appendChild(host);
      const dispose = render(() => <StatusBar />, host);
      try {
        expect(host.querySelector("span")?.textContent).toBe(c.badge);
        press("c");
        expect(fired).toEqual(c.cFires);
      } finally {
        dispose();
        host.remove();
      }
    });
  }
});
