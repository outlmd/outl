/**
 * Regression tests for the `OpenRefUnderCursor` handler (issue #70).
 *
 * The desktop's Normal mode has no character cursor — only a selected
 * block — so "open the ref under the cursor" cannot be resolved. An
 * earlier handler approximated it as "the first `[[ref]]` in the
 * block", which made every ref-carrying block impossible to edit via
 * `Enter`. These tests pin the fixed contract:
 *
 * 1. Selection on an outline block (refs or not) → enter Insert.
 *    Following a ref stays the click on the token (`onRefClick`).
 * 2. Selection on a backlink row (read-only) → open the source page
 *    and land the cursor on the referencing block.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Backlink, BlockNode, PageView } from "@outl/shared/api/types";

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => ({ close: vi.fn() })),
}));

vi.mock("@outl/shared/api/commands", () => ({
  copyBlockMarkdown: vi.fn(),
  createBlock: vi.fn(),
  deleteBlock: vi.fn(),
  deletePage: vi.fn(),
  editBlock: vi.fn(),
  indentBlock: vi.fn(),
  moveBlockAfter: vi.fn(),
  moveBlockDown: vi.fn(),
  moveBlockUp: vi.fn(),
  nextDay: vi.fn(),
  openJournalFor: vi.fn(),
  openRef: vi.fn(),
  openTodayJournal: vi.fn(),
  outdentBlock: vi.fn(),
  pasteBlockAfter: vi.fn(),
  previousDay: vi.fn(),
  setBlockCollapsed: vi.fn(),
  todaySlug: vi.fn(),
  toggleTodo: vi.fn(),
}));

vi.mock("./api", () => ({
  redoPage: vi.fn(),
  runCodeBlock: vi.fn(),
  undoPage: vi.fn(),
}));

import {
  copyBlockMarkdown,
  deleteBlock,
  moveBlockAfter,
  moveBlockDown,
  moveBlockUp,
  openRef,
  pasteBlockAfter,
} from "@outl/shared/api/commands";

import { buildHandlers } from "./action-handlers";
import { redoPage, undoPage } from "./api";
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

function pageView(): PageView {
  return {
    page: { id: "pg-source", slug: "source", title: "Source", kind: "page" },
    outline: [],
    backlinks: [],
    backlinks_order: "newest",
  };
}

describe("OpenRefUnderCursor (Normal-mode Enter)", () => {
  const applyView = vi.fn();
  const setError = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    setAppState({
      page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
      outline: [
        block("blk-plain", "no refs here"),
        block("blk-with-ref", "see [[some-page]] and [[another]]"),
      ],
      backlinks: [],
      selectedBlockId: null,
      selectedBacklinkBlockId: null,
      editingBlockId: null,
      backlinksOpen: true,
    });
  });

  it("enters Insert on the selected block even when it contains a [[ref]] (#70)", async () => {
    setAppState("selectedBlockId", "blk-with-ref");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.OpenRefUnderCursor?.();

    // Enter means edit — never "follow the first ref in the block".
    expect(appState.editingBlockId).toBe("blk-with-ref");
    expect(openRef).not.toHaveBeenCalled();
    expect(applyView).not.toHaveBeenCalled();
  });

  it("enters Insert on a ref-free block", async () => {
    setAppState("selectedBlockId", "blk-plain");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.OpenRefUnderCursor?.();

    expect(appState.editingBlockId).toBe("blk-plain");
    expect(openRef).not.toHaveBeenCalled();
  });

  it("does nothing when no block is selected", async () => {
    const handlers = buildHandlers({ applyView, setError });
    await handlers.OpenRefUnderCursor?.();

    expect(appState.editingBlockId).toBeNull();
    expect(openRef).not.toHaveBeenCalled();
    expect(applyView).not.toHaveBeenCalled();
  });

  it("opens the source page when the selection sits on a backlink row", async () => {
    const view = pageView();
    vi.mocked(openRef).mockResolvedValue(view);
    const backlink: Backlink = {
      block_id: "blk-source",
      todo: null,
      source_page: {
        id: "pg-source",
        slug: "source",
        title: "Source",
        kind: "page",
      },
      source_block: block("blk-source", "points at [[today]]"),
      source_block_path: [0],
      ancestors: [],
    };
    setAppState({
      backlinks: [backlink],
      selectedBlockId: null,
      selectedBacklinkBlockId: "blk-source",
    });

    const handlers = buildHandlers({ applyView, setError });
    await handlers.OpenRefUnderCursor?.();

    // Backlink rows are read-only: Enter opens, never edits.
    expect(openRef).toHaveBeenCalledWith("source");
    expect(applyView).toHaveBeenCalledWith(view);
    expect(appState.editingBlockId).toBeNull();
    // Cursor lands on the referencing block of the opened page.
    expect(appState.selectedBacklinkBlockId).toBeNull();
    expect(appState.selectedBlockId).toBe("blk-source");
  });
});

describe("Undo / Redo (Cmd+Z / Cmd+Shift+Z)", () => {
  const applyView = vi.fn();
  const setError = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    setAppState({
      page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
      outline: [block("blk-1", "hello")],
      backlinks: [],
      selectedBlockId: "blk-1",
      selectedBacklinkBlockId: null,
      editingBlockId: null,
      backlinksOpen: false,
    });
  });

  it("undoes on the current page and applies the restored view", async () => {
    const view = pageView();
    vi.mocked(undoPage).mockResolvedValue(view);

    const handlers = buildHandlers({ applyView, setError });
    await handlers.Undo?.();

    expect(undoPage).toHaveBeenCalledWith("pg-1");
    expect(applyView).toHaveBeenCalledWith(view);
    expect(setError).not.toHaveBeenCalled();
  });

  it("redoes on the current page and applies the restored view", async () => {
    const view = pageView();
    vi.mocked(redoPage).mockResolvedValue(view);

    const handlers = buildHandlers({ applyView, setError });
    await handlers.Redo?.();

    expect(redoPage).toHaveBeenCalledWith("pg-1");
    expect(applyView).toHaveBeenCalledWith(view);
  });

  it("surfaces an empty history as a status message, not a crash", async () => {
    vi.mocked(undoPage).mockRejectedValue("nothing to undo");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.Undo?.();

    expect(setError).toHaveBeenCalledWith("nothing to undo");
    expect(applyView).not.toHaveBeenCalled();
  });

  it("does nothing when no page is open", async () => {
    setAppState("page", null);

    const handlers = buildHandlers({ applyView, setError });
    await handlers.Undo?.();
    await handlers.Redo?.();

    expect(undoPage).not.toHaveBeenCalled();
    expect(redoPage).not.toHaveBeenCalled();
  });
});

/**
 * Smoke tests for the view-mode block clipboard (cut / copy / paste).
 *
 * Cut is deferred: it only arms `appState.blockClipboard`; the actual
 * move happens on paste (a single identity-preserving `Op::Move`, so
 * `((blk-…))` refs survive). Copy snapshots the subtree as markdown and
 * its paste duplicates with fresh ids. These pin the cut-vs-copy branch
 * of `PasteBlock` so the two clipboards can't quietly swap behaviour.
 */
