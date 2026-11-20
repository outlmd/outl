/**
 * Structural chords and task state on a **focused backlink row**.
 *
 * A backlink row shows a block that lives on another page. The cursor
 * can sit on one (`backlinkCursor()`), and every mutation reachable from
 * there has to be addressed to that block's **own** page — never to the
 * page on screen, and never to `appState.selectedBlockId`, which is
 * `null` by construction while the cursor is here.
 *
 * Three failure shapes these pin, each of which a handler reaches by
 * doing the obvious thing:
 *
 * 1. **The wrong page.** `createBlock(appState.page.id, { afterId: null })`
 *    reads as "new block below" and appends to the page the user is
 *    reading, because the selection cursor is null.
 * 2. **The wrong screen.** The reply of a cross-page mutation describes
 *    the *source* page; applying it replaces the outline being read with
 *    someone else's. `backlinkCommitPlan` owns that verdict, including
 *    the one case where applying it is right (a page citing itself).
 * 3. **Silence.** A row with no page to address, or an id the list no
 *    longer holds, must say so rather than drop the keystroke.
 *
 * **Which command each gesture goes to, and what a refused one says,
 * is `backlink-commands.test.ts`'s** — the sibling this file was split
 * against at the size ratchet. Two questions, and the second one is
 * about the `block.rs` twins rather than about the cursor.
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
  createBlock,
  deleteBlock,
  editBlock,
  indentBlock,
  markBlockDone,
  moveBlockDown,
  moveBlockUp,
  outdentBlock,
  pageBacklinks,
  pluginSyncHooks,
  setBlockCollapsed,
  setBlockRemind,
  toggleTodo,
} from "@outl/shared/api/commands";

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

const SOURCE = {
  id: "pg-source",
  slug: "source",
  title: "Source",
  kind: "page",
} as const;

/** The reply of a mutation sent to the source page. */
function sourceView(): PageView {
  return {
    page: { ...SOURCE },
    outline: [block("blk-a", "cites [[today]]")],
    backlinks: [],
    backlinks_order: "newest",
  };
}

/** A `page_backlinks` reply. The default fixture answers with the rows
 *  that were already there, which is what the real scan does after an
 *  indent or a task toggle: neither changes what cites the open page. */
function backlinksReply(rows: Backlink[]): PageBacklinks {
  return {
    backlinks: rows,
    backlinks_order: "newest",
    namespace_backlinks: [],
    namespace_backlinks_total: 0,
    namespace_children: [],
  };
}

/** A block on `Source` citing the open page. */
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

