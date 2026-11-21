/**
 * **Which command** a gesture on a focused backlink row reaches, and
 * what a refused one says.
 *
 * Sibling of `backlink-handlers.test.ts`, split off at the size ratchet
 * on a real boundary: that file asks *which block on which page* a
 * keystroke addresses and where the cursor lands afterwards. This one
 * asks a question about the **other** commands — the `block.rs` twins
 * the outline uses — because every one of these gestures used to go
 * through them, and that is the whole defect:
 *
 * - `createBlock` recovers from a stale anchor by appending at the end
 *   of the page it was given. That is a kindness on the page the user
 *   is reading and a silent move to a page they are not, so `o` / `O`
 *   on a row whose id had gone stale created an empty block at the end
 *   of the source page — the exact recovery the
 *   `edit_block` / `edit_backlink_block` split exists to refuse,
 *   reached through a different door.
 * - the other eight reported a stale id as `ActionError`'s `Display`,
 *   which is `"block 01J… is not in the tree"`, while the catalog's
 *   sentence for that event
 *   (`outl_actions::refusal::stale_backlink_block_unchanged_refusal`)
 *   was called only by the TUI. Root `CLAUDE.md` invariant 12.
 * - three of them did not even report: `moveBlockUp`, `moveBlockDown`
 *   and `indentBlock` answer success when they find no sibling, so a
 *   stale row id was a silent nothing.
 *
 * The fix is one cross-page command family, so the net is the same
 * shape: **a twin called at all from a row is a failure**, whatever
 * page id it was handed. Asserting only "not with `pg-1`" would pass a
 * gesture that went to the tolerant command with the right page, which
 * is precisely what `o` did.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Backlink, BlockNode, PageView } from "@outl/shared/api/types";

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => ({ close: vi.fn() })),
}));

vi.mock("@outl/shared/api/commands", () => ({
  backlinkRowOp: vi.fn(),
  copyBlockMarkdown: vi.fn(),
  copyMarkdown: vi.fn(),
  createBacklinkSibling: vi.fn(),
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
  createBlock,
  deleteBlock,
  indentBlock,
  markBlockDone,
  moveBlockDown,
  moveBlockUp,
  outdentBlock,
  pageBacklinks,
  pluginSyncHooks,
  toggleTodo,
} from "@outl/shared/api/commands";

import { buildHandlers } from "./action-handlers";
import { setAppState } from "./store";

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

const SOURCE = {
  id: "pg-source",
  slug: "source",
  title: "Source",
  kind: "page",
} as const;

function sourceView(): PageView {
  return {
    page: { ...SOURCE },
    outline: [block("blk-a", "cites [[today]]")],
    backlinks: [],
    backlinks_order: "newest",
  };
}

function row(id: string): Backlink {
  return {
    block_id: id,
    todo: null,
    source_page: { ...SOURCE },
    source_block: block(id, "cites [[today]]"),
    source_block_path: [0],
    ancestors: [],
  };
}

/**
 * Every `block.rs` command a cross-page gesture must not reach.
 *
 * `createBlock` is the one that silently *succeeded* on a stale anchor;
 * the rest are the ones that reported a ULID. They are watched together
 * because the user cannot tell the two failures apart — both are "the
 * keystroke went somewhere I cannot see".
 */
const TOLERANT_TWINS: Array<[string, (...args: never[]) => unknown]> = [
  ["toggleTodo", toggleTodo],
  ["markBlockDone", markBlockDone],
  ["deleteBlock", deleteBlock],
  ["indentBlock", indentBlock],
  ["outdentBlock", outdentBlock],
  ["moveBlockUp", moveBlockUp],
  ["moveBlockDown", moveBlockDown],
  ["createBlock", createBlock],
];

