/**
 * The fold chevron on a "Linked from" row folds the citing block **on
 * the page it lives on**, so the fold syncs and shows on that page too.
 *
 * Pinned here: the click sends an explicit `"collapse"` / `"expand"`
 * (never a toggle) to the source page, the panel re-asks for its rows,
 * a folded block hides its children, and a row with no page to write to
 * offers nothing to click.
 */
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type {
  Backlink,
  BlockNode,
  PageBacklinks,
  PageView,
} from "@outl/shared/api/types";

vi.mock("@outl/shared/api/commands", () => ({
  backlinkRowOp: vi.fn(),
  createBacklinkSibling: vi.fn(),
  editBacklinkBlock: vi.fn(),
  editBlock: vi.fn(),
  listTemplates: vi.fn(() => Promise.resolve([])),
  openPageBySlug: vi.fn(),
  openRef: vi.fn(),
  pageBacklinks: vi.fn(),
  pluginList: vi.fn(() => Promise.resolve([])),
  resolveEmbeds: vi.fn(() => Promise.resolve([])),
  resolvePageEmbeds: vi.fn(() => Promise.resolve([])),
  runAutoRunBlocks: vi.fn(() => Promise.resolve({ ran: 0 })),
  runCodeBlock: vi.fn(),
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

import { backlinkRowOp, pageBacklinks } from "@outl/shared/api/commands";

import { InlineBacklinks } from "./InlineBacklinks";
import { appState, setAppState } from "../lib/store";

const OPEN_PAGE = { id: "open-page", slug: "topic", title: "topic", kind: "page" } as const;
const SOURCE_PAGE = { id: "journal-page", slug: "2026-11-26", title: "2026-11-26", kind: "journal" } as const;

function node(id: string, text: string, over: Partial<BlockNode> = {}): BlockNode {
  return {
    id,
    text,
    todo: null,
    tokens: [{ kind: "plain", value: text }],
    collapsed: false,
    properties: [],
    children: [],
    ...over,
  };
}

function link(n: BlockNode, sourcePage: Backlink["source_page"] = SOURCE_PAGE): Backlink {
  return {
    block_id: n.id,
    todo: null,
    ancestors: [],
    source_page: sourcePage,
    source_block: n,
    source_block_path: [],
  };
}

function reply(backlinks: Backlink[]): PageBacklinks {
  return {
    backlinks,
    backlinks_order: "newest",
    namespace_children: [],
    namespace_backlinks: [],
    namespace_backlinks_total: 0,
  };
}

const sourceView = (): PageView => ({
  page: SOURCE_PAGE,
  outline: [],
  backlinks: [],
  backlinks_order: "newest",
});

const parent = (collapsed: boolean) =>
  node("b-parent", "cites [[topic]]", {
    collapsed,
    children: [node("b-child", "a child")],
  });

let dispose: (() => void) | undefined;

function mount(backlinks: Backlink[]): HTMLElement {
  setAppState({
    page: OPEN_PAGE,
    backlinksOpen: true,
    backlinks,
    backlinksOrder: "newest",
    editingBacklink: null,
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <InlineBacklinks />, host);
  return host;
}

function chevronOf(host: HTMLElement, rowKey: string): HTMLButtonElement {
  const found = host.querySelector<HTMLButtonElement>(
    `[data-backlink-row="${rowKey}"] [data-fold-chevron]`,
  );
  if (!found) throw new Error(`row ${rowKey} drew no fold chevron`);
  return found;
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  vi.clearAllMocks();
  setAppState({
    page: null,
    backlinks: [],
    backlinksOpen: true,
    backlinksLoading: false,
    editingBacklink: null,
    lastError: null,
    selectedBlockId: null,
    selectedBacklinkBlockId: null,
  });
  document.body.innerHTML = "";
});

describe("the fold chevron on a backlink row", () => {
  it("collapses the block on its source page and re-asks for the rows", async () => {
    vi.mocked(backlinkRowOp).mockResolvedValue(sourceView());
    vi.mocked(pageBacklinks).mockResolvedValue(reply([link(parent(true))]));
    const host = mount([link(parent(false))]);

    const chevron = chevronOf(host, "b-parent");
    expect(chevron.getAttribute("aria-label")).toBe("Collapse");
    chevron.click();

    await vi.waitFor(() => {
      expect(backlinkRowOp).toHaveBeenCalledWith(
        "journal-page",
        "b-parent",
        "collapse",
      );
      expect(pageBacklinks).toHaveBeenCalledWith("topic");
    });
    // The reply describes the journal; the page on screen stays.
    expect(appState.page?.id).toBe("open-page");
  });

  it("expands a folded block, and hides its children until then", async () => {
    vi.mocked(backlinkRowOp).mockResolvedValue(sourceView());
    vi.mocked(pageBacklinks).mockResolvedValue(reply([link(parent(false))]));
    const host = mount([link(parent(true))]);

    // `flattenBacklinkRows` stops under a folded node: no child row.
    expect(host.querySelector('[data-backlink-row="b-parent>b-child"]')).toBeNull();
    const chevron = chevronOf(host, "b-parent");
    expect(chevron.getAttribute("aria-label")).toBe("Expand");
    expect(chevron.textContent).toContain("1");

    chevron.click();

    await vi.waitFor(() => {
      expect(backlinkRowOp).toHaveBeenCalledWith(
        "journal-page",
        "b-parent",
        "expand",
      );
    });
    await vi.waitFor(() => {
      expect(
        host.querySelector('[data-backlink-row="b-parent>b-child"]'),
      ).not.toBeNull();
    });
  });

  it("does not edit the row when the chevron is clicked", async () => {
    vi.mocked(backlinkRowOp).mockResolvedValue(sourceView());
    vi.mocked(pageBacklinks).mockResolvedValue(reply([link(parent(false))]));
    const host = mount([link(parent(false))]);

    chevronOf(host, "b-parent").click();

    await vi.waitFor(() => expect(backlinkRowOp).toHaveBeenCalled());
    expect(appState.editingBacklink).toBeNull();
  });

  it("offers nothing on a leaf", () => {
    const host = mount([link(node("b-leaf", "cites [[topic]]"))]);

    const chevron = chevronOf(host, "b-leaf");
    expect(chevron.disabled).toBe(true);
    expect(chevron.textContent).toBe("");
    chevron.click();
    expect(backlinkRowOp).not.toHaveBeenCalled();
  });

  it("moves a cursor the fold hid up to the row that was folded", async () => {
    vi.mocked(backlinkRowOp).mockResolvedValue(sourceView());
    vi.mocked(pageBacklinks).mockResolvedValue(reply([link(parent(true))]));
    const host = mount([link(parent(false))]);
    setAppState({ selectedBlockId: null, selectedBacklinkBlockId: "b-parent>b-child" });

    chevronOf(host, "b-parent").click();

    // Left on `b-parent>b-child`, the key names no drawn row: the
    // highlight vanishes and `j` / `k` / every row chord dead-end.
    await vi.waitFor(() => {
      expect(appState.selectedBacklinkBlockId).toBe("b-parent");
    });
    expect(
      host.querySelector('[data-backlink-row="b-parent"]')?.className,
    ).toContain("bg-(--color-outl-accent)/6");
  });

  it("leaves a cursor the fold did not hide where it was", async () => {
    const other = node("b-other", "also cites [[topic]]");
    vi.mocked(backlinkRowOp).mockResolvedValue(sourceView());
    vi.mocked(pageBacklinks).mockResolvedValue(
      reply([link(parent(true)), link(other)]),
    );
    const host = mount([link(parent(false)), link(other)]);
    setAppState({ selectedBlockId: null, selectedBacklinkBlockId: "b-other" });

    chevronOf(host, "b-parent").click();

    await vi.waitFor(() => expect(pageBacklinks).toHaveBeenCalled());
    await vi.waitFor(() => {
      expect(host.querySelector('[data-backlink-row="b-parent>b-child"]')).toBeNull();
    });
    expect(appState.selectedBacklinkBlockId).toBe("b-other");
  });

  it("is inert on an orphan row, which has no page to write the fold to", () => {
    const host = mount([link(parent(false), null)]);

    const chevron = chevronOf(host, "b-parent");
    expect(chevron.disabled).toBe(true);
    chevron.click();
    expect(backlinkRowOp).not.toHaveBeenCalled();
  });
});
