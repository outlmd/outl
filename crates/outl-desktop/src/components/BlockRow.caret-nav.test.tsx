/**
 * Cross-block caret nav, at the only layer where its guard can fail.
 *
 * `↑` / `↓` are bound in the catalog's **Normal** rows
 * (`SelectionUp` / `SelectionDown`), and `keydown` is one of Solid's
 * delegated events — so `<BlockEditor />`'s handler runs from a
 * listener on `document`, one node *below* the `window` listener
 * `installShortcuts` adds. Its arrow branch commits first, and
 * committing an unchanged draft writes `editingBlockId = null`
 * synchronously, unmounting the textarea. By the time the event reaches
 * `window` nothing is focused, `detectMode()` answers `normal`, and the
 * catalog fires a *second* move on the same keystroke: the caret lands
 * two blocks away, and the cursor and the editor disagree about where
 * it went.
 *
 * `stopImmediatePropagation` is what stops that, and nothing else does.
 * These drive the real dispatcher against a real `<BlockRow />` because
 * the defect lives in the interaction between the two — each half is
 * correct on its own. Same shape as `BlockRow.tab-dispatch.test.tsx`,
 * for the same reason.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Binding } from "../lib/api";

vi.mock("../lib/api", async (importOriginal) => {
  const real = await importOriginal<typeof import("../lib/api")>();
  const arrow = (kind: string) => ({ mods: 0, key: { kind } });
  const bindings: Binding[] = [
    {
      chord: [arrow("Up")],
      mode: "normal",
      action: { kind: "SelectionUp" },
      description: "",
    },
    {
      chord: [arrow("Down")],
      mode: "normal",
      action: { kind: "SelectionDown" },
      description: "",
    },
  ] as Binding[];
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
import { installShortcuts } from "../lib/shortcuts";
import { appState, setAppState } from "../lib/store";

/** Actions the window dispatcher resolved, in order. */
const fired: string[] = [];
let uninstall: () => void;
let dispose: (() => void) | undefined;

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