describe("block clipboard (cut / copy / paste)", () => {
  const applyView = vi.fn();
  const setError = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    setAppState({
      page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
      outline: [block("blk-a", "a"), block("blk-b", "b")],
      backlinks: [],
      selectedBlockId: null,
      blockClipboard: null,
      editingBlockId: null,
    });
  });

  it("cut only arms the clipboard, no backend call yet", async () => {
    setAppState("selectedBlockId", "blk-a");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.CutBlock?.();

    expect(appState.blockClipboard).toEqual({ kind: "cut", nodeId: "blk-a" });
    expect(moveBlockAfter).not.toHaveBeenCalled();
  });

  it("copy snapshots the block as markdown", async () => {
    vi.mocked(copyBlockMarkdown).mockResolvedValue("- a");
    setAppState("selectedBlockId", "blk-a");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.CopyBlock?.();

    expect(copyBlockMarkdown).toHaveBeenCalledWith("blk-a");
    expect(appState.blockClipboard).toEqual({ kind: "copy", markdown: "- a" });
  });

  it("paste after a cut moves by id, consumes the clipboard, follows the block", async () => {
    const view = pageView();
    vi.mocked(moveBlockAfter).mockResolvedValue(view);
    setAppState("blockClipboard", { kind: "cut", nodeId: "blk-a" });
    setAppState("selectedBlockId", "blk-b");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.PasteBlock?.();

    expect(moveBlockAfter).toHaveBeenCalledWith("pg-1", "blk-a", "blk-b");
    expect(pasteBlockAfter).not.toHaveBeenCalled();
    expect(applyView).toHaveBeenCalledWith(view);
    // A cut is consumed by its paste; selection follows the moved block.
    expect(appState.blockClipboard).toBeNull();
    expect(appState.selectedBlockId).toBe("blk-a");
  });

  it("paste after a copy duplicates and keeps the clipboard armed", async () => {
    const view = pageView();
    vi.mocked(pasteBlockAfter).mockResolvedValue(view);
    setAppState("blockClipboard", { kind: "copy", markdown: "- a" });
    setAppState("selectedBlockId", "blk-b");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.PasteBlock?.();

    expect(pasteBlockAfter).toHaveBeenCalledWith("pg-1", "blk-b", "- a");
    expect(moveBlockAfter).not.toHaveBeenCalled();
    expect(applyView).toHaveBeenCalledWith(view);
    // A copy persists so it can be pasted again.
    expect(appState.blockClipboard).toEqual({ kind: "copy", markdown: "- a" });
  });

  it("pasting a cut onto itself is a no-op", async () => {
    setAppState("blockClipboard", { kind: "cut", nodeId: "blk-a" });
    setAppState("selectedBlockId", "blk-a");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.PasteBlock?.();

    expect(moveBlockAfter).not.toHaveBeenCalled();
    expect(applyView).not.toHaveBeenCalled();
  });
});

