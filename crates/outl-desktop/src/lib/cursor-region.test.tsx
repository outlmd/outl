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
import { PageSections } from "../components/PageSections";
import { buildHandlers } from "./action-handlers";
import { backlinkCursor } from "./cursor-region";
import { isEditing } from "./mode";
import { appState, setAppState, setOutline, zoomView } from "./store";

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
    // Both editing ids, explicitly: `isEditing()` reads them, and one
    // left behind by an earlier test reports an editor on screen that
    // this page never mounted.
    editingBlockId: null,
    editingBacklink: null,
  });
  setOutline([block("blk-1"), block("blk-2"), block("blk-3")]);
  setAppState("selectedBlockId", "blk-2");
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <InlineBacklinks />, host);
  return host;
}

/** The backlink's text — the outline's `<BlockBody />` prose, which
 *  carries the row's handlers. */
function backlinkRow(host: HTMLElement): HTMLElement {
  const found = Array.from(host.querySelectorAll<HTMLElement>("[data-backlink-body] .cursor-text")).find((b) =>
    b.textContent?.includes("cites this page"),
  );
  if (!found) throw new Error("backlink row not rendered");
  return found;
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
    editingBlockId: null,
    editingBacklink: null,
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
    const row = () => backlinkRow(host).closest("[data-backlink-row]")!.className;

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
    // `selectedBlockId` is not the whole condition — see the describe
    // below — but it is the half this rule started as: the two handlers
    // that cross the separator null it in the same breath, so no other
    // writer has to know this rule exists.
    setAppState("selectedBacklinkBlockId", "src-1");
    expect(backlinkCursor()).toBeNull();

    setAppState("selectedBlockId", null);
    expect(backlinkCursor()).toBe("src-1");
  });
});

/**
 * A cursor cannot be in a section that is not on screen, and this is the
 * **agreement** half of that: the row's render condition and the two
 * accessors have to answer the same question, or the keyboard acts on a
 * row nobody drew.
 *
 * It is a behavioural net rather than a source read, because what matters
 * is that the three agree — not how each spells it. The rule used to live
 * in `cursor-handlers.ts`'s `backlinkBlockIds()` (which decided whether
 * `j` may *enter* the section) and nowhere else, so `backlinkCursor()`
 * said "the cursor is on a row" after `Cmd+Shift+B` had taken the row
 * away. Nine mutating chords then wrote to another page from there, and
 * `isEditing()` reported an editor that had been unmounted, deferring
 * every peer reload for the rest of the session.
 *
 * `ToggleBacklinks` is **`Global`** (`defaults.rs`), so the chord fires
 * from anywhere, and `togglePanel` flips the flag and nothing else — no
 * writer of either field is reached, and a DOM removal does not reliably
 * fire `blur`. So clearing the fields in the *writer* was never an
 * option; the question belongs in the accessor.
 */
describe("a cursor cannot be in a section that is not drawn", () => {
  /** Rows the section actually painted — the editor's own row included,
   *  which is why this counts `<li>`s rather than looking for the
   *  read-mode button's text. */
  const rowsDrawn = (host: HTMLElement) => host.querySelectorAll("li").length;

  it("the row, the cursor and the editor all go when the panel closes", () => {
    const host = mountPage();
    setAppState({
      selectedBlockId: null,
      selectedBacklinkBlockId: "src-1",
      editingBacklink: { pageId: "other-page", blockId: "src-1" },
    });
    // All three agree the row is there.
    expect(rowsDrawn(host)).toBe(1);
    expect(backlinkCursor()).toBe("src-1");
    expect(isEditing()).toBe(true);

    handlers().ToggleBacklinks?.();

    // …and all three agree it is gone. Before this, the first assertion
    // flipped and the other two did not: nine mutating chords then wrote
    // to another page from a row nobody could see, and every deferred
    // peer reload waited on an editor that had been unmounted.
    expect(rowsDrawn(host)).toBe(0);
    expect(backlinkCursor()).toBeNull();
    expect(isEditing()).toBe(false);
  });

  it("the same three answers while zoomed", () => {
    mountPage();
    setAppState({
      selectedBlockId: null,
      selectedBacklinkBlockId: "src-1",
      editingBacklink: { pageId: "other-page", blockId: "src-1" },
      namespaceChildren: [],
      namespaceMentions: [],
      namespaceMentionsTotal: 0,
    });

    // The render side has to be `<PageSections />`, not `<InlineBacklinks />`:
    // the zoom gate lives there, and it takes `zoomed` as a **prop** on
    // purpose, so the component cannot be asked whether the page is
    // zoomed. Mounting it with the prop derived from `zoomView()` is what
    // ties the three together — the same `zoomView()` that empties
    // `backlinkRowIds()` is the one `<OutlineView />` passes down.
    // Asserting the prop's own effect instead would only prove `<Show>`
    // works.
    const sections = document.createElement("div");
    document.body.appendChild(sections);
    const unmount = render(
      () => (
        <PageSections onOpenPage={() => {}} zoomed={zoomView() !== null} />
      ),
      sections,
    );
    // `<li>`s, not the row's text: `editingBacklink` is set here, so the
    // row mounts its editor and the text lives in a `<textarea>`'s value
    // rather than in `textContent`. The sibling sections are given empty
    // slices above so they contribute no rows of their own.
    const rowsDrawn = () => sections.querySelectorAll("li").length;

    try {
      expect(rowsDrawn()).toBe(1);
      expect(backlinkCursor()).toBe("src-1");
      expect(isEditing()).toBe(true);

      setAppState("focusBlockId", "blk-1");

      expect(rowsDrawn()).toBe(0);
      expect(backlinkCursor()).toBeNull();
      expect(isEditing()).toBe(false);
    } finally {
      unmount();
      sections.remove();
    }
  });
});
