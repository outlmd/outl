/**
 * `openBacklinkContext`: the sequence behind a breadcrumb click and `g o`.
 *
 * Pinned: one paint after every await, the unfold only when a fold is in
 * the way (and with the citing block), the fallbacks, the two ways a jump
 * is overtaken, and that the selection survives `installPageChangeEffect`.
 */
import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Backlink, BlockNode, PageMeta, PageView } from "@outl/shared/api/types";

vi.mock("@outl/shared/api/commands", () => ({
  openRef: vi.fn(),
  unfoldAncestors: vi.fn(),
  runAutoRunBlocks: vi.fn(() => Promise.resolve({ ran: 0 })),
  resolveEmbeds: vi.fn(() => Promise.resolve([])),
  resolvePageEmbeds: vi.fn(() => Promise.resolve([])),
  listTemplates: vi.fn(() => Promise.resolve([])),
  pluginList: vi.fn(() => Promise.resolve([])),
}));
vi.mock("./keyboard-reach", () => ({ revealBlock: vi.fn() }));

import { openRef, unfoldAncestors } from "@outl/shared/api/commands";

import { openBacklinkContext } from "./backlink-jump";
import { revealBlock } from "./keyboard-reach";
import { installPageChangeEffect } from "./page-change";
import { appState, landedMarks, setAppState, setOutline } from "./store";

const SRC: PageMeta = { id: "src", slug: "2026-11-24", title: "2026-11-24", kind: "journal" };
const OPEN: PageMeta = { id: "open", slug: "topic", title: "topic", kind: "page" };

function b(id: string, children: BlockNode[] = [], collapsed = false): BlockNode {
  return { id, text: id, todo: null, tokens: [], collapsed, properties: [], children };
}

function view(page: PageMeta, outline: BlockNode[]): PageView {
  return { page, outline, backlinks: [], backlinks_order: "newest" } as PageView;
}

function link(over: Partial<Backlink> = {}): Backlink {
  return {
    block_id: "cite",
    todo: null,
    ancestors: [{ id: "parent", text: "parent", tokens: [] }],
    source_page: SRC,
    source_block: b("cite"),
    source_block_path: [],
    ...over,
  };
}

const tree = (collapsed = false) => [b("parent", [b("cite")], collapsed)];

beforeEach(() => {
  setAppState({ page: OPEN, selectedBlockId: null, lastError: null } as never);
  setOutline([b("first")]);
});
afterEach(() => {
  vi.clearAllMocks();
  setAppState({ page: null, selectedBlockId: null, lastError: null, focusBlockId: null } as never);
  setOutline([]);
});

