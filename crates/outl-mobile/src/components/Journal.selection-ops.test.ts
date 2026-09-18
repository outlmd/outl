import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  createSelectionOps,
  type JournalSelectionDeps,
} from "./Journal.selection-ops";
import type { BlockSelection } from "../lib/block-selection";
import type { BlockNode, PageView } from "@outl/shared/api/types";

vi.mock("../lib/haptics", () => ({ haptic: vi.fn() }));

const commands = vi.hoisted(() => ({
  copyMarkdown: vi.fn(),
  deleteBlock: vi.fn(),
  indentBlock: vi.fn(),
  moveBlockDown: vi.fn(),
  moveBlockUp: vi.fn(),
  outdentBlock: vi.fn(),
}));
vi.mock("@outl/shared/api/commands", () => commands);

function block(id: string, children: BlockNode[] = []): BlockNode {
  return {
    id,
    text: id,
    tokens: [],
    children,
    collapsed: false,
    todo: null,
    properties: [],
  } as unknown as BlockNode;
}

function pageView(outline: BlockNode[]): PageView {
  return {
    page: { id: "page-1", slug: "2026-09-17", title: "", kind: "journal" },
    outline,
  } as unknown as PageView;
}

/** Four flat siblings — enough for a range with an interior. */
const FLAT = [block("a"), block("b"), block("c"), block("d")];

function harness(outline: BlockNode[] = FLAT) {
  const state = {
    view: pageView(outline),
    editingId: null as string | null,
    selection: null as BlockSelection | null,
    lastSelection: null as BlockSelection | null,
    pendingRangeDelete: null as string[] | null,
    applied: [] as PageView[],
    commits: 0,
  };
  const deps: JournalSelectionDeps = {
    pageId: () => state.view.page.id,
    view: () => state.view,
    withError: async (fn) => {
      try {
        return await fn();
      } catch {
        return undefined;
      }
    },
    applyView: (v) => state.applied.push(v),
    setView: () => {},
    editingId: () => state.editingId,
    setEditingId: (id) => {
      state.editingId = id;
    },
    draft: () => "",
    setDraft: () => {},
    blockClipboard: () => null,
    setBlockClipboard: () => {},
    setPendingDelete: () => {},
    selection: () => state.selection,
    setSelection: (sel) => {
      state.selection = sel;
    },
    lastSelection: () => state.lastSelection,
    setLastSelection: (sel) => {
      state.lastSelection = sel;
    },
    setPendingRangeDelete: (ids) => {
      state.pendingRangeDelete = ids;
    },
    commitEdit: async () => {
      state.commits += 1;
    },
  };
  return { state, ops: createSelectionOps(deps) };
}

beforeEach(() => {
  for (const fn of Object.values(commands)) fn.mockReset();
});

describe("entering and leaving selection", () => {
  it("commits an in-flight edit before selection mode takes the row over", async () => {
    const { state, ops } = harness();
    state.editingId = "b";

    await ops.selectBlocks("b");

    expect(state.commits).toBe(1);
    expect(state.selection).not.toBeNull();
  });

  it("does not commit when nothing is being edited", async () => {
    const { state, ops } = harness();

    await ops.selectBlocks("b");

    expect(state.commits).toBe(0);
  });

  it("captures the range as lastSelection on every exit (vim gv)", () => {
    const { state, ops } = harness();
    state.selection = { anchorId: "b", cursorId: "c" };

    ops.exit();

    expect(state.selection).toBeNull();
    expect(state.lastSelection).toEqual({ anchorId: "b", cursorId: "c" });
  });

  it("refuses to reselect a range whose endpoint has left the outline", () => {
    const { state, ops } = harness();
    state.lastSelection = { anchorId: "b", cursorId: "gone" };

    ops.reselectLast();

    expect(state.selection).toBeNull();
  });
});

describe("currentRangeIds", () => {
  it("returns the covered ids in visible order, regardless of drag direction", () => {
    const { state, ops } = harness();

    state.selection = { anchorId: "b", cursorId: "d" };
    expect(ops.currentRangeIds()).toEqual(["b", "c", "d"]);

    // Dragging upward must produce the same range, not a reversed one.
    state.selection = { anchorId: "d", cursorId: "b" };
    expect(ops.currentRangeIds()).toEqual(["b", "c", "d"]);
  });

  it("is null with no selection", () => {
    const { ops } = harness();
    expect(ops.currentRangeIds()).toBeNull();
  });
});

