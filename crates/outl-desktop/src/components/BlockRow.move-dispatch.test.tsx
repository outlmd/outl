/**
 * The move chords inside a block's textarea (catalog Insert rows).
 *
 * Same shape as `BlockRow.tab-dispatch.test.tsx`: the defect space is
 * the interaction between `<BlockEditor />`'s delegated `document`
 * listener and the `window` dispatcher, so both are real. `moveOnce` is
 * module state, hence every test settles what it leaves pending.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Binding } from "../lib/api";

vi.mock("../lib/api", async (importOriginal) => {
  const real = await importOriginal<typeof import("../lib/api")>();
  const arrow = (mods: number, kind: string) => ({ mods, key: { kind } });
  const chords = (kind: "Up" | "Down") => [
    arrow(real.MOD_META | real.MOD_SHIFT, kind),
    arrow(real.MOD_CTRL | real.MOD_SHIFT, kind),
    arrow(real.MOD_ALT, kind),
  ];
  const rows = (
    mode: string,
    up: string,
    down: string,
    letters: boolean,
  ): Binding[] => {
    const out = [
      ...chords("Up").map((c) => ({ chord: [c], mode, action: { kind: up } })),
      ...chords("Down").map((c) => ({
        chord: [c],
        mode,
        action: { kind: down },
      })),
    ];
    if (letters) {
      const k = (value: string) => ({
        mods: real.MOD_SHIFT,
        key: { kind: "Char", value },
      });
      out.push(
        { chord: [k("k")], mode, action: { kind: up } },
        { chord: [k("j")], mode, action: { kind: down } },
      );
    }
    return out.map((b) => ({ ...b, description: "" })) as Binding[];
  };
  const bindings: Binding[] = [
    ...rows("insert", "MoveBlockUp", "MoveBlockDown", false),
    ...rows("normal", "MoveBlockUp", "MoveBlockDown", true),
    ...rows("visual", "MoveVisualRangeUp", "MoveVisualRangeDown", true),
  ];
  return {
    ...real,
    listShortcutBindings: vi.fn(async () => bindings),
    listActionSupport: vi.fn(async () => []),
    pluginKeybindings: vi.fn(async () => []),
  };
});

vi.mock("@outl/shared/api/commands", () => ({
  openRef: vi.fn(),
  pluginRun: vi.fn(),
  pluginList: vi.fn().mockResolvedValue([]),
  searchEmojis: vi.fn().mockResolvedValue([]),
  searchPages: vi.fn().mockResolvedValue([]),
  searchPersons: vi.fn().mockResolvedValue([]),
  listTemplates: vi.fn().mockResolvedValue([]),
}));
vi.mock("@outl/shared/plugins/transformer-registry", () => ({
  runTransform: vi.fn(),
  transformerFor: vi.fn(() => null),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ readText: vi.fn() }));

import { render } from "solid-js/web";

import type { BlockNode } from "@outl/shared/api/types";
import { BlockRow, type BlockCallbacks } from "./BlockRow";
import { setEditorOwnsHold } from "../lib/move-gate";
import { installShortcuts } from "../lib/shortcuts";
import { appState, setAppState } from "../lib/store";

const fired: string[] = [];
let uninstall: () => void;
let dispose: (() => void) | undefined;
/** Promises a test left pending; settled in `afterEach`. */
const settle: Array<() => void> = [];

function makeBlock(id: string, text = "hello"): BlockNode {
  return {
    id,
    text,
    todo: null,
    tokens: [],
    collapsed: false,
    properties: [],
    children: [],
  };
}

function makeCb(): BlockCallbacks {
  return {
    onStartEdit: vi.fn(),
    onCommit: vi.fn().mockResolvedValue(undefined),
    onEnter: vi.fn().mockResolvedValue(undefined),
    onCreateBefore: vi.fn().mockResolvedValue(undefined),
    resolveCaretExit: vi.fn().mockReturnValue(null),
    openCaretLanding: vi.fn(),
    onIndent: vi.fn().mockResolvedValue(undefined),
    onOutdent: vi.fn().mockResolvedValue(undefined),
    onMoveUp: vi.fn().mockResolvedValue(undefined),
    onMoveDown: vi.fn().mockResolvedValue(undefined),
    onDeleteEmpty: vi.fn().mockResolvedValue(undefined),
    onToggleTodo: vi.fn().mockResolvedValue(undefined),
    onToggleCollapsed: vi.fn().mockResolvedValue(undefined),
    onPasteMarkdown: vi.fn().mockResolvedValue(undefined),
    onPastePlain: vi.fn().mockResolvedValue(undefined),
    onRunCodeBlock: vi.fn().mockResolvedValue(undefined),
    onRunPluginCommand: vi.fn().mockResolvedValue(undefined),
    onRefClick: vi.fn(),
    onTagClick: vi.fn(),
    onOpenPage: vi.fn(),
  };
}

