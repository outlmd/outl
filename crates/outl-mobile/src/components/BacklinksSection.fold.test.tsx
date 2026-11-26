/**
 * A "Linked from" row carries the outline row's disclosure triangle, and
 * a tap folds the citing block **on its own page** through the same
 * `ops` object the checkbox uses, so the one-write-at-a-time guard
 * covers it too.
 */
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { Backlink, BlockNode, PageView } from "@outl/shared/api/types";

import { BacklinksSection } from "./BacklinksSection";

const commands = vi.hoisted(() => ({
  backlinkRowOp: vi.fn(),
  createBacklinkSibling: vi.fn(),
  editBacklinkBlock: vi.fn(),
  resolveEmbeds: vi.fn(() => Promise.resolve({})),
  resolvePageEmbeds: vi.fn(() => Promise.resolve({})),
}));
vi.mock("@outl/shared/api/commands", () => commands);
vi.mock("@outl/shared/plugins/transformer-registry", () => ({
  transformerFor: vi.fn(() => null),
  runTransform: vi.fn(),
}));
vi.mock("../lib/haptics", () => ({ haptic: vi.fn() }));

const SOURCE = { id: "p-src", slug: "journal", title: "Journal", kind: "journal" } as const;

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  vi.clearAllMocks();
  document.body.innerHTML = "";
});

function node(id: string, over: Partial<BlockNode> = {}): BlockNode {
  return {
    id,
    text: id,
    todo: null,
    tokens: [{ kind: "plain", value: id }],
    collapsed: false,
    properties: [],
    children: [],
    ...over,
  };
}

function backlink(n: BlockNode, sourcePage: Backlink["source_page"] = SOURCE): Backlink {
  return {
    block_id: n.id,
    todo: null,
    ancestors: [],
    source_page: sourcePage,
    source_block: n,
    source_block_path: [],
  };
}

const parent = (collapsed: boolean) =>
  node("b-parent", { collapsed, children: [node("b-child")] });

function mount(backlinks: Backlink[], onEditCommitted = vi.fn()): HTMLElement {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => (
      <BacklinksSection
        backlinks={backlinks}
        onJump={() => {}}
        order="newest"
        onToggleOrder={() => {}}
        onEditCommitted={onEditCommitted}
        onError={() => {}}
      />
    ),
    host,
  );
  return host;
}

const triangle = (host: HTMLElement, label: string) =>
  host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);

describe("the disclosure triangle on a backlink row", () => {
  it("collapses the block on its source page", async () => {
    const view: PageView = { page: SOURCE, outline: [], backlinks: [], backlinks_order: "newest" };
    commands.backlinkRowOp.mockResolvedValue(view);
    const committed = vi.fn();
    const host = mount([backlink(parent(false))], committed);

    triangle(host, "Collapse block")!.click();

    await vi.waitFor(() => {
      expect(commands.backlinkRowOp).toHaveBeenCalledWith(
        "p-src",
        "b-parent",
        "collapse",
      );
      // The host's re-ask: the fold changes which rows this card draws.
      expect(committed).toHaveBeenCalledWith({
        sourcePageId: "p-src",
        view,
        created: undefined,
      });
    });
  });

  it("expands a folded block, whose children are hidden until then", async () => {
    commands.backlinkRowOp.mockResolvedValue({
      page: SOURCE,
      outline: [],
      backlinks: [],
      backlinks_order: "newest",
    });
    const host = mount([backlink(parent(true))]);

    expect(host.querySelector('[data-backlink-key="b-parent>b-child"]')).toBeNull();
    triangle(host, "Expand block")!.click();

    await vi.waitFor(() => {
      expect(commands.backlinkRowOp).toHaveBeenCalledWith(
        "p-src",
        "b-parent",
        "expand",
      );
    });
  });

  it("does not open the row's editor", async () => {
    commands.backlinkRowOp.mockResolvedValue({
      page: SOURCE,
      outline: [],
      backlinks: [],
      backlinks_order: "newest",
    });
    const host = mount([backlink(parent(false))]);

    triangle(host, "Collapse block")!.click();

    await vi.waitFor(() => expect(commands.backlinkRowOp).toHaveBeenCalled());
    expect(host.querySelector("textarea")).toBeNull();
  });

  it("is absent on a leaf and on an orphan", () => {
    const host = mount([
      backlink(node("b-leaf")),
      backlink(parent(false), null),
    ]);

    expect(triangle(host, "Collapse block")).toBeNull();
    expect(triangle(host, "Expand block")).toBeNull();
  });
});
