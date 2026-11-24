/**
 * A focused backlink row that is a **descendant** of a citing block.
 *
 * The panel draws each citing block's subtree, so the keyboard walks it
 * (root, then its children depth-first, then the next root) and every
 * gesture addresses the row under the cursor with *that block's* id. Rows
 * are named by `BacklinkRow.key`, never by block id, because a block that
 * cites the page and also sits under another citing block is drawn twice.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import type {
  Backlink,
  BlockNode,
  PageBacklinks,
  PageView,
} from "@outl/shared/api/types";

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
  createBacklinkSibling,
  pageBacklinks,
  pluginSyncHooks,
} from "@outl/shared/api/commands";

import { buildHandlers } from "./action-handlers";
import { appState, setAppState } from "./store";
import { isEditing } from "./mode";

function block(id: string, children: BlockNode[] = []): BlockNode {
  return {
    id,
    text: id,
    todo: null,
    tokens: [],
    collapsed: false,
    properties: [],
    children,
  };
}

const SOURCE = {
  id: "pg-source",
  slug: "source",
  title: "Source",
  kind: "page",
} as const;

function link(node: BlockNode): Backlink {
  return {
    block_id: node.id,
    todo: null,
    source_page: { ...SOURCE },
    source_block: node,
    source_block_path: [0],
    ancestors: [],
  };
}

function sourceView(): PageView {
  return {
    page: { ...SOURCE },
    outline: [],
    backlinks: [],
    backlinks_order: "newest",
  };
}

function reply(rows: Backlink[]): PageBacklinks {
  return {
    backlinks: rows,
    backlinks_order: "newest",
    namespace_backlinks: [],
    namespace_backlinks_total: 0,
    namespace_children: [],
  };
}

/** a > (c1 > gc1, c2) ; b */
function tree(): Backlink[] {
  return [
    link(block("blk-a", [block("c1", [block("gc1")]), block("c2")])),
    link(block("blk-b")),
  ];
}

describe("acting on a backlink row that is a descendant", () => {
  const applyView = vi.fn();
  const setError = vi.fn();
  const handlers = () => buildHandlers({ applyView, setError });

  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(backlinkRowOp).mockResolvedValue(sourceView());
    vi.mocked(createBacklinkSibling).mockResolvedValue({
      view: sourceView(),
      new_id: "n1",
    });
    vi.mocked(pageBacklinks).mockResolvedValue(reply(tree()));
    vi.mocked(pluginSyncHooks).mockResolvedValue({ errors: [], views: [] });
    setAppState({
      page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
      outline: [block("mine")],
      backlinks: tree(),
      backlinksOpen: true,
      focusBlockId: null,
      selectedBlockId: null,
      selectedBacklinkBlockId: "blk-a",
      editingBlockId: null,
      editingBacklink: null,
      lastError: null,
      notice: null,
    });
  });

  it("j and k walk root, children depth-first, then the next root", () => {
    const h = handlers();
    const seen: Array<string | null> = [];
    for (let i = 0; i < 5; i++) {
      h.SelectionDown?.();
      seen.push(appState.selectedBacklinkBlockId);
    }
    expect(seen).toEqual([
      "blk-a>c1",
      "blk-a>c1>gc1",
      "blk-a>c2",
      "blk-b",
      "blk-b", // last row: stays
    ]);
    h.SelectionUp?.();
    expect(appState.selectedBacklinkBlockId).toBe("blk-a>c2");
  });

  it("parks Insert on the child, addressed by the child's own id", async () => {
    setAppState("selectedBacklinkBlockId", "blk-a>c1");

    await handlers().EnterInsert?.();

    expect(appState.editingBacklink).toEqual({
      pageId: "pg-source",
      blockId: "c1",
      key: "blk-a>c1",
    });
  });

  it("toggles and marks done the child, not its citing ancestor", async () => {
    setAppState("selectedBacklinkBlockId", "blk-a>c1");
    const h = handlers();

    await h.ToggleTodo?.();
    await h.MarkDone?.();

    expect(vi.mocked(backlinkRowOp).mock.calls).toEqual([
      ["pg-source", "c1", "toggle-todo"],
      ["pg-source", "c1", "mark-done"],
    ]);
  });

  it("deleting a child steps the cursor to the row above", async () => {
    setAppState("selectedBacklinkBlockId", "blk-a>c2");

    await handlers().DeleteBlock?.();

    expect(backlinkRowOp).toHaveBeenCalledWith("pg-source", "c2", "delete");
    expect(appState.selectedBacklinkBlockId).toBe("blk-a>c1>gc1");
  });

  it("deleting a root skips its own subtree when stepping off", async () => {
    setAppState("selectedBacklinkBlockId", "blk-a");

    await handlers().DeleteBlock?.();

    expect(appState.selectedBacklinkBlockId).toBe("blk-b");
  });

  it("a block drawn twice is two rows: the nested copy edits as the nested copy", async () => {
    setAppState({
      backlinks: [link(block("blk-a", [block("x")])), link(block("x"))],
      selectedBacklinkBlockId: "blk-a>x",
    });

    await handlers().EnterInsert?.();

    expect(appState.editingBacklink).toEqual({
      pageId: "pg-source",
      blockId: "x",
      key: "blk-a>x",
    });
    expect(appState.selectedBacklinkBlockId).toBe("blk-a>x");
    expect(isEditing()).toBe(true);
    // The own-row copy shares the block id but is not the one editing.
    expect(appState.editingBacklink?.key).not.toBe("x");
  });

  it("deleting a block drops the other drawn copy of its descendants from the next-cursor choice", async () => {
    setAppState({
      backlinks: [link(block("blk-a", [block("x")])), link(block("x"))],
      selectedBacklinkBlockId: "blk-a",
    });

    await handlers().DeleteBlock?.();

    // `x` as its own row dies with `blk-a`'s subtree, so nothing survives.
    expect(appState.selectedBacklinkBlockId).toBeNull();
  });

  it("a sibling made from a child lands in the refetched subtree: focus it, no notice", async () => {
    setAppState("selectedBacklinkBlockId", "blk-a>c1");
    vi.mocked(pageBacklinks).mockResolvedValue(
      reply([
        link(
          block("blk-a", [block("c1", [block("gc1")]), block("n1"), block("c2")]),
        ),
        link(block("blk-b")),
      ]),
    );

    await handlers().NewBlockBelow?.();

    expect(createBacklinkSibling).toHaveBeenCalledWith("pg-source", "c1", "below");
    expect(appState.editingBacklink).toEqual({
      pageId: "pg-source",
      blockId: "n1",
      key: "blk-a>n1",
    });
    expect(appState.notice).toBeNull();
  });
});
