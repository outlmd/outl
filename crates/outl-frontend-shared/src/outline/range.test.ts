import { describe, expect, it } from "vitest";

import type { BlockNode } from "../api/types";

import { visibleRangeSlice } from "./index";

/**
 * What a selection range *contains* — a different question from how the
 * outline is navigated, which is why it is a different file.
 *
 * This is the one owner for every range op on both GUI clients, so the
 * cases below are the contract those ops inherit rather than a sample of
 * one client's behaviour.
 */
function block(
  id: string,
  opts: { collapsed?: boolean; children?: BlockNode[] } = {},
): BlockNode {
  return {
    id,
    text: id,
    todo: null,
    tokens: [],
    collapsed: opts.collapsed ?? false,
    properties: [],
    children: opts.children ?? [],
  };
}

describe("visibleRangeSlice", () => {
  const flat = [block("a"), block("b"), block("c"), block("d")];

  it("returns the covered ids in visible order", () => {
    expect(visibleRangeSlice("b", "d", flat)).toEqual(["b", "c", "d"]);
  });

  it("gives the same range regardless of drag direction", () => {
    expect(visibleRangeSlice("d", "b", flat)).toEqual(["b", "c", "d"]);
  });

  it("is a single block when anchor and cursor meet", () => {
    expect(visibleRangeSlice("c", "c", flat)).toEqual(["c"]);
  });

  it("is null when an endpoint is unset", () => {
    expect(visibleRangeSlice(null, "b", flat)).toBeNull();
    expect(visibleRangeSlice("b", null, flat)).toBeNull();
  });

  /**
   * The guard three of the four original copies were missing. `indexOf`
   * returns -1 for a stale endpoint, and `slice(-1, hi + 1)` is not
   * empty — it is the last block of the page. Without this, a range op
   * fired with a stale anchor silently operated on one unrelated block.
   */
  it("is null when an endpoint left the outline, never the last block", () => {
    expect(visibleRangeSlice("gone", "b", flat)).toBeNull();
    expect(visibleRangeSlice("b", "gone", flat)).toBeNull();
  });

  it("skips blocks hidden inside a collapsed parent", () => {
    const tree = [
      block("a"),
      block("b", { collapsed: true, children: [block("b1")] }),
      block("c"),
    ];
    expect(visibleRangeSlice("a", "c", tree)).toEqual(["a", "b", "c"]);
  });
});
