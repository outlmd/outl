/**
 * Pure state transitions for mobile's touch-native block selection
 * (RFC 0254 phase 3). No DOM, no Solid, no gestures — those live in
 * `Journal.tsx` / `BlockRow.tsx` and are exercised through the app;
 * this is the part that is worth pinning directly because a stateful
 * gesture is the hardest kind to test and the easiest to regress.
 */

import { describe, expect, it } from "vitest";

import type { BlockNode } from "@outl/shared/api/types";
import {
  extendSelectionTo,
  growSelectionDown,
  growSelectionUp,
  selectionIsLive,
  startSelection,
} from "./block-selection";

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

// a
//   b
//   c
// d
// e (collapsed)
//   f (hidden — under a collapsed parent)
const OUTLINE: BlockNode[] = [
  block("a", { children: [block("b"), block("c")] }),
  block("d"),
  block("e", { collapsed: true, children: [block("f")] }),
];

describe("startSelection", () => {
  it("anchors and cursors on the same block — a single-block range", () => {
    expect(startSelection("a")).toEqual({ anchorId: "a", cursorId: "a" });
  });
});

describe("extendSelectionTo", () => {
  it("moves the cursor, keeps the anchor fixed", () => {
    const sel = startSelection("b");
    expect(extendSelectionTo(sel, "d")).toEqual({
      anchorId: "b",
      cursorId: "d",
    });
  });

  it("extends upward just as well as downward — the anchor doesn't care which side the tap landed on", () => {
    const sel = startSelection("d");
    expect(extendSelectionTo(sel, "a")).toEqual({
      anchorId: "d",
      cursorId: "a",
    });
  });

  it("tapping the anchor itself collapses back to a single-block range", () => {
    const sel = extendSelectionTo(startSelection("a"), "d");
    expect(extendSelectionTo(sel, "a")).toEqual({
      anchorId: "a",
      cursorId: "a",
    });
  });
});

describe("growSelectionDown", () => {
  it("moves the cursor to the next visible block, anchor fixed", () => {
    const sel = startSelection("a");
    expect(growSelectionDown(sel, OUTLINE)).toEqual({
      anchorId: "a",
      cursorId: "b",
    });
  });

  it("skips a collapsed subtree, matching flattenVisible", () => {
    const sel = startSelection("d");
    // next visible after "d" is "e" (not "f" — "f" is hidden under
    // e's collapsed flag), same rule flattenVisible enforces
    // everywhere else in the app.
    expect(growSelectionDown(sel, OUTLINE).cursorId).toBe("e");
  });

  it("clamps at the bottom instead of falling off the outline", () => {
    const sel = startSelection("e");
    expect(growSelectionDown(sel, OUTLINE)).toEqual({
      anchorId: "e",
      cursorId: "e",
    });
  });
});

describe("growSelectionUp", () => {
  it("moves the cursor to the previous visible block, anchor fixed", () => {
    // Flat visible order is [a, b, c, d, e] — "c" is "d"'s immediate
    // predecessor, not "a".
    const sel = startSelection("d");
    expect(growSelectionUp(sel, OUTLINE)).toEqual({
      anchorId: "d",
      cursorId: "c",
    });
  });

  it("clamps at the top instead of returning null", () => {
    const sel = startSelection("a");
    // previousVisibleId(null-at-top) must not poison the cursor with
    // `null` — the selection has to stay a valid BlockSelection.
    expect(growSelectionUp(sel, OUTLINE)).toEqual({
      anchorId: "a",
      cursorId: "a",
    });
  });
});

describe("selectionIsLive", () => {
  it("is true while both endpoints are still visible", () => {
    expect(selectionIsLive(extendSelectionTo(startSelection("a"), "d"), OUTLINE)).toBe(
      true,
    );
  });

  it("is false once an endpoint left the outline (peer deleted it, or it's now under a fold)", () => {
    const sel = extendSelectionTo(startSelection("a"), "gone");
    expect(selectionIsLive(sel, OUTLINE)).toBe(false);
  });

  it("is false when the cursor is hidden under a collapsed parent", () => {
    // "f" exists in the tree but is not in flattenVisible while "e"
    // stays collapsed — a stale `lastSelection` from before the fold
    // must not resurrect as reachable.
    const sel = extendSelectionTo(startSelection("d"), "f");
    expect(selectionIsLive(sel, OUTLINE)).toBe(false);
  });
});
