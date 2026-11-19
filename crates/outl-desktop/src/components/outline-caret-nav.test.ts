import { describe, expect, it } from "vitest";

import type { BlockNode } from "@outl/shared/api/types";
import { createCaretNavOps, type CaretNavOpsDeps } from "./outline-caret-nav";

/**
 * The wiring, not the walk.
 *
 * `resolveCaretExit`'s own behaviour — the edges, collapsed subtrees,
 * a block a peer deleted, the TODO prefix — is `@outl/shared/caret-nav`'s
 * and is tested there. What only this client can get wrong is handing it
 * the wrong block list, or writing a landing to some of the store but
 * not all of it.
 */

function block(id: string, text: string): BlockNode {
  return {
    id,
    text,
    todo: null,
    tokens: [],
    collapsed: false,
    properties: [],
    children: [],
  } as BlockNode;
}

function harness(blocks: BlockNode[]) {
  const state = {
    caretIntent: null as number | null,
    selected: null as string | null,
    editing: null as string | null,
  };
  const deps: CaretNavOpsDeps = {
    navBlocks: () => blocks,
    setCaretIntent: (offset) => (state.caretIntent = offset),
    setSelectedBlockId: (id) => (state.selected = id),
    setEditingId: (id) => (state.editing = id),
  };
  return { state, ops: createCaretNavOps(deps) };
}

describe("createCaretNavOps", () => {
  it("resolves against the blocks the cursor may walk", () => {
    // `navBlocks` is `visibleRoots()` in the app — the zoom's rows when
    // zoomed. Handing it the whole page instead would let the caret walk
    // out of a zoom the user is reading.
    const { ops } = harness([block("a", "first"), block("b", "second")]);
    expect(
      ops.resolveCaretExit("b", { to: "previous", landing: "end" }),
    ).toEqual({ id: "a", caret: 5, text: "first" });
  });

  it("puts Insert, the selection and the caret on the target together", () => {
    const { state, ops } = harness([block("a", "first")]);
    ops.openCaretLanding({ id: "a", caret: 5, text: "first" });
    expect(state).toEqual({ caretIntent: 5, selected: "a", editing: "a" });
  });
});
