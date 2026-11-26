/**
 * Visual `y`, `y`, `p` through the real dispatcher.
 *
 * In Visual, `y` alone yanks and leaves Visual. A vim hand types `yy`
 * anyway, and without `yank-echo` the stray second `y` armed Normal's
 * `y y` / `y r` prefix, so the `p` after it completed `[y, p]`, matched
 * nothing and was thrown away: yank, paste, nothing pasted.
 *
 * The bindings are the catalog's own rows for these chords
 * (`outl-shortcuts/src/defaults.rs`), in the shape
 * `list_shortcut_bindings` returns them.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Binding, Chord } from "./api";

vi.mock("./api", async (importOriginal) => {
  const real = await importOriginal<typeof import("./api")>();
  const ch = (value: string, mods = 0): Chord => ({
    mods,
    key: { kind: "Char", value },
  });
  const row = (chord: Chord[], mode: string, kind: string) =>
    ({ chord, mode, action: { kind }, description: "" }) as unknown as Binding;
  const bindings: Binding[] = [
    row([ch("y"), ch("y")], "normal", "YankCurrentBlock"),
    row([ch("y", real.MOD_SHIFT)], "normal", "YankCurrentBlock"),
    row([ch("y"), ch("r")], "normal", "CopyBlockRef"),
    row([ch("p")], "normal", "PasteClipboard"),
    row([ch("j")], "normal", "SelectionDown"),
    row([ch("y")], "visual", "YankRange"),
    row([ch("c", real.MOD_META)], "visual", "YankRange"),
  ];
  return {
    ...real,
    listShortcutBindings: vi.fn(async () => bindings),
    listActionSupport: vi.fn(async () => []),
    pluginKeybindings: vi.fn(async () => []),
  };
});

vi.mock("@outl/shared/api/commands", () => ({ pluginRun: vi.fn() }));

import { installShortcuts } from "./shortcuts";
import { setAppState } from "./store";

const fired: string[] = [];
let uninstall: () => void;

/** Press a key; returns whether the page was allowed to handle it. */
function press(key: string, init: KeyboardEventInit = {}): boolean {
  const e = new KeyboardEvent("keydown", {
    key,
    bubbles: true,
    cancelable: true,
    ...init,
  });
  document.body.dispatchEvent(e);
  return !e.defaultPrevented;
}

beforeEach(async () => {
  fired.length = 0;
  vi.spyOn(console, "info").mockImplementation(() => {});
  const record = (kind: string) => () => void fired.push(kind);
  uninstall = await installShortcuts({
    // What `exitVisual` does to the mode, which is what makes the next
    // `y` a Normal-mode key.
    YankRange: () => {
      fired.push("YankRange");
      setAppState("mode", "vim-normal");
    },
    YankCurrentBlock: record("YankCurrentBlock"),
    CopyBlockRef: record("CopyBlockRef"),
    PasteClipboard: record("PasteClipboard"),
    SelectionDown: record("SelectionDown"),
  });
  setAppState({
    mode: "vim-visual",
    pickerOpen: false,
    settingsOpen: false,
    helpOpen: false,
  });
});

afterEach(() => {
  uninstall();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("Visual yy", () => {
  it("y, y, p yanks the range and pastes it", () => {
    press("y");
    expect(press("y")).toBe(false);
    press("p");
    expect(fired).toEqual(["YankRange", "PasteClipboard"]);
  });

  it("a held y yanks once, and the p after it still pastes", () => {
    press("y");
    expect(press("y", { repeat: true })).toBe(false);
    expect(press("y", { repeat: true })).toBe(false);
    press("p");
    expect(fired).toEqual(["YankRange", "PasteClipboard"]);
  });

  it("swallows exactly one y: the next two are Normal's yy", () => {
    press("y");
    press("y");
    press("y");
    press("y");
    expect(fired).toEqual(["YankRange", "YankCurrentBlock"]);
  });

  it("does not swallow a y typed after the chord window", () => {
    vi.useFakeTimers();
    press("y");
    vi.advanceTimersByTime(1100);
    press("y");
    press("y");
    expect(fired).toEqual(["YankRange", "YankCurrentBlock"]);
  });

  it("any other key ends it, and nothing is replayed", () => {
    press("y");
    press("j");
    press("y");
    press("y");
    expect(fired).toEqual(["YankRange", "SelectionDown", "YankCurrentBlock"]);
  });

  it("a Cmd+C yank leaves the next y alone", () => {
    press("c", { metaKey: true });
    press("y");
    press("y");
    expect(fired).toEqual(["YankRange", "YankCurrentBlock"]);
  });

  it("a y typed into a block right after is a letter, not the echo", () => {
    press("y");
    const field = document.createElement("textarea");
    document.body.appendChild(field);
    field.focus();
    expect(press("y")).toBe(true);
    field.remove();
    expect(fired).toEqual(["YankRange"]);
  });

  it("Normal's y r still copies a ref", () => {
    setAppState("mode", "vim-normal");
    press("y");
    press("r");
    expect(fired).toEqual(["CopyBlockRef"]);
  });
});
