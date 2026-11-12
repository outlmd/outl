import { createRoot } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";

import type { BlockNode, PageMeta } from "@outl/shared/api/types";

import { installPageChangeEffect } from "./page-change";
import { appState, setAppState, setOutline } from "./store";

function block(id: string, children: BlockNode[] = []): BlockNode {
  return {
    id,
    text: `text ${id}`,
    todo: null,
    tokens: [],
    collapsed: false,
    properties: [],
    children,
  };
}

function page(id: string): PageMeta {
  return { id, slug: `slug-${id}` } as unknown as PageMeta;
}

describe("installPageChangeEffect", () => {
  let dispose: () => void = () => {};
  afterEach(() => {
    dispose();
    setAppState({ page: null, focusBlockId: null, selectedBlockId: null } as never);
    setOutline([]);
  });

  function mount() {
    setAppState("page", page("p1"));
    setOutline([block("a", [block("a1")]), block("b")]);
    setAppState("selectedBlockId", null);
    setAppState("focusBlockId", null);
    createRoot((d) => {
      dispose = d;
      installPageChangeEffect();
    });
  }

  it("selects the first block on initial load", () => {
    mount();
    expect(appState.selectedBlockId).toBe("a");
  });

  it("keeps the zoom across a same-id page replace with a new outline", () => {
    mount();
    setAppState("focusBlockId", "a");
    setAppState("page", page("p1"));
    setOutline([block("a", [block("a1")]), block("b"), block("c")]);
    expect(appState.focusBlockId).toBe("a");
  });

  it("keeps the zoom when the selection changes", () => {
    mount();
    setAppState("focusBlockId", "a");
    setAppState("selectedBlockId", "a1");
    setAppState("selectedBlockId", null);
    expect(appState.focusBlockId).toBe("a");
  });

  it("clears the zoom and selects the first block on another page", () => {
    mount();
    setAppState("focusBlockId", "a");
    setOutline([block("x"), block("y")]);
    setAppState("page", page("p2"));
    expect(appState.focusBlockId).toBeNull();
    expect(appState.selectedBlockId).toBe("x");
  });
});
