import { render } from "solid-js/web";
import { describe, expect, it } from "vitest";

import type { BlockNode, InlineToken } from "../api/types";
import { EmbeddedSubtree } from "./EmbeddedSubtree";

function node(
  id: string,
  text: string,
  opts: { todo?: "TODO" | "DONE" | null; children?: BlockNode[] } = {},
): BlockNode {
  const tokens: InlineToken[] = [{ kind: "plain", value: text }];
  return {
    id,
    text,
    todo: opts.todo ?? null,
    tokens,
    collapsed: false,
    properties: [],
    children: opts.children ?? [],
  };
}

function mount(nodes: BlockNode[], depth?: number) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(
    () => <EmbeddedSubtree nodes={nodes} depth={depth} />,
    host,
  );
  return {
    host,
    dispose: () => {
      dispose();
      host.remove();
    },
  };
}

describe("EmbeddedSubtree", () => {
  // These pin the *decisions*, not the glyph. The suite they replaced
  // asserted a `↳` on every row, so swapping that arrow for anything
  // else failed four tests while changing nothing a reader cares
  // about — and the one property that actually matters (the region is
  // marked once, as a region) was pinned by nothing.

  it("renders every row's text", () => {
    const { host, dispose } = mount([node("a", "first"), node("b", "second")]);
    expect(host.textContent).toContain("first");
    expect(host.textContent).toContain("second");
    dispose();
  });

  it("renders nested children recursively", () => {
    const { host, dispose } = mount([
      node("p", "parent", { children: [node("c", "child")] }),
    ]);
    expect(host.textContent).toContain("parent");
    expect(host.textContent).toContain("child");
    dispose();
  });

  it("prefixes the task marker", () => {
    const { host, dispose } = mount([
      node("t", "todo item", { todo: "TODO" }),
      node("d", "done item", { todo: "DONE" }),
    ]);
    expect(host.textContent).toContain("☐ todo item");
    expect(host.textContent).toContain("✓ done item");
    dispose();
  });

  it("stops recursing at depth 4, which is the whole cycle guard", () => {
    // A page that embeds itself has no visited-set to stop it; the cap
    // is the guard. One of three copies (TUI, backend, here).
    let deep = node("leaf", "deep-leaf");
    for (let i = 5; i >= 1; i--) deep = node(`d${i}`, `d${i}`, { children: [deep] });
    const { host, dispose } = mount([deep]);
    expect(host.textContent).toContain("d1");
    expect(host.textContent).not.toContain("deep-leaf");
    dispose();
  });

  it("draws the source rail once, around the whole region", () => {
    // The fact a reader cannot infer is "this came from another page",
    // and it is true of the region, not of any one row. One rail says
    // it once; the thirteen arrows this replaced said "nested" thirteen
    // times and never said this at all.
    const { host, dispose } = mount([
      node("p", "parent", { children: [node("c", "child")] }),
    ]);
    expect(host.querySelectorAll("[class*='border-l-']").length).toBe(1);
    dispose();
  });

  it("does not redraw the rail on a nested call", () => {
    // A nested render is already inside the rail. A second line per
    // level would redraw the page's own indent guides in a colour that
    // means something else.
    const { host, dispose } = mount([node("c", "child")], 2);
    expect(host.querySelectorAll("[class*='border-l-']").length).toBe(0);
    dispose();
  });

  it("steps each level by the outline's own 22px", () => {
    // `DESIGN.md` → "Outline indentation — 22px per level". This used
    // to step at 16px (`1rem`), so transcluded rows drifted out of
    // phase with the page around them the deeper they went.
    const { host, dispose } = mount([
      node("p", "parent", { children: [node("c", "child")] }),
    ]);
    expect(host.innerHTML).toContain("padding-left: 22px");
    dispose();
  });
});
