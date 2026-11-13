/**
 * Regression: `Shift+Tab` inside a block outdented **two** levels, not
 * one (and `Tab` indented twice whenever the second indent happened to
 * be legal).
 *
 * `keydown` is one of Solid's delegated events, so `<BlockEditor />`'s
 * `handleKeydown` runs from a listener on `document` — one node *below*
 * the `window` listener `installShortcuts` adds. The Tab branch commits
 * first, and committing an unchanged draft writes `editingBlockId =
 * null` synchronously, which unmounts the textarea. By the time the
 * event reaches `window`, nothing is focused, so `detectMode()` answers
 * `normal` and the catalog's Normal rows (`Tab` → `IndentBlock`,
 * `Shift+Tab` → `OutdentBlock`) fire a second move on the same
 * keystroke.
 *
 * Outdent is where the user sees it: a second outdent always succeeds
 * until the block hits the page root, while a second indent usually
 * returns `NoPreviousSibling` and is swallowed.
 *
 * These drive the real dispatcher against a real `<BlockRow />`, because
 * the defect is in the interaction between the two — each half is
 * correct on its own.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Binding } from "../lib/api";

vi.mock("../lib/api", async (importOriginal) => {
  const real = await importOriginal<typeof import("../lib/api")>();
  const tab = (mods: number) => ({ mods, key: { kind: "Tab" } });
  const bindings: Binding[] = [
    {
      chord: [tab(0)],
      mode: "normal",
      action: { kind: "IndentBlock" },
      description: "",
    },
    {
      chord: [tab(real.MOD_SHIFT)],
      mode: "normal",
      action: { kind: "OutdentBlock" },
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

function makeCb(): BlockCallbacks {
  return {
    onStartEdit: vi.fn(),
    onCommit: vi.fn().mockResolvedValue(undefined),
    onEnter: vi.fn().mockResolvedValue(undefined),
    onCreateBefore: vi.fn().mockResolvedValue(undefined),
    onIndent: vi.fn().mockResolvedValue(undefined),
    onOutdent: vi.fn().mockResolvedValue(undefined),
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

/**
 * Mount one row whose edit state is driven by the store, the way
 * `<OutlineView />` wires it — the whole point is that `commit()` can
 * unmount the textarea mid-keystroke.
 */
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

function pressTab(target: HTMLElement, shift: boolean) {
  target.dispatchEvent(
    new KeyboardEvent("keydown", {
      key: "Tab",
      shiftKey: shift,
      bubbles: true,
      cancelable: true,
    }),
  );
}

beforeEach(async () => {
  fired.length = 0;
  vi.spyOn(console, "info").mockImplementation(() => {});
  uninstall = await installShortcuts({
    IndentBlock: () => void fired.push("IndentBlock"),
    OutdentBlock: () => void fired.push("OutdentBlock"),
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

describe("Tab / Shift+Tab inside a block move it exactly once", () => {
  it("Shift+Tab outdents one level, not two", async () => {
    const cb = makeCb();
    const ta = mountEditing(makeBlock("blk-1"), cb);
    ta.focus();

    pressTab(ta, true);
    await Promise.resolve();
    await Promise.resolve();

    expect(cb.onOutdent).toHaveBeenCalledTimes(1);
    expect(fired).toEqual([]);
  });

  it("Tab indents one level, not two", async () => {
    const cb = makeCb();
    const ta = mountEditing(makeBlock("blk-2"), cb);
    ta.focus();

    pressTab(ta, false);
    await Promise.resolve();
    await Promise.resolve();

    expect(cb.onIndent).toHaveBeenCalledTimes(1);
    expect(fired).toEqual([]);
  });
});

describe("what the fix must not break", () => {
  it("Shift+Tab in view mode still reaches the Normal-mode handler", () => {
    pressTab(document.body, true);
    expect(fired).toEqual(["OutdentBlock"]);
  });

  it("Tab in view mode still reaches the Normal-mode handler", () => {
    pressTab(document.body, false);
    expect(fired).toEqual(["IndentBlock"]);
  });
});
