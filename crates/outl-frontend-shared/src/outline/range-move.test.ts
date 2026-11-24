import { describe, expect, it } from "vitest";

import type { BlockNode } from "../api/types";
import { rangeMoveTargets } from "./range-move";

function b(id: string, children: BlockNode[] = []): BlockNode {
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

const flat = [b("a"), b("b"), b("c"), b("d")];
// p has two children and sits between two plain siblings.
const nested = [
  b("above"),
  b("p", [b("p1"), b("p2")]),
  b("below", [b("below1")]),
];

describe("rangeMoveTargets", () => {
  it("leaves b,c of a,b,c,d: top-down going up, bottom-up going down", () => {
    expect(rangeMoveTargets(["b", "c"], flat, "up")).toEqual(["b", "c"]);
    expect(rangeMoveTargets(["b", "c"], flat, "down")).toEqual(["c", "b"]);
  });

  it("a parent selected with its children is one unit", () => {
    const ids = ["p", "p1", "p2"];
    expect(rangeMoveTargets(ids, nested, "up")).toEqual(["p"]);
    expect(rangeMoveTargets(ids, nested, "down")).toEqual(["p"]);
  });

  it("a displaced sibling with children does not matter to the targets", () => {
    expect(rangeMoveTargets(["p"], nested, "down")).toEqual(["p"]);
  });

  it("a range at the edge moves nothing", () => {
    expect(rangeMoveTargets(["a", "b"], flat, "up")).toEqual([]);
    expect(rangeMoveTargets(["c", "d"], flat, "down")).toEqual([]);
    expect(rangeMoveTargets(["a", "b"], flat, "down")).toEqual(["b", "a"]);
  });

  it("roots with different parents move nothing", () => {
    expect(rangeMoveTargets(["p2", "below"], nested, "up")).toEqual([]);
    expect(rangeMoveTargets(["p1", "p2", "below"], nested, "down")).toEqual([]);
  });

  it("a range starting on a child and running past its parent's end moves nothing", () => {
    expect(rangeMoveTargets(["p2", "below", "below1"], nested, "up")).toEqual([]);
  });

  it("non-contiguous siblings move nothing", () => {
    expect(rangeMoveTargets(["a", "c"], flat, "up")).toEqual([]);
  });

  it("an empty or stale selection moves nothing", () => {
    expect(rangeMoveTargets([], flat, "up")).toEqual([]);
    expect(rangeMoveTargets(["ghost"], flat, "down")).toEqual([]);
  });

  it("children of the same parent move among themselves", () => {
    expect(rangeMoveTargets(["p2"], nested, "up")).toEqual(["p2"]);
    expect(rangeMoveTargets(["p1"], nested, "up")).toEqual([]);
  });
});
