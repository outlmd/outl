/**
 * The cursor is in one region at a time, and nothing may put it in two.
 *
 * `selectedBlockId` (outline) and `selectedBacklinkBlockId` (the
 * read-only backlinks section below it) are declared mutually exclusive
 * in `store.ts`, because `SelectionDown` / `SelectionUp` /
 * `OpenRefUnderCursor` all read the backlink one **first**. With both
 * set, the keyboard is in the backlinks while the outline still draws a
 * highlight on the block the user left the cursor on — so `j` walks
 * someone else's page and `Enter` navigates away instead of editing.
 *
 * That exclusion used to be a sentence rather than a mechanism: ~20 call
 * sites wrote one field directly and each had to remember to clear the
 * other. These pin the two doors that did not, and the walk that made it
 * visible. The TUI has no equivalent test because it cannot reach the
 * state: `Focus` is an enum there, so the compiler is the net.
 */
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { Backlink, BlockNode } from "@outl/shared/api/types";

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
  pageBacklinks: vi.fn(),
  pluginSyncHooks: vi.fn(),
  previousDay: vi.fn(),
  resolveEmbeds: vi.fn(() => Promise.resolve([])),
  resolvePageEmbeds: vi.fn(() => Promise.resolve([])),
  runCodeBlock: vi.fn(),
  setBacklinksOrder: vi.fn(),
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

import { InlineBacklinks } from "../components/InlineBacklinks";
import { buildHandlers } from "./action-handlers";
import { appState, backlinkCursor, setAppState, setOutline } from "./store";

function block(id: string): BlockNode {
  return {
    id,
    text: id,
    todo: null,
    tokens: [{ kind: "plain", value: id }],
    collapsed: false,
    properties: [],
    children: [],
  };
}

function backlink(id: string): Backlink {
  return {
    block_id: id,
    todo: null,
    ancestors: [],
    source_page: {
      id: "other-page",
      slug: "other",
      title: "Other",
      kind: "page",
    },
    source_block: {
      id,
      text: "cites this page",
      todo: null,
      tokens: [{ kind: "plain", value: "cites this page" }],
      collapsed: false,
      properties: [],
      children: [],
    },
    source_block_path: [],
  };
}

let dispose: (() => void) | undefined;

/** The page the user is reading: three blocks, and one other page
 *  citing it. Cursor parked on the middle block, as `j` would leave it. */
function mountPage(): HTMLElement {
  setAppState({
    page: { id: "page-1", slug: "project", title: "project", kind: "page" },
    backlinksOpen: true,
    backlinksLoading: false,
    backlinks: [backlink("src-1")],
    backlinksOrder: "newest",
    focusBlockId: null,
    selectedBacklinkBlockId: null,
  });
  setOutline([block("blk-1"), block("blk-2"), block("blk-3")]);
  setAppState("selectedBlockId", "blk-2");
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <InlineBacklinks />, host);
  return host;
}

/** The backlink's text button — the one carrying the row's handlers. */
function backlinkRow(host: HTMLElement): HTMLButtonElement {
  const found = Array.from(host.querySelectorAll("button")).find((b) =>
    b.textContent?.includes("cites this page"),
  );
  if (!found) throw new Error("backlink row not rendered");
  return found as HTMLButtonElement;
}

function handlers() {
  return buildHandlers({
    applyView: vi.fn(),
    setError: vi.fn(),
  } as unknown as Parameters<typeof buildHandlers>[0]);
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  vi.clearAllMocks();
  setAppState({
    page: null,
    backlinks: [],
    backlinksOpen: true,
    selectedBlockId: null,
    selectedBacklinkBlockId: null,
    focusBlockId: null,
  });
  setOutline([]);
  document.body.innerHTML = "";
});

describe("the cursor lives in one region", () => {
  it("a mouse crossing the backlinks section does not take the cursor", () => {
    const host = mountPage();

    // The user is not aiming at anything — the pointer passes over the
    // section on its way somewhere else. Hover is a highlight (the row
    // has `hover:bg-…` for it), never a cursor move.
    backlinkRow(host).dispatchEvent(new MouseEvent("mouseenter"));

    expect(appState.selectedBacklinkBlockId).toBeNull();
    expect(appState.selectedBlockId).toBe("blk-2");
  });

  it("j after the mouse crossed the backlinks still walks the outline", () => {
    const host = mountPage();

    backlinkRow(host).dispatchEvent(new MouseEvent("mouseenter"));
    handlers().SelectionDown?.();

    // The reported symptom: the cursor was left on blk-2, so `j` owes
    // blk-3. Reading the backlink field first sent it to someone
    // else's page instead.
    expect(appState.selectedBlockId).toBe("blk-3");
    expect(appState.selectedBacklinkBlockId).toBeNull();
  });

  it("Enter after the mouse crossed the backlinks edits the selected block", () => {
    const host = mountPage();

    backlinkRow(host).dispatchEvent(new MouseEvent("mouseenter"));
    handlers().OpenRefUnderCursor?.();

    // Normal-mode Enter on an outline block means "edit it". With the
    // backlink field set it opened the citing page instead, losing the
    // user's place entirely.
    expect(appState.editingBlockId).toBe("blk-2");
  });

  it("a backlink id left behind paints no accent bar on its row", () => {
    const host = mountPage();
    const row = () => backlinkRow(host).closest("li")!.className;

    // The cursor genuinely crossed into the backlinks, so the row owns
    // the highlight.
    setAppState("selectedBlockId", null);
    setAppState("selectedBacklinkBlockId", "src-1");
    expect(row()).toContain("before:bg-(--color-outl-accent)");

    // Then the user clicks a block in the outline. Most writers of
    // `selectedBlockId` do not clear the backlink field, so the id
    // stays behind — what must not stay is the second cursor on screen.
    setAppState("selectedBlockId", "blk-1");

    expect(row()).toBe("");
  });

  it("the outline holds the cursor until it releases it", () => {
    mountPage();
    // `selectedBlockId` set is the whole condition: the two handlers
    // that cross the separator null it in the same breath, so no other
    // writer has to know this rule exists.
    setAppState("selectedBacklinkBlockId", "src-1");
    expect(backlinkCursor()).toBeNull();

    setAppState("selectedBlockId", null);
    expect(backlinkCursor()).toBe("src-1");
  });
});