function makeCb(over: Partial<BlockCallbacks> = {}): BlockCallbacks {
  return {
    onStartEdit: vi.fn(),
    onCommit: vi.fn().mockResolvedValue(undefined),
    onEnter: vi.fn().mockResolvedValue(undefined),
    onCreateBefore: vi.fn().mockResolvedValue(undefined),
    resolveCaretExit: vi
      .fn()
      .mockReturnValue({ id: "blk-neighbour", caret: 3, text: "neighbour" }),
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
    ...over,
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

function press(target: HTMLElement, key: string, init: KeyboardEventInit = {}) {
  target.dispatchEvent(
    new KeyboardEvent("keydown", {
      key,
      bubbles: true,
      cancelable: true,
      ...init,
    }),
  );
}

/** Put the caret where the arrow is meant to leave the block. */
function caretAt(ta: HTMLTextAreaElement, offset: number) {
  ta.focus();
  ta.setSelectionRange(offset, offset);
}

beforeEach(async () => {
  fired.length = 0;
  vi.spyOn(console, "info").mockImplementation(() => {});
  uninstall = await installShortcuts({
    SelectionUp: () => void fired.push("SelectionUp"),
    SelectionDown: () => void fired.push("SelectionDown"),
  });
  setAppState({
    mode: "normal",
    pickerOpen: false,
    settingsOpen: false,
    helpOpen: false,
    editingBlockId: null,
    caretIntent: null,
  });
});

afterEach(() => {
  uninstall();
  dispose?.();
  dispose = undefined;
  setAppState({ editingBlockId: null, caretIntent: null });
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

describe("an arrow at the block's edge moves the caret exactly once", () => {
  it("↑ hands over to the neighbour without the catalog also firing", async () => {
    const cb = makeCb();
    const ta = mountEditing(makeBlock("blk-1"), cb);
    caretAt(ta, 0);

    press(ta, "ArrowUp");
    await Promise.resolve();
    await Promise.resolve();

    expect(cb.openCaretLanding).toHaveBeenCalledTimes(1);
    expect(cb.openCaretLanding).toHaveBeenCalledWith({
      id: "blk-neighbour",
      caret: 3,
      text: "neighbour",
    });
    expect(fired).toEqual([]);
  });

  it("↓ does the same downwards", async () => {
    const cb = makeCb();
    const ta = mountEditing(makeBlock("blk-2"), cb);
    caretAt(ta, "hello".length);

    press(ta, "ArrowDown");
    await Promise.resolve();
    await Promise.resolve();

    expect(cb.openCaretLanding).toHaveBeenCalledTimes(1);
    expect(fired).toEqual([]);
  });

  it("← at offset 0 steps out, aiming at the previous block's end", async () => {
    const cb = makeCb();
    const ta = mountEditing(makeBlock("blk-3"), cb);
    caretAt(ta, 0);

    press(ta, "ArrowLeft");
    await Promise.resolve();
    await Promise.resolve();

    expect(cb.resolveCaretExit).toHaveBeenCalledWith("blk-3", {
      to: "previous",
      landing: "end",
    });
    expect(cb.openCaretLanding).toHaveBeenCalledTimes(1);
  });

  it("→ at the end steps out, aiming at the next block's start", async () => {
    const cb = makeCb();
    const ta = mountEditing(makeBlock("blk-4"), cb);
    caretAt(ta, "hello".length);

    press(ta, "ArrowRight");
    await Promise.resolve();
    await Promise.resolve();

    expect(cb.resolveCaretExit).toHaveBeenCalledWith("blk-4", {
      to: "next",
      landing: "start",
    });
    expect(cb.openCaretLanding).toHaveBeenCalledTimes(1);
  });
});

describe("what the walk must not do", () => {
  it("keeps ← inside the block when the caret has room to its left", async () => {
    const cb = makeCb();
    const ta = mountEditing(makeBlock("blk-5"), cb);
    caretAt(ta, 2);

    press(ta, "ArrowLeft");
    await Promise.resolve();

    expect(cb.resolveCaretExit).not.toHaveBeenCalled();
    expect(cb.openCaretLanding).not.toHaveBeenCalled();
    expect(appState.editingBlockId).toBe("blk-5");
  });

  it("leaves the user in Insert at the outline's edge, where there is no neighbour", async () => {
    // `resolveCaretExit` answering null is the top / bottom of the walk.
    // The editor must not commit on it: committing writes
    // `editingBlockId = null`, so a `←` in the page's first block would
    // drop the user out of edit mode instead of doing nothing.
    const cb = makeCb({ resolveCaretExit: vi.fn().mockReturnValue(null) });
    const ta = mountEditing(makeBlock("blk-6"), cb);
    caretAt(ta, 0);

    press(ta, "ArrowLeft");
    await Promise.resolve();
    await Promise.resolve();

    expect(cb.onCommit).not.toHaveBeenCalled();
    expect(cb.openCaretLanding).not.toHaveBeenCalled();
    expect(appState.editingBlockId).toBe("blk-6");
  });

  it("leaves a modified arrow alone — Alt+↑ moves the block, it does not walk", async () => {
    const cb = makeCb();
    const ta = mountEditing(makeBlock("blk-7"), cb);
    caretAt(ta, 0);

    press(ta, "ArrowUp", { altKey: true });
    press(ta, "ArrowUp", { shiftKey: true });
    press(ta, "ArrowUp", { metaKey: true });
    await Promise.resolve();

    expect(cb.resolveCaretExit).not.toHaveBeenCalled();
  });

  it("swallows a held arrow's repeat while the step's commit is in flight", async () => {
    // A changed draft makes `commit()` a backend round-trip, and the
    // textarea stays mounted until it lands. A held arrow repeats inside
    // that window; each repeat must not commit the same draft again.
    let land!: () => void;
    const onCommit = vi.fn(
      () => new Promise<void>((resolve) => (land = resolve)),
    );
    const cb = makeCb({ onCommit });
    const ta = mountEditing(makeBlock("blk-8"), cb);
    ta.value = "hello!";
    ta.dispatchEvent(new Event("input", { bubbles: true }));
    caretAt(ta, 0);

    press(ta, "ArrowUp");
    await Promise.resolve();
    press(ta, "ArrowUp");
    press(ta, "ArrowUp");
    await Promise.resolve();

    expect(onCommit).toHaveBeenCalledTimes(1);
    expect(fired).toEqual([]);

    land();
    await vi.waitFor(() => expect(cb.openCaretLanding).toHaveBeenCalledTimes(1));
  });

  it("still lets ↑ reach the Normal-mode handler in view mode", () => {
    press(document.body, "ArrowUp");
    expect(fired).toEqual(["SelectionUp"]);
  });
});