function mountEditing(block: BlockNode, cb: BlockCallbacks): HTMLTextAreaElement {
  const host = document.createElement("div");
  document.body.appendChild(host);
  setAppState("editingBlockId", block.id);
  dispose = render(
    () => (
      <BlockRow
        block={block}
        depth={0}
        editingId={appState.editingBlockId}
        visualSet={null}
        cb={cb}
      />
    ),
    host,
  );
  const ta = host.querySelector("textarea");
  if (!ta) throw new Error("textarea did not render in edit mode");
  return ta;
}

/** A promise the test resolves itself; settled in `afterEach` if not. */
function deferred(): { promise: Promise<void>; resolve: () => void } {
  let resolve!: () => void;
  const promise = new Promise<void>((r) => (resolve = r));
  settle.push(resolve);
  return { promise, resolve };
}

const CHORDS: Array<[string, KeyboardEventInit, "up" | "down"]> = [
  ["Alt+Up", { key: "ArrowUp", altKey: true }, "up"],
  ["Meta+Shift+Up", { key: "ArrowUp", metaKey: true, shiftKey: true }, "up"],
  ["Ctrl+Shift+Down", { key: "ArrowDown", ctrlKey: true, shiftKey: true }, "down"],
];

/** Returns whether the page was allowed to handle the key itself. */
function press(target: HTMLElement, init: KeyboardEventInit): boolean {
  const e = new KeyboardEvent("keydown", {
    bubbles: true,
    cancelable: true,
    ...init,
  });
  target.dispatchEvent(e);
  return !e.defaultPrevented;
}

const tick = async () => {
  for (let i = 0; i < 4; i++) await Promise.resolve();
};

beforeEach(async () => {
  fired.length = 0;
  vi.spyOn(console, "info").mockImplementation(() => {});
  uninstall = await installShortcuts({
    MoveBlockUp: () => void fired.push("MoveBlockUp"),
    MoveBlockDown: () => void fired.push("MoveBlockDown"),
    MoveVisualRangeUp: () => void fired.push("MoveVisualRangeUp"),
    MoveVisualRangeDown: () => void fired.push("MoveVisualRangeDown"),
  });
  setAppState({
    mode: "normal",
    pickerOpen: false,
    settingsOpen: false,
    helpOpen: false,
    editingBlockId: null,
    caretIntent: null,
  });
  setEditorOwnsHold(false);
});

