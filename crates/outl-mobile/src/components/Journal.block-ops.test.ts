import { beforeEach, describe, expect, it, vi } from "vitest";

import { createBlockOps, type JournalBlockDeps } from "./Journal.block-ops";
import type { BlockNode, PageView } from "@outl/shared/api/types";

vi.mock("../lib/haptics", () => ({ haptic: vi.fn() }));

const commands = vi.hoisted(() => ({
  copyBlockMarkdown: vi.fn(),
  copyBlockRef: vi.fn(),
  cutBlock: vi.fn(),
  deleteBlock: vi.fn(),
  editBlock: vi.fn(),
  indentBlock: vi.fn(),
  moveBlockDown: vi.fn(),
  moveBlockUp: vi.fn(),
  outdentBlock: vi.fn(),
  pasteBlockAfter: vi.fn(),
  toggleTodo: vi.fn(),
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

/** A deps object backed by plain mutable state, so a test can assert on
 *  what the ops wrote without a Solid runtime. */
function harness(outline: BlockNode[] = [block("blk-1")]) {
  const state = {
    view: pageView(outline),
    editingId: null as string | null,
    draft: "",
    clipboard: null as string | null,
    pendingDelete: null as { id: string; descendants: number } | null,
    applied: [] as PageView[],
    setViewCalls: [] as PageView[],
  };
  const deps: JournalBlockDeps = {
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
    setView: (v) => state.setViewCalls.push(v),
    editingId: () => state.editingId,
    setEditingId: (id) => {
      state.editingId = id;
    },
    draft: () => state.draft,
    setDraft: (t) => {
      state.draft = t;
    },
    blockClipboard: () => state.clipboard,
    setBlockClipboard: (md) => {
      state.clipboard = md;
    },
    setPendingDelete: (p) => {
      state.pendingDelete = p;
    },
  };
  return { state, ops: createBlockOps(deps) };
}

beforeEach(() => {
  for (const fn of Object.values(commands)) fn.mockReset();
});

describe("delete", () => {
  it("deletes a leaf immediately, without prompting", async () => {
    const { state, ops } = harness([block("blk-1")]);
    commands.deleteBlock.mockResolvedValue(pageView([]));

    ops.requestDelete("blk-1");
    await vi.waitFor(() => expect(commands.deleteBlock).toHaveBeenCalled());

    expect(state.pendingDelete).toBeNull();
  });

  it("prompts instead of deleting when the block has descendants", () => {
    const { state, ops } = harness([block("blk-1", [block("blk-2")])]);

    ops.requestDelete("blk-1");

    expect(commands.deleteBlock).not.toHaveBeenCalled();
    expect(state.pendingDelete).toEqual({ id: "blk-1", descendants: 1 });
  });

  it("leaves edit mode when the deleted block is the one being edited", async () => {
    const { state, ops } = harness();
    state.editingId = "blk-1";
    commands.deleteBlock.mockResolvedValue(pageView([]));

    await ops.performDelete("blk-1");

    expect(state.editingId).toBeNull();
  });
});

describe("toggleTodo", () => {
  it("commits the in-flight draft first when the block is being edited", async () => {
    const { state, ops } = harness();
    state.editingId = "blk-1";
    state.draft = "typed but not committed";
    commands.editBlock.mockResolvedValue(pageView([block("blk-1")]));
    commands.toggleTodo.mockResolvedValue(pageView([block("blk-1")]));

    await ops.toggleTodo("blk-1");

    expect(commands.editBlock).toHaveBeenCalledWith(
      "page-1",
      "blk-1",
      "typed but not committed",
    );
    // The commit must land before the toggle, or the cycle runs against
    // the stale backend text and the user's keystrokes are lost.
    expect(commands.editBlock.mock.invocationCallOrder[0]).toBeLessThan(
      commands.toggleTodo.mock.invocationCallOrder[0],
    );
  });

  it("does not commit anything when the block is not being edited", async () => {
    const { ops } = harness();
    commands.toggleTodo.mockResolvedValue(pageView([block("blk-1")]));

    await ops.toggleTodo("blk-1");

    expect(commands.editBlock).not.toHaveBeenCalled();
  });
});

describe("clipboard", () => {
  it("arms the clipboard on copy without mutating the workspace", async () => {
    const { state, ops } = harness();
    commands.copyBlockMarkdown.mockResolvedValue("- copied");

    await ops.copyBlock("blk-1");

    expect(state.clipboard).toBe("- copied");
    expect(state.applied).toHaveLength(0);
  });

  it("refuses to paste when nothing is armed", async () => {
    const { ops } = harness();

    await ops.pasteBlock("blk-1");

    expect(commands.pasteBlockAfter).not.toHaveBeenCalled();
  });

  it("keeps the clipboard armed after a paste, so it can be pasted again", async () => {
    const { state, ops } = harness();
    state.clipboard = "- copied";
    commands.pasteBlockAfter.mockResolvedValue(pageView([block("blk-1")]));

    await ops.pasteBlock("blk-1");

    expect(state.clipboard).toBe("- copied");
  });

  it("arms the clipboard from a cut and leaves edit mode", async () => {
    const { state, ops } = harness();
    state.editingId = "blk-1";
    commands.cutBlock.mockResolvedValue({
      markdown: "- cut",
      view: pageView([]),
    });

    await ops.cutBlock("blk-1");

    expect(state.clipboard).toBe("- cut");
    expect(state.editingId).toBeNull();
  });

  it("does not arm the clipboard when the cut failed", async () => {
    const { state, ops } = harness();
    commands.cutBlock.mockRejectedValue(new Error("nope"));

    await ops.cutBlock("blk-1");

    expect(state.clipboard).toBeNull();
  });
});

describe("structural ops", () => {
  it.each([
    ["indent", "indentBlock"],
    ["outdent", "outdentBlock"],
    ["moveUp", "moveBlockUp"],
    ["moveDown", "moveBlockDown"],
  ] as const)("%s fires %s and folds the view back in", async (op, cmd) => {
    const { state, ops } = harness();
    const next = pageView([block("blk-1")]);
    commands[cmd].mockResolvedValue(next);

    await ops[op]("blk-1");

    expect(commands[cmd]).toHaveBeenCalledWith("page-1", "blk-1");
    expect(state.applied).toEqual([next]);
  });

  it("applies nothing when the command failed", async () => {
    const { state, ops } = harness();
    commands.indentBlock.mockRejectedValue(new Error("nope"));

    await ops.indent("blk-1");

    expect(state.applied).toHaveLength(0);
  });
});

describe("detached handlers", () => {
  /** `Journal` passes every one of these to `<BlockRow />` as a prop, so
   *  they are always called off the object. A `this.` inside the factory
   *  would pass every test above and break at the first prop pass. */
  it("survives being destructured off the returned object", async () => {
    const { state, ops } = harness([block("blk-1")]);
    const { requestDelete } = ops;
    commands.deleteBlock.mockResolvedValue(pageView([]));

    expect(() => requestDelete("blk-1")).not.toThrow();
    await vi.waitFor(() => expect(commands.deleteBlock).toHaveBeenCalled());
    expect(state.pendingDelete).toBeNull();
  });
});
