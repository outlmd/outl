import { describe, expect, it } from "vitest";

import type { Backlink, BlockNode, PageMeta } from "../api/types";
import {
  backlinkCommitPlan,
  backlinkCreatedElsewhere,
  backlinkEditTarget,
  backlinkEditTargetFor,
  flattenBacklinkRows,
} from "./index";

function link(opts: {
  pageId: string | null;
  blockId?: string;
  title?: string;
}): Backlink {
  return {
    block_id: opts.blockId ?? "b1",
    todo: null,
    source_page:
      opts.pageId === null
        ? null
        : ({ id: opts.pageId, title: opts.title ?? "Journal" } as PageMeta),
    ancestors: [],
  } as unknown as Backlink;
}

describe("backlinkEditTarget", () => {
  it("pairs the source page id with the cited block id", () => {
    expect(backlinkEditTarget(link({ pageId: "p1", blockId: "b9" }))).toEqual({
      pageId: "p1",
      blockId: "b9",
    });
  });

  it("refuses an orphan backlink, so the row stays read-only", () => {
    expect(backlinkEditTarget(link({ pageId: null }))).toBeNull();
  });

  it("addresses a self-citing row to the source page by value, not by the open page", () => {
    // page-a cites itself: the row's source page IS the page on screen.
    // The target must still carry the source id explicitly; asserting on
    // the literal values keeps this red if it ever falls back to "the
    // open page", which the equality would hide.
    const target = backlinkEditTarget(
      link({ pageId: "page-a", blockId: "blk-self", title: "Page A" }),
    );

    expect(target).toEqual({ pageId: "page-a", blockId: "blk-self" });
  });
});

describe("backlinkCommitPlan", () => {
  it("does not apply a reply that describes another page", () => {
    expect(backlinkCommitPlan({ sourcePageId: "p1", openPageId: "p2" })).toEqual({
      applyView: false,
      refetchBacklinks: true,
    });
  });

  it("applies the reply when the source page IS the open page", () => {
    expect(backlinkCommitPlan({ sourcePageId: "p1", openPageId: "p1" })).toEqual({
      applyView: true,
      refetchBacklinks: true,
    });
  });

  it("still refetches when no page is open", () => {
    expect(backlinkCommitPlan({ sourcePageId: "p1", openPageId: undefined })).toEqual({
      applyView: false,
      refetchBacklinks: true,
    });
  });

  it("applies the reply of a self-citing row to the screen", () => {
    // The user is looking at the page that changed; dropping the reply
    // would make the edit appear to not happen until a refetch.
    const target = backlinkEditTarget(link({ pageId: "page-a", blockId: "blk-self" }));
    const plan = backlinkCommitPlan({
      sourcePageId: target!.pageId,
      openPageId: "page-a",
    });

    expect(plan.applyView).toBe(true);
    expect(plan.refetchBacklinks).toBe(true);
  });

  it("does not apply the reply of an ordinary backlink row over the open page", () => {
    const target = backlinkEditTarget(link({ pageId: "page-b", blockId: "blk-b" }));
    const plan = backlinkCommitPlan({
      sourcePageId: target!.pageId,
      openPageId: "page-a",
    });

    expect(plan.applyView).toBe(false);
    expect(plan.refetchBacklinks).toBe(true);
  });

  it("is silent on a self-citing row and speaks on any other, without a caller-side gate", () => {
    const src = { id: "page-a", title: "Src" } as PageMeta;

    expect(backlinkCreatedElsewhere(src, "below", "page-a")).toBeNull();
    expect(backlinkCreatedElsewhere(src, "above", "page-a")).toBeNull();
    expect(backlinkCreatedElsewhere(src, "below", "page-b")).toContain("on Src");
  });

  it("speaks when no page is open, because nothing on screen can hold the block", () => {
    const src = { id: "page-a", title: "Src" } as PageMeta;

    expect(backlinkCreatedElsewhere(src, "below", undefined)).toContain("on Src");
  });
});

/**
 * The sentence a client says after creating a block through a backlink
 * row. Shared because every client has to answer "where did my block
 * go", and a second phrasing is a second answer.
 */