/**
 * Multi-select batch ops (issue #23).
 *
 * The desktop reaches a contiguous block range two ways — vim's `V`
 * and the non-vim `Shift+↑/↓` entry — and both land in the same
 * `vim-visual` selection state. These tests pin the non-vim entry
 * (`SelectRange*`) and the batch reorder (`MoveVisualRange*`) that the
 * `<BatchToolbar />` and the keyboard both fire.
 */
describe("multi-select batch ops (#23)", () => {
  const applyView = vi.fn();
  const setError = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    setAppState({
      page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
      outline: [
        block("blk-a", "a"),
        block("blk-b", "b"),
        block("blk-c", "c"),
        block("blk-d", "d"),
      ],
      backlinks: [],
      selectedBlockId: null,
      selectedBacklinkBlockId: null,
      editingBlockId: null,
      visualAnchorId: null,
      mode: "normal",
    });
  });

  it("Shift+Down (SelectRangeDown) from Normal anchors and extends into Visual", () => {
    setAppState("selectedBlockId", "blk-b");

    const handlers = buildHandlers({ applyView, setError });
    handlers.SelectRangeDown?.();

    expect(appState.mode).toBe("vim-visual");
    expect(appState.visualAnchorId).toBe("blk-b");
    expect(appState.selectedBlockId).toBe("blk-c");
  });

  it("a second SelectRangeDown keeps the anchor and grows the range", () => {
    setAppState("selectedBlockId", "blk-b");

    const handlers = buildHandlers({ applyView, setError });
    handlers.SelectRangeDown?.();
    handlers.SelectRangeDown?.();

    expect(appState.visualAnchorId).toBe("blk-b");
    expect(appState.selectedBlockId).toBe("blk-d");
  });

  it("SelectRangeUp extends upward without crossing into backlinks", () => {
    setAppState("selectedBlockId", "blk-c");

    const handlers = buildHandlers({ applyView, setError });
    handlers.SelectRangeUp?.();

    expect(appState.mode).toBe("vim-visual");
    expect(appState.visualAnchorId).toBe("blk-c");
    expect(appState.selectedBlockId).toBe("blk-b");
  });

  it("MoveVisualRangeUp moves every block in the range top-down", async () => {
    setAppState({
      mode: "vim-visual",
      visualAnchorId: "blk-b",
      selectedBlockId: "blk-c",
    });

    const handlers = buildHandlers({ applyView, setError });
    await handlers.MoveVisualRangeUp?.();

    expect(vi.mocked(moveBlockUp).mock.calls).toEqual([
      ["pg-1", "blk-b"],
      ["pg-1", "blk-c"],
    ]);
    expect(moveBlockDown).not.toHaveBeenCalled();
  });

  it("DeleteRange erases every selected line bottom-up and leaves Visual", async () => {
    vi.mocked(deleteBlock).mockResolvedValue({
      page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
      outline: [block("blk-a", "a"), block("blk-d", "d")],
      backlinks: [],
      backlinks_order: "newest",
    });
    setAppState({
      mode: "vim-visual",
      visualAnchorId: "blk-b",
      selectedBlockId: "blk-c",
    });

    const handlers = buildHandlers({ applyView, setError });
    await handlers.DeleteRange?.();

    // Bottom-up so a parent's move-to-trash can't strand a targeted child.
    expect(vi.mocked(deleteBlock).mock.calls).toEqual([
      ["pg-1", "blk-c"],
      ["pg-1", "blk-b"],
    ]);
    // Selection lands above the erased range and Visual is cleared.
    expect(appState.mode).toBe("vim-normal");
    expect(appState.visualAnchorId).toBeNull();
    expect(appState.selectedBlockId).toBe("blk-a");
  });

  it("MoveVisualRangeDown moves every block in the range bottom-up", async () => {
    setAppState({
      mode: "vim-visual",
      visualAnchorId: "blk-b",
      selectedBlockId: "blk-c",
    });

    const handlers = buildHandlers({ applyView, setError });
    await handlers.MoveVisualRangeDown?.();

    // Bottom-up: the last block clears the block below the range first.
    expect(vi.mocked(moveBlockDown).mock.calls).toEqual([
      ["pg-1", "blk-c"],
      ["pg-1", "blk-b"],
    ]);
    expect(moveBlockUp).not.toHaveBeenCalled();
  });
});

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
});

/**
 * `Cmd/Ctrl+Shift+P` (issue #13). The chord has no chip to click, so
 * it names the block and `<BlockRow />` opens a blank editor on it —
 * the handler's whole job is that hand-off, plus refusing to be a
 * no-op when nothing is selected. Silence there is how "the shortcut
 * is broken" bug reports get filed.
 */
describe("AddProperty", () => {
  const applyView = vi.fn();
  const setError = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    setAppState({
      page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
      outline: [block("blk-1", "hello")],
      selectedBlockId: null,
      addPropertyBlockId: null,
    });
  });

  it("asks the selected block's row to open a blank editor", () => {
    setAppState("selectedBlockId", "blk-1");

    buildHandlers({ applyView, setError }).AddProperty?.();

    expect(appState.addPropertyBlockId).toBe("blk-1");
    expect(setError).not.toHaveBeenCalled();
  });

  it("says why nothing happened when no block is selected", () => {
    buildHandlers({ applyView, setError }).AddProperty?.();

    expect(appState.addPropertyBlockId).toBeNull();
    expect(setError).toHaveBeenCalled();
  });
});