describe("acting on a focused backlink row", () => {
  const applyView = vi.fn();
  const setError = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    // The cross-page pair every gesture on a row goes through. The
    // `block.rs` twins below stay mocked because the *outline* path
    // still uses them, and because "never the twin" is an assertion.
    vi.mocked(backlinkRowOp).mockResolvedValue(sourceView());
    vi.mocked(createBacklinkSibling).mockResolvedValue({
      view: sourceView(),
      new_id: "blk-new",
    });
    vi.mocked(toggleTodo).mockResolvedValue(sourceView());
    vi.mocked(markBlockDone).mockResolvedValue(sourceView());
    vi.mocked(deleteBlock).mockResolvedValue(sourceView());
    vi.mocked(indentBlock).mockResolvedValue(sourceView());
    vi.mocked(outdentBlock).mockResolvedValue(sourceView());
    vi.mocked(moveBlockUp).mockResolvedValue(sourceView());
    vi.mocked(moveBlockDown).mockResolvedValue(sourceView());
    vi.mocked(createBlock).mockResolvedValue({
      view: sourceView(),
      new_id: "blk-new",
    });
    vi.mocked(pageBacklinks).mockResolvedValue(
      backlinksReply([row("blk-a"), row("blk-b")]),
    );
    vi.mocked(pluginSyncHooks).mockResolvedValue({ errors: [], views: [] });
    setAppState({
      page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
      outline: [block("blk-1", "mine")],
      backlinks: [row("blk-a"), row("blk-b")],
      backlinksOpen: true,
      // Reset explicitly: `backlinkRowIds()` reads it, so a zoom left
      // behind by one test takes the cursor out of the section for every
      // test after it.
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

  // ── task state ──────────────────────────────────────────────────

  it("toggles the cited block's task state on its own page", async () => {
    await handlers().ToggleTodo?.();

    expect(backlinkRowOp).toHaveBeenCalledWith(
      "pg-source",
      "blk-a",
      "toggle-todo",
    );
    // The reply describes `Source`; the screen is showing `today`.
    expect(applyView).not.toHaveBeenCalled();
    expect(setError).not.toHaveBeenCalled();
  });

  it("marks the cited block done on its own page", async () => {
    await handlers().MarkDone?.();

    expect(backlinkRowOp).toHaveBeenCalledWith(
      "pg-source",
      "blk-a",
      "mark-done",
    );
    expect(applyView).not.toHaveBeenCalled();
  });

  // ── structure ───────────────────────────────────────────────────

  it("indents, outdents and moves the cited block on its own page", async () => {
    const h = handlers();
    await h.IndentBlock?.();
    await h.OutdentBlock?.();
    await h.MoveBlockUp?.();
    await h.MoveBlockDown?.();

    expect(vi.mocked(backlinkRowOp).mock.calls).toEqual([
      ["pg-source", "blk-a", "indent"],
      ["pg-source", "blk-a", "outdent"],
      ["pg-source", "blk-a", "move-up"],
      ["pg-source", "blk-a", "move-down"],
    ]);
    expect(applyView).not.toHaveBeenCalled();
  });

  it("deletes the cited block and steps the cursor onto a row that survives", async () => {
    await handlers().DeleteBlock?.();

    expect(backlinkRowOp).toHaveBeenCalledWith("pg-source", "blk-a", "delete");
    // `blk-a` is about to leave the list; its row is the one the cursor
    // is on, and a cursor on a row nobody draws answers no keystroke.
    expect(appState.selectedBacklinkBlockId).toBe("blk-b");
    expect(appState.selectedBlockId).toBeNull();
  });

  it("steps the cursor back up when the deleted row was the last one", async () => {
    setAppState("selectedBacklinkBlockId", "blk-b");

    await handlers().DeleteBlock?.();

    expect(backlinkRowOp).toHaveBeenCalledWith("pg-source", "blk-b", "delete");
    expect(appState.selectedBacklinkBlockId).toBe("blk-a");
  });

  it("leaves the cursor nowhere when the deleted row was the only one", async () => {
    setAppState("backlinks", [row("blk-a")]);

    await handlers().DeleteBlock?.();

    expect(appState.selectedBacklinkBlockId).toBeNull();
    expect(appState.selectedBlockId).toBeNull();
  });

  // ── creating a sibling ──────────────────────────────────────────
  //
  // Which *command* `o` / `O` reach, and why it is not `createBlock`,
  // is `backlink-commands.test.ts`'s. What is left here is where the
  // cursor and the notice end up.

  it("says where the new block went, and moves nothing on screen", async () => {
    await handlers().NewBlockBelow?.();

    expect(applyView).not.toHaveBeenCalled();
    // The new block cites nothing, so no row here renders it and there
    // is nowhere to put the cursor. Saying so is the behaviour.
    expect(appState.notice?.join(" ")).toContain("Source");
    expect(appState.editingBlockId).toBeNull();
    expect(appState.selectedBacklinkBlockId).toBe("blk-a");
  });

  // ── a page citing itself ────────────────────────────────────────

  it("applies the reply when the cited block lives on the page on screen", async () => {
    setAppState("page", { ...SOURCE });

    await handlers().ToggleTodo?.();

    // `backlinkCommitPlan`'s one `applyView: true` case: discarding the
    // reply here would drop a genuine refresh of the open outline.
    expect(applyView).toHaveBeenCalledTimes(1);
  });

  it("edits the new block when the source page is the page on screen", async () => {
    setAppState("page", { ...SOURCE });

    await handlers().NewBlockBelow?.();

    // It *is* drawn here, so parking the editor on it is honest and the
    // "you cannot see it" notice would be a lie.
    expect(appState.editingBlockId).toBe("blk-new");
    expect(appState.selectedBlockId).toBe("blk-new");
    expect(appState.selectedBacklinkBlockId).toBeNull();
    expect(appState.notice).toBeNull();
  });

  // ── the refresh ─────────────────────────────────────────────────

  it("re-asks for the backlinks once per commit, for the open page", async () => {
    await handlers().IndentBlock?.();

    expect(vi.mocked(pageBacklinks).mock.calls).toEqual([["today"]]);
  });

  // ── falling through ─────────────────────────────────────────────

  it("falls through to the outline when the cursor is not in the backlinks", async () => {
    setAppState({
      selectedBlockId: "blk-1",
      selectedBacklinkBlockId: null,
    });

    const h = handlers();
    await h.IndentBlock?.();
    await h.NewBlockBelow?.();

    expect(indentBlock).toHaveBeenCalledWith("pg-1", "blk-1");
    expect(createBlock).toHaveBeenCalledWith("pg-1", {
      afterId: "blk-1",
      parentId: null,
      text: "",
    });
    // The outline keeps the tolerant twins; the cross-page pair is not
    // reached at all from here.
    expect(backlinkRowOp).not.toHaveBeenCalled();
    expect(createBacklinkSibling).not.toHaveBeenCalled();
    expect(pageBacklinks).not.toHaveBeenCalled();
  });

  // ── Tab means two things on one row ─────────────────────────────

  it("addresses the source page even with the row's own textarea focused", async () => {
    // `<BacklinkRowEditor />` mounts a `<BlockEditor blockId={…}>`
    // carrying the **cited** block's id, while `appState.page` is still
    // the page on screen. `targetBlockId()` reads the focused textarea
    // first, so pairing its answer with `appState.page.id` writes to the
    // wrong document — which is why the backlink branch comes before it.
    //
    // The two Tabs do not collide: the editor's own `Tab` is Insert and
    // commits (`<BlockEditor />` calls `stopImmediatePropagation`), while
    // the catalog's `IndentBlock` is Normal and lands here. This pins the
    // half that would be silently wrong if the order were reversed.
    const ta = document.createElement("textarea");
    ta.dataset.blockId = "blk-a";
    document.body.append(ta);
    ta.focus();
    try {
      await handlers().IndentBlock?.();

      expect(backlinkRowOp).toHaveBeenCalledWith(
        "pg-source",
        "blk-a",
        "indent",
      );
      expect(indentBlock).not.toHaveBeenCalled();
    } finally {
      ta.remove();
    }
  });

  // ── entering Insert, through the same owner ─────────────────────

  it("parks Insert on the row from `i`", async () => {
    await handlers().EnterInsert?.();

    expect(appState.editingBacklink).toEqual({
      pageId: "pg-source",
      blockId: "blk-a",
    });
    expect(appState.editingBlockId).toBeNull();
  });

  // ── the section has to be on screen ─────────────────────────────

  it("does nothing once the panel is closed under the cursor", async () => {
    // `ToggleBacklinks` is **Global** (`defaults.rs`), so `Cmd+Shift+B`
    // fires from anywhere, and `togglePanel` flips the flag and nothing
    // else — no writer of `selectedBacklinkBlockId` is reached, and
    // removing a node from the DOM does not reliably fire `blur`. So the
    // cursor is still on a row the section no longer draws, and a
    // mutation from here edits a page nothing on screen shows.
    setAppState("backlinksOpen", false);

    const h = handlers();
    await h.DeleteBlock?.();
    await h.IndentBlock?.();
    await h.ToggleTodo?.();
    await h.MarkDone?.();

    // Nothing reaches the other page. These fall through to the outline
    // instead, where nothing is selected, so they are the no-op they
    // were before backlink rows became editable.
    expect(backlinkRowOp).not.toHaveBeenCalled();
    expect(deleteBlock).not.toHaveBeenCalled();
    expect(indentBlock).not.toHaveBeenCalled();
    expect(toggleTodo).not.toHaveBeenCalled();
    // Not a refusal either: the cursor is simply not in this section.
    expect(setError).not.toHaveBeenCalledWith(
      expect.stringContaining("reference"),
    );
  });

  it("creates on the page being read, never the source, once the panel is closed", async () => {
    setAppState("backlinksOpen", false);

    await handlers().NewBlockBelow?.();

    // `o` with nothing selected appends at the open page's end, which is
    // what it has always done and is the page the user is looking at.
    // What it must never be is `pg-source`.
    expect(createBlock).toHaveBeenCalledWith("pg-1", {
      afterId: null,
      parentId: null,
      text: "",
    });
    expect(createBacklinkSibling).not.toHaveBeenCalled();
  });

  it("does nothing while zoomed, for the same reason", async () => {
    // `<PageSections zoomed>` draws nothing of the page's own, so the
    // rows are off screen with `backlinksOpen` still true.
    setAppState("focusBlockId", "blk-1");

    const h = handlers();
    await h.DeleteBlock?.();
    await h.IndentBlock?.();
    await h.NewBlockBelow?.();

    expect(backlinkRowOp).not.toHaveBeenCalled();
    expect(createBacklinkSibling).not.toHaveBeenCalled();
  });

  // ── nothing to act on ───────────────────────────────────────────

  it("does nothing on an orphan backlink, and says why", async () => {
    setAppState("backlinks", [{ ...row("blk-a"), source_page: null }]);

    const h = handlers();
    await h.IndentBlock?.();
    await h.DeleteBlock?.();
    await h.NewBlockBelow?.();
    await h.EnterInsert?.();
    await h.OpenRefUnderCursor?.();

    // `i` and `Enter` used to be silent here: the row resolution answered
    // `false` and neither caller read it. Entering Insert is now the same
    // owner as the mutations, so one sentence covers all of them.
    expect(appState.editingBacklink).toBeNull();
    expect(appState.editingBlockId).toBeNull();

    // No page to address: the citing block lives outside any page, and
    // guessing one writes to the wrong document.
    expect(backlinkRowOp).not.toHaveBeenCalled();
    // Above all: it must NOT fall through, which would append a block to
    // the page the user is reading.
    expect(createBlock).not.toHaveBeenCalled();
    expect(createBacklinkSibling).not.toHaveBeenCalled();
    expect(setError).toHaveBeenCalledTimes(5);
  });

  it("says so when the cursor names a row the list no longer holds", async () => {
    setAppState("selectedBacklinkBlockId", "blk-gone");

    const h = handlers();
    await h.IndentBlock?.();
    await h.NewBlockBelow?.();

    expect(backlinkRowOp).not.toHaveBeenCalled();
    expect(createBacklinkSibling).not.toHaveBeenCalled();
    expect(setError).toHaveBeenCalledTimes(2);
  });

  // ── the guard is hand-written, so enumerate ─────────────────────

  it("no handler in the whole map mutates the page being read from a row", async () => {
    // Ten sites consult `onBacklink` by hand and nothing makes an
    // eleventh. So this calls **every** handler in the map with the
    // cursor on a backlink row and fails if any mutation command is
    // addressed to `pg-1`, the page on screen.
    //
    // That is the shape of the bug this task fixed twice: `o` and `O`
    // appended an empty block to the document the user was reading,
    // because the selection cursor is null here and `{ afterId: null }`
    // means "at the end". A future handler that forgets the branch is
    // caught here rather than by a user watching the wrong page change.
    const watched = [
      backlinkRowOp,
      createBacklinkSibling,
      toggleTodo,
      markBlockDone,
      deleteBlock,
      indentBlock,
      outdentBlock,
      moveBlockUp,
      moveBlockDown,
      createBlock,
      editBlock,
      setBlockCollapsed,
      setBlockRemind,
    ];
    // Handlers whose unit is the **page**, not the cursor. `zM` / `zR`
    // mean "fold / unfold everything on this page" wherever the cursor
    // happens to be, so addressing the open page is their right answer
    // rather than a missed branch. Declared with a reason instead of
    // dropping `setBlockCollapsed` from the watch list, which would also
    // stop covering the cursor-targeted `ToggleCollapsed`.
    const pageScoped: Record<string, string> = {
      FoldAll: "zM folds every parent on the open page, by definition",
      UnfoldAll: "zR unfolds every block on the open page, by definition",
    };
    const h = handlers();
    /** The cursor on a row, over an outline with a **parent**, so the
     *  two page-scoped fold handlers genuinely fire rather than finding
     *  nothing to fold — otherwise their exemption would be satisfied by
     *  an empty fixture instead of by being true. */
    const armRow = () =>
      setAppState({
        page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
        outline: [{ ...block("blk-p", "parent"), children: [block("blk-c", "child")] }],
        backlinks: [row("blk-a"), row("blk-b")],
        backlinksOpen: true,
        focusBlockId: null,
        selectedBlockId: null,
        selectedBacklinkBlockId: "blk-a",
        editingBlockId: null,
        editingBacklink: null,
        blockClipboard: null,
        visualAnchorId: null,
      });

    for (const action of Object.keys(h)) {
      if (action in pageScoped) continue;
      vi.clearAllMocks(); // keeps the resolved values; drops the calls
      armRow();

      try {
        await h[action as keyof typeof h]?.();
      } catch {
        // A chrome handler may reach a DOM API jsdom does not implement
        // (`window.confirm`). What is under test is where a mutation was
        // addressed, not whether every handler runs headless.
      }

      for (const cmd of watched) {
        for (const call of vi.mocked(cmd).mock.calls) {
          expect(
            call[0],
            `${action} mutated the open page while the cursor was on a backlink row — route it through onBacklink, or declare it in pageScoped with a reason`,
          ).not.toBe("pg-1");
        }
      }
    }

    // **An exemption has to stay true, not merely stay named.**
    // Each exempt handler must actually address the open page, so the
    // row fails the moment it stops being needed — if one is later
    // routed through `onBacklink`, an exemption left behind would go on
    // hiding every future regression on it. Checking only that the
    // handler still exists catches a deletion and nothing else.
    for (const action of Object.keys(pageScoped)) {
      expect(Object.keys(h)).toContain(action);
      vi.clearAllMocks();
      armRow();
      await h[action as keyof typeof h]?.();
      const hitOpenPage = watched.some((cmd) =>
        vi.mocked(cmd).mock.calls.some((call) => call[0] === "pg-1"),
      );
      expect(
        hitOpenPage,
        `${action} no longer mutates the open page from a backlink row, so its pageScoped exemption is stale — drop it`,
      ).toBe(true);
    }
  });
});
