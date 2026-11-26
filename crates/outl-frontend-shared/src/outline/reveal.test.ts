import { describe, expect, it } from "vitest";

import type { BlockNode, PageView } from "../api/types";
import { keepOpenedNotices, landingTarget, needsUnfold } from "./reveal";

function block(
  id: string,
  children: BlockNode[] = [],
  collapsed = false,
): BlockNode {
  return {
    id,
    text: id,
    todo: null,
    tokens: [],
    collapsed,
    children,
  } as unknown as BlockNode;
}

// a > b > c > citing ; d (top level)
const outline = (collapsed: string[] = []): BlockNode[] => [
  block(
    "a",
    [block("b", [block("c", [block("citing")], collapsed.includes("c"))], collapsed.includes("b"))],
    collapsed.includes("a"),
  ),
  block("d"),
];

describe("landingTarget", () => {
  it("prefers the crumb's block, unfolding from the citing block", () => {
    expect(landingTarget(outline(), "c", "citing")).toEqual({
      target: "c",
      unfoldFrom: "citing",
    });
  });

  it("falls back to the citing block's parent when the crumb is gone", () => {
    expect(landingTarget(outline(), "gone", "citing")).toEqual({
      target: "c",
      unfoldFrom: "citing",
    });
    expect(landingTarget(outline(), null, "citing")?.target).toBe("c");
  });

  it("unfolds from the target when the citing block is gone", () => {
    expect(landingTarget(outline(), "b", "gone")).toEqual({
      target: "b",
      unfoldFrom: "b",
    });
  });

  it("is null when nothing resolves", () => {
    expect(landingTarget(outline(), "gone", "gone")).toBeNull();
    expect(landingTarget([], null, "x")).toBeNull();
  });
});

describe("a top-level citing block", () => {
  it("lands on itself: it is on the page, just parentless", () => {
    expect(landingTarget(outline(), "gone", "d")).toEqual({
      target: "d",
      unfoldFrom: "d",
    });
    expect(needsUnfold(outline(), { target: "d", unfoldFrom: "d" })).toBe(false);
  });
});

describe("keepOpenedNotices", () => {
  it("paints the unfold outline with the opened view's ahead-of-log notice", () => {
    const notice = { path: "p", lines: 3 } as unknown as PageView["md_ahead_of_log"];
    const opened = { md_ahead_of_log: notice, md_ahead_of_log_checked: true } as PageView;
    const unfolded = { outline: [block("x")] } as unknown as PageView;
    const out = keepOpenedNotices(opened, unfolded);
    expect(out.outline).toBe(unfolded.outline);
    expect(out.md_ahead_of_log).toBe(notice);
    expect(out.md_ahead_of_log_checked).toBe(true);
  });
});

describe("needsUnfold", () => {
  const landing = { target: "c", unfoldFrom: "citing" };

  it("is false when nothing on the path is folded", () => {
    expect(needsUnfold(outline(), landing)).toBe(false);
  });

  it("is true when an ancestor is folded, including the target itself", () => {
    expect(needsUnfold(outline(["a"]), landing)).toBe(true);
    expect(needsUnfold(outline(["c"]), landing)).toBe(true);
  });

  it("ignores the unfoldFrom block's own fold and unrelated folds", () => {
    expect(needsUnfold(outline(), { target: "c", unfoldFrom: "c" })).toBe(false);
    const o = outline();
    o[1].collapsed = true;
    expect(needsUnfold(o, landing)).toBe(false);
  });

  it("is false when unfoldFrom is not in the outline", () => {
    expect(needsUnfold(outline(["a"]), { target: "x", unfoldFrom: "x" })).toBe(false);
  });
});
