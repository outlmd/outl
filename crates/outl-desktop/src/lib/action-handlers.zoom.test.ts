/**
 * Zoom, and the dead end it used to leave behind.
 *
 * Zoom is local view state (`appState.focusBlockId`), never an op, so
 * these drive the handlers against the store and assert on it. Split
 * from `action-handlers.test.ts` because zoom is its own question and
 * that file was at the size ceiling.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { BlockNode } from "@outl/shared/api/types";

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => ({ close: vi.fn() })),
}));

vi.mock("@outl/shared/api/commands", () => ({
  copyBlockMarkdown: vi.fn(),
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

import { openTodayJournal } from "@outl/shared/api/commands";

import { buildHandlers } from "./action-handlers";
import { appState, setAppState } from "./store";

function block(id: string, text: string): BlockNode {
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

/**
 * Zoom / focus (ZoomIn / ZoomOut).
 *
 * Zoom is local view state (`appState.focusBlockId`), never an op — the
 * handlers only flip the store. The desktop keeps no zoom stack, so
 * `ZoomOut` derives the parent from the current focus's breadcrumb
 * (`focusSubtree`): pop to the last crumb, or exit when the focused
 * block is already top-level. These pin that derivation.
 */
describe("zoom / focus (ZoomIn / ZoomOut)", () => {
  const applyView = vi.fn();
  const setError = vi.fn();

  function nested(id: string, text: string, children: BlockNode[]): BlockNode {
    return { ...block(id, text), children };
  }

  beforeEach(() => {
    vi.clearAllMocks();
    setAppState({
      page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
      // top → mid → leaf, plus a top-level sibling.
      outline: [
        nested("blk-top", "top", [
          nested("blk-mid", "mid", [block("blk-leaf", "leaf")]),
        ]),
        block("blk-sibling", "sibling"),
      ],
      backlinks: [],
      selectedBlockId: null,
      focusBlockId: null,
    });
  });

  it("ZoomIn focuses the selected block (view state, no op)", () => {
    setAppState("selectedBlockId", "blk-mid");

    const handlers = buildHandlers({ applyView, setError });
    handlers.ZoomIn?.();

    expect(appState.focusBlockId).toBe("blk-mid");
    expect(applyView).not.toHaveBeenCalled();
  });

  it("ZoomIn is a no-op with nothing selected", () => {
    const handlers = buildHandlers({ applyView, setError });
    handlers.ZoomIn?.();

    expect(appState.focusBlockId).toBeNull();
  });

  /**
   * `z i` on a block with no children shows an empty page — the outline
   * it zooms into is `[]`, so there is no row for j/k to reach. Every
   * movement key then went quiet while a row still looked parked, and
   * `z o` was the only way out: a chord has to be known about before it
   * can be pressed. Reported from the field as "it is visually on the
   * block but j/k and the arrows do nothing".
   *
   * `k` leaves the zoom, which is the rule the backlinks section
   * already uses (stepping up off the first backlink returns to the
   * outline). `j` deliberately keeps doing nothing: down is not a way
   * out of anything, and the page below is empty either way.
   */
  it("k leaves a zoom that has nothing above it", () => {
    setAppState({ focusBlockId: "blk-leaf", selectedBlockId: null });

    const handlers = buildHandlers({ applyView, setError });
    handlers.SelectionUp?.();

    expect(appState.focusBlockId).toBe("blk-mid");
  });

  it("k at the top of a zoom that has rows does not leave it", () => {
    // blk-top holds blk-mid, so this zoom is not a dead end. `prev` is
    // null here all the same (blk-mid is its first row), and escaping
    // on that would throw the reader out of a subtree they are reading.
    // The first version of the rescue did exactly that.
    setAppState({ focusBlockId: "blk-top", selectedBlockId: "blk-mid" });

    const handlers = buildHandlers({ applyView, setError });
    handlers.SelectionUp?.();

    expect(appState.focusBlockId).toBe("blk-top");
  });

  it("k at the top of an unzoomed page changes nothing", () => {
    // The guard that rescues a zoom must not fire here: there is no
    // zoom to leave, and `k` on the first block has always been a
    // no-op rather than a wrap.
    setAppState({ focusBlockId: null, selectedBlockId: "blk-top" });

    const handlers = buildHandlers({ applyView, setError });
    handlers.SelectionUp?.();

    expect(appState.focusBlockId).toBeNull();
    expect(appState.selectedBlockId).toBe("blk-top");
  });

  it("ZoomOut pops to the parent via the breadcrumb (no stack)", () => {
    setAppState("focusBlockId", "blk-leaf");

    const handlers = buildHandlers({ applyView, setError });
    handlers.ZoomOut?.();

    // Breadcrumb of blk-leaf is [top, mid]; last crumb (mid) is the parent.
    expect(appState.focusBlockId).toBe("blk-mid");
  });

  it("ZoomOut exits the zoom when the focused block is top-level", () => {
    setAppState("focusBlockId", "blk-top");

    const handlers = buildHandlers({ applyView, setError });
    handlers.ZoomOut?.();

    // Empty breadcrumb → exit to the full page.
    expect(appState.focusBlockId).toBeNull();
  });

  it("ZoomOut clears a stale focus (block no longer in the outline)", () => {
    setAppState("focusBlockId", "blk-gone");

    const handlers = buildHandlers({ applyView, setError });
    handlers.ZoomOut?.();

    expect(appState.focusBlockId).toBeNull();
  });

  /**
   * Asking for a page means the whole page, including when it is the
   * page already open.
   *
   * The zoom used to be dropped by one thing only: `installPageChangeEffect`
   * noticing a different page id. So `Cmd+J` zoomed into a block of
   * today's journal navigated to today, landed on the same id, and the
   * reset never ran — the user pressed the one chord that means "take me
   * back to today" and the screen did not move.
   */
  it("OpenToday leaves a zoom held on today's journal", async () => {
    setAppState({ focusBlockId: "blk-mid", selectedBlockId: "blk-leaf" });
    vi.mocked(openTodayJournal).mockResolvedValue({
      page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
      outline: [],
      backlinks: [],
      backlinks_order: "newest",
    });

    const handlers = buildHandlers({ applyView, setError });
    await handlers.OpenToday?.();

    expect(appState.focusBlockId).toBeNull();
    expect(applyView).toHaveBeenCalledOnce();
  });

  it("a failed OpenToday keeps the zoom", async () => {
    // Nothing navigated, so there is nothing to come back out to: the
    // error is on the status line and the subtree the user was reading
    // stays on screen.
    setAppState("focusBlockId", "blk-mid");
    vi.mocked(openTodayJournal).mockRejectedValue(new Error("no workspace"));

    const handlers = buildHandlers({ applyView, setError });
    await handlers.OpenToday?.();

    expect(appState.focusBlockId).toBe("blk-mid");
    expect(applyView).not.toHaveBeenCalled();
  });

  /**
   * `j` off the last row of a zoom used to cross into the page's
   * backlinks — a section that is not on screen while zoomed, so the
   * cursor left the outline for nowhere.
   */
  it("j stays inside the zoom instead of entering the page's backlinks", () => {
    setAppState({
      focusBlockId: "blk-mid",
      selectedBlockId: "blk-leaf",
      backlinks: [
        {
          block_id: "blk-citing",
          todo: null,
          ancestors: [],
          source_page: {
            id: "pg-2",
            slug: "other",
            title: "Other",
            kind: "page",
          },
          source_block: block("blk-citing", "cites today"),
          source_block_path: [],
        },
      ],
    });

    const handlers = buildHandlers({ applyView, setError });
    handlers.SelectionDown?.();

    expect(appState.selectedBacklinkBlockId).toBeNull();
    expect(appState.selectedBlockId).toBe("blk-leaf");
  });
});