describe("openBacklinkContext", () => {
  it("nothing folded: no unfold, one paint, the parent selected and lit", async () => {
    vi.mocked(openRef).mockResolvedValue(view(SRC, tree()));
    await openBacklinkContext(link());
    expect(unfoldAncestors).not.toHaveBeenCalled();
    expect(appState.page?.id).toBe("src");
    expect(appState.selectedBlockId).toBe("parent");
    expect(landedMarks.isSaved("parent")).toBe(true);
    expect(revealBlock).toHaveBeenCalledWith("parent", "center");
  });

  it("folded: unfolds from the citing block and paints the unfolded view", async () => {
    vi.mocked(openRef).mockResolvedValue(view(SRC, tree(true)));
    const unfolded = view(SRC, [b("parent", [b("cite")], false)]);
    vi.mocked(unfoldAncestors).mockResolvedValue(unfolded);
    await openBacklinkContext(link());
    expect(unfoldAncestors).toHaveBeenCalledWith("src", "cite");
    expect(appState.outline[0].collapsed).toBe(false);
    expect(appState.selectedBlockId).toBe("parent");
  });

  it("falls back to the citing block's parent when the crumb is gone", async () => {
    vi.mocked(openRef).mockResolvedValue(view(SRC, tree()));
    await openBacklinkContext(link({ ancestors: [{ id: "gone", text: "", tokens: [] }] }));
    expect(appState.selectedBlockId).toBe("parent");
  });

  it("everything gone: opens the page, says so, writes nothing", async () => {
    vi.mocked(openRef).mockResolvedValue(view(SRC, [b("other")]));
    await openBacklinkContext(link());
    expect(unfoldAncestors).not.toHaveBeenCalled();
    expect(appState.page?.id).toBe("src");
    expect(appState.lastError).toBe(
      "That block is no longer on 2026-11-24; opened the page instead.",
    );
  });

  it("openRef rejecting leaves the view alone and reports", async () => {
    vi.mocked(openRef).mockRejectedValue(new Error("boom"));
    await openBacklinkContext(link());
    expect(appState.page?.id).toBe("open");
    expect(appState.lastError).toBe("boom");
  });

  it("an unfold that rejects keeps the opened view and selects nothing hidden", async () => {
    vi.mocked(openRef).mockResolvedValue(view(SRC, tree(true)));
    vi.mocked(unfoldAncestors).mockRejectedValue(new Error("no unfold"));
    const mark = vi.spyOn(landedMarks, "mark");
    await openBacklinkContext(link());
    const marked = mark.mock.calls.length;
    mark.mockRestore();
    expect(appState.page?.id).toBe("src");
    expect(appState.selectedBlockId).toBeNull();
    expect(marked).toBe(0);
    expect(revealBlock).not.toHaveBeenCalled();
    expect(appState.lastError).toMatch(/no unfold.*folded on 2026-11-24/);
  });

  it("the unfold reply does not drop the opened page's ahead-of-log notice", async () => {
    const notice = { path: "/x/2026-11-24.md", lines: 2 };
    vi.mocked(openRef).mockResolvedValue({
      ...view(SRC, tree(true)),
      md_ahead_of_log: notice,
      md_ahead_of_log_checked: true,
    } as PageView);
    vi.mocked(unfoldAncestors).mockResolvedValue(view(SRC, tree(false)));
    await openBacklinkContext(link());
    expect(appState.mdAheadOfLog).toEqual(notice);
  });

  it("a live top-level citing block is selected, not reported as gone", async () => {
    vi.mocked(openRef).mockResolvedValue(view(SRC, [b("cite")]));
    await openBacklinkContext(link({ ancestors: [] }));
    expect(appState.selectedBlockId).toBe("cite");
    expect(appState.lastError).toBeNull();
  });

  it("two overlapping jumps: only the second paints", async () => {
    let release!: (v: PageView) => void;
    vi.mocked(openRef)
      .mockReturnValueOnce(new Promise<PageView>((r) => (release = r)))
      .mockResolvedValueOnce(view({ ...SRC, id: "src2", slug: "s2" }, tree()));
    const first = openBacklinkContext(link());
    await openBacklinkContext(link({ source_page: { ...SRC, id: "src2", slug: "s2" } }));
    release(view(SRC, tree()));
    await first;
    expect(appState.page?.id).toBe("src2");
  });

  it("the page changing by another path mid-jump paints nothing", async () => {
    let release!: (v: PageView) => void;
    vi.mocked(openRef).mockReturnValueOnce(new Promise<PageView>((r) => (release = r)));
    const jump = openBacklinkContext(link());
    setAppState("page", { ...OPEN, id: "elsewhere" });
    release(view(SRC, tree()));
    await jump;
    expect(appState.page?.id).toBe("elsewhere");
    expect(appState.selectedBlockId).toBeNull();
  });

  it("a page that cites itself lands on the parent of the citing block", async () => {
    setAppState("page", SRC);
    vi.mocked(openRef).mockResolvedValue(view(SRC, tree()));
    await openBacklinkContext(link());
    expect(appState.selectedBlockId).toBe("parent");
  });

  it("an orphan row has nowhere to go and says so", async () => {
    await openBacklinkContext(link({ source_page: null }));
    expect(openRef).not.toHaveBeenCalled();
    expect(appState.lastError).toMatch(/no page of its own/);
  });

  it("the selection survives installPageChangeEffect", async () => {
    vi.mocked(openRef).mockResolvedValue(view(SRC, [b("first"), ...tree()]));
    let dispose = () => {};
    createRoot((d) => {
      dispose = d;
      installPageChangeEffect();
    });
    try {
      await openBacklinkContext(link());
      expect(appState.selectedBlockId).toBe("parent");
    } finally {
      dispose();
    }
  });
});