describe("range ops walk order", () => {
  it("indents top-down", async () => {
    const { state, ops } = harness();
    state.selection = { anchorId: "a", cursorId: "c" };
    commands.indentBlock.mockResolvedValue(pageView(FLAT));

    await ops.indentRange();

    expect(commands.indentBlock.mock.calls.map((c) => c[1])).toEqual([
      "a",
      "b",
      "c",
    ]);
  });

  /**
   * The one genuinely subtle rule in this module, and the reason it is
   * worth a test rather than a comment: a move-down has to clear the
   * block *below* the range before its neighbours slide into place. An
   * ascending walk drags each block over its own not-yet-moved
   * neighbour and scrambles the range.
   */
  it("moves down bottom-up", async () => {
    const { state, ops } = harness();
    state.selection = { anchorId: "a", cursorId: "c" };
    commands.moveBlockDown.mockResolvedValue(pageView(FLAT));

    await ops.moveRangeDown();

    expect(commands.moveBlockDown.mock.calls.map((c) => c[1])).toEqual([
      "c",
      "b",
      "a",
    ]);
  });

  it("moves up top-down", async () => {
    const { state, ops } = harness();
    state.selection = { anchorId: "b", cursorId: "d" };
    commands.moveBlockUp.mockResolvedValue(pageView(FLAT));

    await ops.moveRangeUp();

    expect(commands.moveBlockUp.mock.calls.map((c) => c[1])).toEqual([
      "b",
      "c",
      "d",
    ]);
  });

  it("keeps the range selected afterward so the op can be repeated", async () => {
    const { state, ops } = harness();
    state.selection = { anchorId: "a", cursorId: "b" };
    commands.outdentBlock.mockResolvedValue(pageView(FLAT));

    await ops.outdentRange();

    expect(state.selection).toEqual({ anchorId: "a", cursorId: "b" });
  });

  it("applies only the last view, not one per block", async () => {
    const { state, ops } = harness();
    state.selection = { anchorId: "a", cursorId: "c" };
    const last = pageView([block("a")]);
    commands.indentBlock
      .mockResolvedValueOnce(pageView(FLAT))
      .mockResolvedValueOnce(pageView(FLAT))
      .mockResolvedValueOnce(last);

    await ops.indentRange();

    expect(state.applied).toEqual([last]);
  });

  it("does nothing at all without a selection", async () => {
    const { ops } = harness();
    await ops.indentRange();
    expect(commands.indentBlock).not.toHaveBeenCalled();
  });
});

describe("range delete", () => {
  it("always confirms rather than deleting, however small the range", () => {
    const { state, ops } = harness();
    state.selection = { anchorId: "a", cursorId: "a" };

    ops.requestDeleteRange();

    expect(commands.deleteBlock).not.toHaveBeenCalled();
    expect(state.pendingRangeDelete).toEqual(["a"]);
  });

  /** Children before parents: deleting a parent first trashes the whole
   *  subtree, and the follow-up delete on a descendant then fails. */
  it("deletes bottom-up", async () => {
    const { ops } = harness();
    commands.deleteBlock.mockResolvedValue(pageView([]));

    await ops.performDeleteRange(["a", "b", "c"]);

    expect(commands.deleteBlock.mock.calls.map((c) => c[1])).toEqual([
      "c",
      "b",
      "a",
    ]);
  });

  it("does not let one failed id strand the rest of the range", async () => {
    const { ops } = harness();
    commands.deleteBlock
      .mockResolvedValueOnce(pageView([]))
      .mockRejectedValueOnce(new Error("already in trash"))
      .mockResolvedValueOnce(pageView([]));

    await ops.performDeleteRange(["a", "b", "c"]);

    expect(commands.deleteBlock).toHaveBeenCalledTimes(3);
  });

  it("leaves edit mode when the range covers the block being edited", async () => {
    const { state, ops } = harness();
    state.editingId = "b";
    commands.deleteBlock.mockResolvedValue(pageView([]));

    await ops.performDeleteRange(["a", "b"]);

    expect(state.editingId).toBeNull();
  });

  it("exits selection when the delete lands", async () => {
    const { state, ops } = harness();
    state.selection = { anchorId: "a", cursorId: "b" };
    commands.deleteBlock.mockResolvedValue(pageView([]));

    await ops.performDeleteRange(["a", "b"]);

    expect(state.selection).toBeNull();
  });
});

describe("yank", () => {
  it("exits selection after copying, matching vim's y", async () => {
    const { state, ops } = harness();
    state.selection = { anchorId: "a", cursorId: "b" };
    commands.copyMarkdown.mockResolvedValue("- a\n- b");
    Object.defineProperty(globalThis, "navigator", {
      value: { clipboard: { writeText: vi.fn() } },
      configurable: true,
    });

    await ops.yankRange();

    expect(commands.copyMarkdown).toHaveBeenCalledWith(["a", "b"]);
    expect(state.selection).toBeNull();
  });

  it("still exits when the clipboard write is refused by the webview", async () => {
    const { state, ops } = harness();
    state.selection = { anchorId: "a", cursorId: "b" };
    commands.copyMarkdown.mockResolvedValue("- a\n- b");
    Object.defineProperty(globalThis, "navigator", {
      value: {
        clipboard: {
          writeText: vi.fn().mockRejectedValue(new Error("not allowed")),
        },
      },
      configurable: true,
    });

    await ops.yankRange();

    expect(state.selection).toBeNull();
  });
});