describe("which command a backlink row's gesture reaches", () => {
  const applyView = vi.fn();
  const setError = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(backlinkRowOp).mockResolvedValue(sourceView());
    vi.mocked(createBacklinkSibling).mockResolvedValue({
      view: sourceView(),
      new_id: "blk-new",
    });
    // Resolved so a twin reached by mistake does not *also* fail on an
    // unhandled rejection: the assertion has to be "it was called",
    // not "it threw".
    for (const [, twin] of TOLERANT_TWINS) {
      vi.mocked(twin).mockResolvedValue(sourceView() as never);
    }
    vi.mocked(createBlock).mockResolvedValue({
      view: sourceView(),
      new_id: "blk-new",
    });
    vi.mocked(pageBacklinks).mockResolvedValue({
      backlinks: [row("blk-a")],
      backlinks_order: "newest",
      namespace_backlinks: [],
      namespace_backlinks_total: 0,
      namespace_children: [],
    });
    vi.mocked(pluginSyncHooks).mockResolvedValue({ errors: [], views: [] });
    setAppState({
      page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
      outline: [block("blk-1", "mine")],
      backlinks: [row("blk-a"), row("blk-b")],
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

  const handlers = () => buildHandlers({ applyView, setError });

  /**
   * The seven chords, each paired with the catalog tag it must send.
   *
   * Written out rather than derived, because the pairing *is* the thing
   * under test: `onTarget` takes the tag and the outline's command side
   * by side, so a swap between two rows here is a keystroke that
   * performs the wrong mutation on another page.
   */
  const CHORD_TAGS: Array<[string, string]> = [
    ["ToggleTodo", "toggle-todo"],
    ["MarkDone", "mark-done"],
    ["DeleteBlock", "delete"],
    ["IndentBlock", "indent"],
    ["OutdentBlock", "outdent"],
    ["MoveBlockUp", "move-up"],
    ["MoveBlockDown", "move-down"],
  ];

  it.each(CHORD_TAGS)(
    "%s goes to backlink_row_op as %s, never a block.rs twin",
    async (action, tag) => {
      const h = handlers();
      await h[action as keyof typeof h]?.();

      expect(backlinkRowOp).toHaveBeenCalledWith("pg-source", "blk-a", tag);
      for (const [name, twin] of TOLERANT_TWINS) {
        expect(twin, `${action} reached ${name}`).not.toHaveBeenCalled();
      }
    },
  );

  it.each([
    ["NewBlockBelow", "below"],
    ["NewBlockAbove", "above"],
  ])("%s creates through create_backlink_sibling (%s)", async (action, side) => {
    const h = handlers();
    await h[action as keyof typeof h]?.();

    expect(createBacklinkSibling).toHaveBeenCalledWith(
      "pg-source",
      "blk-a",
      side,
    );
    // The one that mattered: `createBlock` resolves its anchor through
    // `create_after_or_append`, so a stale id lands an empty block at
    // the end of a page the user is not looking at — and answers `Ok`,
    // so nothing on screen says anything happened.
    expect(createBlock).not.toHaveBeenCalled();
  });

  /**
   * The backend owns the sentence; this client owns where it goes.
   *
   * Asserted as an exact forward of whatever the command rejected with,
   * not as a match on wording: the wording is
   * `outl_actions::refusal`'s, pinned in
   * `outl-tauri-shared/tests/backlink_row_ops.rs`, and a copy of it
   * here would be the second owner invariant 12 forbids.
   */
  it("forwards a refusal verbatim and re-asks nothing", async () => {
    vi.mocked(backlinkRowOp).mockRejectedValue(
      "That block was deleted, so nothing changed.",
    );

    await handlers().IndentBlock?.();

    expect(setError).toHaveBeenCalledWith(
      "That block was deleted, so nothing changed.",
    );
    // Nothing landed, so the `O(blocks-in-workspace)` scan is not owed.
    expect(pageBacklinks).not.toHaveBeenCalled();
    expect(applyView).not.toHaveBeenCalled();
  });

  it("forwards a refused create the same way", async () => {
    vi.mocked(createBacklinkSibling).mockRejectedValue(
      "That block could not be found, so nothing changed.",
    );

    await handlers().NewBlockBelow?.();

    expect(setError).toHaveBeenCalledWith(
      "That block could not be found, so nothing changed.",
    );
    expect(pageBacklinks).not.toHaveBeenCalled();
  });
});
