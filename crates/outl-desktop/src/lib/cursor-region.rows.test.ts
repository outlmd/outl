/**
 * `backlinkRowIds()` names the rows the panel **draws**: citing blocks
 * and their descendants, by `BacklinkRow.key`. The cursor, the editor and
 * `isEditing()` all resolve through it, so a list of citing blocks only
 * would let a refetch re-seed an open draft on a child.
 */
import { beforeEach, describe, expect, it } from "vitest";

import type { Backlink, BlockNode } from "@outl/shared/api/types";

import { backlinkRowByKey, backlinkRowIds, backlinkRows } from "./cursor-region";
import { setAppState } from "./store";

function block(id: string, children: BlockNode[] = []): BlockNode {
  return {
    id,
    text: id,
    todo: null,
    tokens: [],
    collapsed: false,
    properties: [],
    children,
  };
}

function link(node: BlockNode): Backlink {
  return {
    block_id: node.id,
    todo: null,
    source_page: { id: "pg", slug: "src", title: "Src", kind: "page" },
    source_block: node,
    source_block_path: [0],
    ancestors: [],
  };
}

describe("backlinkRowIds", () => {
  beforeEach(() => {
    setAppState({
      backlinks: [link(block("a", [block("k1"), block("k2")])), link(block("b"))],
      backlinksOpen: true,
      focusBlockId: null,
    });
  });

  it("lists roots and descendants in reading order, by key", () => {
    expect(backlinkRowIds()).toEqual(["a", "a>k1", "a>k2", "b"]);
  });

  it("is one memoised list per fetch, not one per call", () => {
    expect(backlinkRowIds()).toBe(backlinkRowIds());
    expect(backlinkRows()).toBe(backlinkRows());
  });

  it("follows a refetch", () => {
    setAppState("backlinks", [link(block("b"))]);
    expect(backlinkRowIds()).toEqual(["b"]);
    expect(backlinkRowByKey("a>k1")).toBeUndefined();
  });

  it("is empty while the section is not on screen", () => {
    setAppState("backlinksOpen", false);
    expect(backlinkRowIds()).toEqual([]);
  });
});
