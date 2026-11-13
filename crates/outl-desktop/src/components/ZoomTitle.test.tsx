/**
 * The zoom header's `<h1>` rendered the focused block's **raw source**,
 * so zooming into a block titled
 * `#1on1 [[@Roberta Guimarães]] | 17:00 · [GCal](https://…)` showed the
 * brackets, the tag hash and the whole URL — the one place in the app
 * where a block's text is not rendered.
 *
 * Mobile never had it (its zoom root is an ordinary `<BlockRow />`) and
 * neither did the TUI (`render_block` draws the zoom root), because
 * only the desktop promotes the focused block to a page-like title.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it, vi } from "vitest";

vi.mock("@outl/shared/api/commands", () => ({
  readAssetDataUrl: vi.fn().mockResolvedValue(""),
}));

import { render } from "solid-js/web";

import type { BlockNode, InlineToken } from "@outl/shared/api/types";
import { ZoomTitle } from "./ZoomTitle";

function block(text: string, tokens: InlineToken[]): BlockNode {
  return {
    id: "blk-1",
    text,
    todo: null,
    tokens,
    collapsed: false,
    properties: [],
    children: [],
  };
}

/** The block from the bug report, as the backend tokenizes it. */
const MEETING = block(
  "#1on1 [[@Roberta Guimarães]] | 17:00 · [GCal](https://cal.example/e/abc)",
  [
    { kind: "tag", value: "1on1" },
    { kind: "plain", value: " " },
    { kind: "ref", value: "@Roberta Guimarães" },
    { kind: "plain", value: " | 17:00 · " },
    { kind: "link", value: "GCal", href: "https://cal.example/e/abc" },
  ],
);

function mount(el: () => ReturnType<typeof ZoomTitle>) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(el, host);
  return {
    host,
    cleanup: () => {
      dispose();
      host.remove();
    },
  };
}

describe("ZoomTitle", () => {
  it("renders the block's markdown, never its raw source", () => {
    const { host, cleanup } = mount(() => (
      <ZoomTitle root={MEETING} onRefClick={vi.fn()} onTagClick={vi.fn()} />
    ));
    try {
      const text = host.textContent ?? "";
      expect(text).toContain("Roberta Guimarães");
      expect(text).toContain("GCal");
      expect(text).not.toContain("[[");
      expect(text).not.toContain("https://cal.example");
    } finally {
      cleanup();
    }
  });

  it("a ref in the title navigates, like the same ref in a row", () => {
    const onRefClick = vi.fn();
    const { host, cleanup } = mount(() => (
      <ZoomTitle root={MEETING} onRefClick={onRefClick} onTagClick={vi.fn()} />
    ));
    try {
      const ref = [...host.querySelectorAll("span")].find(
        (s) => s.textContent === "@Roberta Guimarães",
      );
      ref?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      expect(onRefClick).toHaveBeenCalledWith("@Roberta Guimarães");
    } finally {
      cleanup();
    }
  });

  it("falls back to the plain text when the backend sent no tokens", () => {
    const { host, cleanup } = mount(() => (
      <ZoomTitle
        root={block("plain words", [])}
        onRefClick={vi.fn()}
        onTagClick={vi.fn()}
      />
    ));
    try {
      expect(host.textContent).toBe("plain words");
    } finally {
      cleanup();
    }
  });

  it("names an empty block instead of rendering nothing", () => {
    const { host, cleanup } = mount(() => (
      <ZoomTitle root={block("", [])} onRefClick={vi.fn()} onTagClick={vi.fn()} />
    ));
    try {
      expect(host.textContent).toBe("(empty block)");
    } finally {
      cleanup();
    }
  });
});

/**
 * The tests above pass whether or not `<OutlineView />` calls this
 * component — which is exactly how the `<h1>` went unrendered in the
 * first place. This reads its JSX rather than mounting it: mounting
 * needs the whole Tauri command surface stubbed, and that would pin the
 * stubs, not the header. Same shape as mobile's
 * `BlockRow.recursion.test.ts`.
 */
describe("the zoom header uses it", () => {
  const source = readFileSync(
    join(import.meta.dirname, "OutlineView.tsx"),
    "utf8",
  );

  it("renders <ZoomTitle /> in the zoomed <h1>", () => {
    expect(source).toContain("<ZoomTitle");
  });

  it("never interpolates the focused block's raw text", () => {
    expect(source).not.toContain("{fv().root.text}");
  });
});
