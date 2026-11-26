/**
 * `ToggleCollapsed` with the cursor on a backlink row folds the cited
 * block **on its own page**, through the same `backlink_row_op` the
 * chevron uses, and never through the outline's `setBlockCollapsed`,
 * which would address the page on screen.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Backlink, BlockNode, PageView } from "@outl/shared/api/types";

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => ({ close: vi.fn() })),
}));

vi.mock("@outl/shared/api/commands", () => ({
  backlinkRowOp: vi.fn(),
  copyBlockMarkdown: vi.fn(),
  createBacklinkSibling: vi.fn(),
  copyMarkdown: vi.fn(),
  createBlock: vi.fn(),
  deleteBlock: vi.fn(),
  deletePage: vi.fn(),
  editBlock: vi.fn(),
  indentBlock: vi.fn(),
  markBlockDone: vi.fn(),
  moveBlockAfter: vi.fn(),
  moveBlockDown: vi.fn(),
  moveBlockUp: vi.fn(),
  nextDay: vi.fn(),
  openJournalFor: vi.fn(),
  openRef: vi.fn(),
  openTodayJournal: vi.fn(),
  outdentBlock: vi.fn(),
  pageBacklinks: vi.fn(),
  pasteBlockAfter: vi.fn(),
  pluginSyncHooks: vi.fn(),
  previousDay: vi.fn(),
  runCodeBlock: vi.fn(),
  setBlockCollapsed: vi.fn(),
  setBlockRemind: vi.fn(),
  snoozeReminder: vi.fn(),
  todaySlug: vi.fn(),
  toggleTodo: vi.fn(),
}));

vi.mock("./api", () => ({
  redoPage: vi.fn(),
  runCodeBlock: vi.fn(),
  undoPage: vi.fn(),
}));

import {
  backlinkRowOp,
  pageBacklinks,
  setBlockCollapsed,
} from "@outl/shared/api/commands";

import { buildHandlers } from "./action-handlers";
import { setAppState } from "./store";

function block(id: string, over: Partial<BlockNode> = {}): BlockNode {
  return {
    id,
    text: id,
    todo: null,
    tokens: [],
    collapsed: false,
    properties: [],
    children: [],
    ...over,
  };
}

const SOURCE = { id: "pg-source", slug: "source", title: "Source", kind: "page" } as const;

function row(n: BlockNode, sourcePage: Backlink["source_page"] = SOURCE): Backlink {
  return {
    block_id: n.id,
    todo: null,
    source_page: sourcePage,
    source_block: n,
    source_block_path: [0],
    ancestors: [],
  };
}

const withChild = (collapsed: boolean) =>
  block("blk-a", { collapsed, children: [block("blk-c")] });

describe("ToggleCollapsed on a focused backlink row", () => {
  const applyView = vi.fn();
  const setError = vi.fn();
  const handlers = () => buildHandlers({ applyView, setError });

  function arm(backlinks: Backlink[]) {
    setAppState({
      page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
      outline: [block("blk-1", { children: [block("blk-2")] })],
      backlinks,
      backlinksOpen: true,
      focusBlockId: null,
      selectedBlockId: null,
      selectedBacklinkBlockId: "blk-a",
      editingBlockId: null,
      editingBacklink: null,
      lastError: null,
      notice: null,
    });
  }

  beforeEach(() => {
    vi.clearAllMocks();
    const view: PageView = {
      page: { ...SOURCE },
      outline: [],
      backlinks: [],
      backlinks_order: "newest",
    };
    vi.mocked(backlinkRowOp).mockResolvedValue(view);
    vi.mocked(pageBacklinks).mockResolvedValue({
      backlinks: [],
      backlinks_order: "newest",
      namespace_backlinks: [],
      namespace_backlinks_total: 0,
      namespace_children: [],
    });
  });

  it("collapses an open block on its own page", async () => {
    arm([row(withChild(false))]);

    await handlers().ToggleCollapsed?.();

    expect(backlinkRowOp).toHaveBeenCalledWith("pg-source", "blk-a", "collapse");
    expect(setBlockCollapsed).not.toHaveBeenCalled();
    expect(applyView).not.toHaveBeenCalled();
    expect(pageBacklinks).toHaveBeenCalledWith("today");
  });

  it("expands a folded block on its own page", async () => {
    arm([row(withChild(true))]);

    await handlers().ToggleCollapsed?.();

    expect(backlinkRowOp).toHaveBeenCalledWith("pg-source", "blk-a", "expand");
    expect(setBlockCollapsed).not.toHaveBeenCalled();
  });

  it("does nothing on a leaf, as the outline does", async () => {
    arm([row(block("blk-a"))]);

    await handlers().ToggleCollapsed?.();

    expect(backlinkRowOp).not.toHaveBeenCalled();
    expect(setBlockCollapsed).not.toHaveBeenCalled();
    expect(setError).not.toHaveBeenCalled();
  });

  it("refuses on an orphan row and says why, without folding the open page", async () => {
    arm([row(withChild(false), null)]);

    await handlers().ToggleCollapsed?.();

    expect(backlinkRowOp).not.toHaveBeenCalled();
    expect(setBlockCollapsed).not.toHaveBeenCalled();
    expect(setError).toHaveBeenCalledTimes(1);
  });

  it("still folds the outline block when the cursor is in the outline", async () => {
    arm([row(withChild(false))]);
    setAppState({ selectedBacklinkBlockId: null, selectedBlockId: "blk-1" });
    vi.mocked(setBlockCollapsed).mockResolvedValue({
      page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
      outline: [],
      backlinks: [],
      backlinks_order: "newest",
    });

    await handlers().ToggleCollapsed?.();

    expect(setBlockCollapsed).toHaveBeenCalledWith("pg-1", "blk-1", true);
    expect(backlinkRowOp).not.toHaveBeenCalled();
  });
});