describe("backlinkCreatedElsewhere", () => {
  const page = (title: string) => ({ id: "p1", title }) as PageMeta;

  it("names the page and the route back to it", () => {
    const text = backlinkCreatedElsewhere(page("Monday"), "below", "other");

    expect(text).toContain("after this reference");
    expect(text).toContain("Monday");
    expect(text).toContain("to type in it");
  });

  it("says before for a block created above", () => {
    const text = backlinkCreatedElsewhere(page("Monday"), "above", "other");

    expect(text).toContain("before this reference");
    expect(text).not.toContain("after this reference");
  });

  it("says why no row appears for it, which is the part a user cannot infer", () => {
    // The new block references nothing, so it is a backlink of no page.
    // Without this clause the message reads as a bug report.
    expect(backlinkCreatedElsewhere(page("Monday"), "below", "other")).toContain(
      "does not reference this page",
    );
  });

  it("names no client's own chrome", () => {
    // The desktop opens the page from a *group* header and the phone
    // from a *card* header. A shared sentence naming either is wrong on
    // one of them.
    const text = backlinkCreatedElsewhere(page("Monday"), "below", "other");

    expect(text).not.toMatch(/group header/i);
    expect(text).not.toMatch(/card header/i);
    expect(text).not.toMatch(/sidebar|panel|long press|tap|click/i);
  });

  it("keeps its wording inside a toast's budget", () => {
    // It lands in a toast on both clients, and the first draft ran ~220
    // characters of *wording*, which overflows a 24rem one. Nothing but
    // jsdom is behind the render tests, so the budget is the net.
    //
    // **The budget is on the wording, not on the sentence.** The title
    // appears twice and can be any length, so a fixed total measured
    // against a short fixture passes while a real long title blows it
    // at runtime with nothing to catch it — a 22-character title
    // already reached 171 of the 180 this first pinned. The fixed part
    // is what this file controls, so that is what is bounded; a long
    // title legitimately wraps, and both toasts render multiple lines.
    const title = "Engineering / Platform / Mobile / Release Readiness";
    const text = backlinkCreatedElsewhere(page(title), "below", "other");

    expect(text).toContain(title);
    // 127 today. The slack is deliberately thin: a rewrite that needs
    // more than three characters more should be a decision, not a
    // drift.
    expect(text!.length - 2 * title.length).toBeLessThanOrEqual(130);
  });

  it("names the page exactly twice, which is what the budget assumes", () => {
    // If a rewrite drops or adds a mention, the budget above silently
    // stops measuring the thing it claims to.
    const text = backlinkCreatedElsewhere(page("Monday"), "below", "other");

    expect(text!.split("Monday").length - 1).toBe(2);
  });
});

function node(id: string, children: BlockNode[] = []): BlockNode {
  return { id, children } as unknown as BlockNode;
}

function withBlock(l: Backlink, source_block: BlockNode): Backlink {
  return { ...l, source_block } as Backlink;
}

describe("backlinkEditTargetFor", () => {
  it("addresses a descendant on the same source page", () => {
    const l = withBlock(link({ pageId: "p1", blockId: "b1" }), node("b1", [node("c1")]));
    expect(backlinkEditTargetFor(l, l.source_block.children[0]!)).toEqual({
      pageId: "p1",
      blockId: "c1",
    });
  });

  it("leaves an orphan read-only, children included", () => {
    const l = withBlock(link({ pageId: null }), node("b1", [node("c1")]));
    expect(backlinkEditTargetFor(l, l.source_block.children[0]!)).toBeNull();
  });
});

describe("flattenBacklinkRows", () => {
  it("walks each citing block then its subtree, depth-first", () => {
    const a = withBlock(
      link({ pageId: "p1", blockId: "a" }),
      node("a", [node("a1", [node("a1x")]), node("a2")]),
    );
    const b = withBlock(link({ pageId: "p2", blockId: "b" }), node("b"));
    const rows = flattenBacklinkRows([a, b]);
    expect(rows.map((r) => [r.node.id, r.depth])).toEqual([
      ["a", 0],
      ["a1", 1],
      ["a1x", 2],
      ["a2", 1],
      ["b", 0],
    ]);
    expect(rows.every((r) => r.link === (r.node.id === "b" ? b : a))).toBe(true);
  });

  it("gives a block drawn twice two different keys", () => {
    // `x` cites the page and sits under `a`, which cites it too: it is a
    // row of its own and a child of `a`.
    const a = withBlock(link({ pageId: "p1", blockId: "a" }), node("a", [node("x")]));
    const x = withBlock(link({ pageId: "p1", blockId: "x" }), node("x"));
    const rows = flattenBacklinkRows([a, x]);
    expect(rows.filter((r) => r.node.id === "x")).toHaveLength(2);
    expect(new Set(rows.map((r) => r.key)).size).toBe(rows.length);
  });

  it("is empty for no links", () => {
    expect(flattenBacklinkRows([])).toEqual([]);
  });
});

describe("backlinkCreatedElsewhere and subtree rows", () => {
  const page = { id: "p1", title: "Journal" } as PageMeta;

  it("still says where a sibling of the citing block went", () => {
    expect(backlinkCreatedElsewhere(page, "below", "other")).toContain("Journal");
    expect(backlinkCreatedElsewhere(page, "below", "other", 0)).toContain("Journal");
  });

  it("says nothing for a sibling of a child: it lands inside the drawn subtree", () => {
    expect(backlinkCreatedElsewhere(page, "below", "other", 1)).toBeNull();
    expect(backlinkCreatedElsewhere(page, "above", "other", 4)).toBeNull();
  });
});
