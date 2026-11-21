/**
 * What one render of the backlinks section costs **per row**.
 *
 * Its own file rather than another describe in `InlineBacklinks.test.tsx`
 * — a different question (that file asks what the section *shows*), and
 * the partial mock below has to stay scoped to the question that needs
 * it.
 *
 * The shape being refused: an O(page) call inside a per-row predicate.
 * `backlinkCursor()` is evaluated once per rendered row (the row's own
 * highlight) and asks whether the section is on screen, which includes
 * `zoomView()` — and that is `focusSubtree(outline, focusBlockId)`, a
 * full page walk, whenever `focusBlockId` is set but names a block the
 * outline has lost. The self-heal that clears it is a `createEffect`,
 * so it runs **after** the render that pays for it.
 *
 * Measured on the busiest real page (1,556 rows, a 688-block outline):
 * **851 ms against 166 ms** to mount in that state, and **724.7 ms
 * against 2.6 ms** for a single cursor move, which is a keystroke.
 *
 * **Structural, not a timing assertion**, for the reason
 * `docs/performance.md` gives: a stopwatch test is flaky and says
 * nothing about why. The question a count can answer is the one that
 * matters — does the work grow with the number of rows.
 */
import { render } from "solid-js/web";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Backlink, BlockNode } from "@outl/shared/api/types";

vi.mock("@outl/shared/outline", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@outl/shared/outline")>();
  // The real walk, counted. Replacing it with a stub would mock away
  // exactly the thing under test.
  return { ...actual, focusSubtree: vi.fn(actual.focusSubtree) };
});
vi.mock("@outl/shared/api/commands", () => ({
  backlinkRowOp: vi.fn(),
  createBacklinkSibling: vi.fn(),
  editBacklinkBlock: vi.fn(),
  editBlock: vi.fn(),
  listTemplates: vi.fn(() => Promise.resolve([])),
  openRef: vi.fn(),
  pageBacklinks: vi.fn(() => Promise.resolve({})),
  pluginList: vi.fn(() => Promise.resolve([])),
  resolveEmbeds: vi.fn(() => Promise.resolve([])),
  resolvePageEmbeds: vi.fn(() => Promise.resolve([])),
  searchBlocks: vi.fn(() => Promise.resolve([])),
  searchEmojis: vi.fn(() => Promise.resolve([])),
  searchPages: vi.fn(() => Promise.resolve([])),
  searchPersons: vi.fn(() => Promise.resolve([])),
  setBacklinksOrder: vi.fn(),
  toggleTodo: vi.fn(),
}));
vi.mock("@outl/shared/plugins/transformer-registry", () => ({
  runTransform: vi.fn(),
  transformerFor: vi.fn(() => null),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ readText: vi.fn() }));

import { focusSubtree } from "@outl/shared/outline";

import { InlineBacklinks } from "./InlineBacklinks";
import { setAppState } from "../lib/store";

function backlink(i: number): Backlink {
  const id = `blk-${i}`;
  return {
    block_id: id,
    todo: null,
    ancestors: [],
    source_page: { id: "page-1", slug: "journal", title: "Journal", kind: "page" },
    source_block: {
      id,
      text: `task ${i}`,
      todo: null,
      tokens: [{ kind: "plain", value: `task ${i}` }],
      collapsed: false,
      properties: [],
      children: [],
    },
    source_block_path: [],
  } as unknown as Backlink;
}

/** A page with enough blocks that a walk of it would be visible in a
 *  count — the shape `focusSubtree` would traverse per row. */
function outline(roots: number): BlockNode[] {
  return Array.from({ length: roots }, (_, r) => ({
    id: `out-${r}`,
    text: `root ${r}`,
    todo: null,
    tokens: [],
    collapsed: false,
    properties: [],
    children: [],
  })) as unknown as BlockNode[];
}

/** Mount the section with a zoom target the outline has lost, which is
 *  the state that makes `zoomView()` do real work.
 *
 *  **Disposal is not hygiene here, it is the measurement.** A section
 *  left mounted keeps its memo subscribed, so the next test's
 *  `setAppState` re-evaluates it too and every count is one too high
 *  per leaked mount — which is how this file first read 2 walks for 400
 *  rows and 3 for a cursor move. */
function mountWithStaleZoom(rows: number): {
  move: () => void;
  dispose: () => void;
} {
  setAppState({
    page: { id: "tag-page", slug: "project", title: "project", kind: "page" },
    backlinksOpen: true,
    backlinksLoading: false,
    backlinks: Array.from({ length: rows }, (_, i) => backlink(i)),
    backlinksOrder: "newest",
    editingBacklink: null,
    editingBlockId: null,
    selectedBlockId: null,
    selectedBacklinkBlockId: null,
    outline: outline(60),
    // A peer deleted or moved the zoomed block: `zoomView()` answers
    // null, so `<PageSections zoomed>` is false and this section
    // renders with `focusBlockId` still set.
    focusBlockId: "gone-blk",
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <InlineBacklinks />, host);
  expect(host.querySelectorAll("li")).toHaveLength(rows);
  return {
    move: () => setAppState("selectedBacklinkBlockId", `blk-${rows - 1}`),
    dispose: () => {
      dispose();
      host.remove();
    },
  };
}

beforeEach(() => {
  vi.mocked(focusSubtree).mockClear();
});

describe("the row highlight does not walk the page once per row", () => {
  it("costs the same number of page walks at 40 rows as at 400", () => {
    const small = mountWithStaleZoom(40);
    const atForty = vi.mocked(focusSubtree).mock.calls.length;
    small.dispose();

    vi.mocked(focusSubtree).mockClear();
    const big = mountWithStaleZoom(400);
    const atFourHundred = vi.mocked(focusSubtree).mock.calls.length;
    big.dispose();

    // Row-count independence is the whole assertion: ten times the rows
    // for the same number of walks. Per row it would be 40 against 400,
    // which is the O(N²) the memo exists to remove.
    expect({ atForty, atFourHundred }).toEqual({ atForty: 1, atFourHundred: 1 });
  });

  it("costs the same number of page walks per cursor move as at mount", () => {
    const { move, dispose } = mountWithStaleZoom(400);
    vi.mocked(focusSubtree).mockClear();

    move();
    dispose();

    // A cursor move is a keystroke, and it re-evaluates every row's
    // highlight. Measured at 724.7 ms per `j` before the memo, on 1,556
    // rows over a 688-block page.
    expect(vi.mocked(focusSubtree).mock.calls.length).toBe(1);
  });
});