afterEach(async () => {
  while (settle.length) settle.pop()?.();
  await tick();
  uninstall();
  dispose?.();
  dispose = undefined;
  setAppState({ editingBlockId: null, caretIntent: null, mode: "normal" });
  setEditorOwnsHold(false);
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

describe("a move chord inside a block", () => {
  for (const [name, init, dir] of CHORDS) {
    it(`${name} moves the edited block exactly once and the window dispatcher never fires`, async () => {
      const cb = makeCb();
      const ta = mountEditing(makeBlock("blk-1"), cb);
      ta.focus();

      expect(press(ta, init)).toBe(false);
      await tick();

      const [hit, miss] =
        dir === "up" ? [cb.onMoveUp, cb.onMoveDown] : [cb.onMoveDown, cb.onMoveUp];
      expect(hit).toHaveBeenCalledTimes(1);
      expect(hit).toHaveBeenCalledWith("blk-1");
      expect(miss).not.toHaveBeenCalled();
      expect(fired).toEqual([]);
    });
  }

  it("commits a changed draft before the move and keeps the caret in caretIntent", async () => {
    const cb = makeCb();
    const order: string[] = [];
    vi.mocked(cb.onCommit).mockImplementation(async () => void order.push("commit"));
    vi.mocked(cb.onMoveUp).mockImplementation(async () => void order.push("move"));
    const ta = mountEditing(makeBlock("blk-2"), cb);
    await tick(); // the mount's own caretIntent pass
    ta.focus();
    ta.value = "hello world";
    ta.dispatchEvent(new Event("input", { bubbles: true }));
    ta.setSelectionRange(5, 5);

    press(ta, { key: "ArrowUp", altKey: true });
    await tick();

    expect(cb.onCommit).toHaveBeenCalledWith("blk-2", "hello world");
    expect(order).toEqual(["commit", "move"]);
    expect(appState.caretIntent).toBe(5);
  });

  it("drops a repeat while the commit round-trip is pending", async () => {
    const cb = makeCb();
    const commit = deferred();
    vi.mocked(cb.onCommit).mockReturnValue(commit.promise);
    const ta = mountEditing(makeBlock("blk-3"), cb);
    ta.focus();
    ta.value = "changed";
    ta.dispatchEvent(new Event("input", { bubbles: true }));

    press(ta, { key: "ArrowUp", altKey: true });
    press(ta, { key: "ArrowUp", altKey: true, repeat: true });
    press(ta, { key: "ArrowUp", altKey: true, repeat: true });
    await tick();
    expect(cb.onMoveUp).not.toHaveBeenCalled();

    commit.resolve();
    await tick();
    expect(cb.onMoveUp).toHaveBeenCalledTimes(1);
  });

  it("drops a repeat that lands on window after the textarea unmounted", async () => {
    const cb = makeCb();
    const ta = mountEditing(makeBlock("blk-4"), cb);
    ta.focus();
    press(ta, { key: "ArrowUp", altKey: true });
    await tick();
    // The editor finished (a backlink row's does) and nothing is focused.
    setAppState("editingBlockId", null);
    ta.blur();

    const allowed = press(document.body, {
      key: "ArrowUp",
      altKey: true,
      repeat: true,
    });

    expect(allowed).toBe(false);
    expect(fired).toEqual([]);
  });

  it("a fresh press on window moves again after the hold ended", () => {
    setEditorOwnsHold(true);
    press(document.body, { key: "ArrowUp", altKey: true });
    expect(fired).toEqual(["MoveBlockUp"]);
  });

  it("moves nothing from a property, title or search input and does not preventDefault", async () => {
    const cb = makeCb();
    mountEditing(makeBlock("blk-5"), cb);
    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();

    for (const [, init] of CHORDS) expect(press(input, init)).toBe(true);
    await tick();

    expect(fired).toEqual([]);
    expect(cb.onMoveUp).not.toHaveBeenCalled();
    expect(cb.onMoveDown).not.toHaveBeenCalled();
  });

  it("a dropped chord leaves caretIntent null", async () => {
    const cb = makeCb();
    const pending = deferred();
    vi.mocked(cb.onMoveUp).mockReturnValue(pending.promise);
    const ta = mountEditing(makeBlock("blk-6"), cb);
    ta.focus();
    press(ta, { key: "ArrowUp", altKey: true });
    await tick();
    setAppState("caretIntent", null); // the remount consumed it

    press(ta, { key: "ArrowUp", altKey: true, repeat: true });
    await tick();

    expect(appState.caretIntent).toBeNull();
  });

  it("leaves Shift+K as text inside a textarea", async () => {
    const cb = makeCb();
    const ta = mountEditing(makeBlock("blk-7"), cb);
    ta.focus();

    expect(press(ta, { key: "K", shiftKey: true })).toBe(true);
    await tick();

    expect(fired).toEqual([]);
    expect(cb.onMoveUp).not.toHaveBeenCalled();
  });
});

describe("the same chords outside a textarea", () => {
  it("Normal: Shift+J, Shift+K and Alt+arrows reach MoveBlock*", () => {
    press(document.body, { key: "K", shiftKey: true });
    press(document.body, { key: "J", shiftKey: true });
    press(document.body, { key: "ArrowUp", altKey: true });
    press(document.body, { key: "ArrowDown", altKey: true });
    expect(fired).toEqual([
      "MoveBlockUp",
      "MoveBlockDown",
      "MoveBlockUp",
      "MoveBlockDown",
    ]);
  });

  it("Visual: the same chords reach MoveVisualRange*", () => {
    setAppState("mode", "vim-visual");
    press(document.body, { key: "K", shiftKey: true });
    press(document.body, { key: "ArrowDown", metaKey: true, shiftKey: true });
    expect(fired).toEqual(["MoveVisualRangeUp", "MoveVisualRangeDown"]);
  });
});
