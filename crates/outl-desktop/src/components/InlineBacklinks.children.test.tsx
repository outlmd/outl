/**
 * The "Linked from" panel draws each citing block's **subtree**, and the
 * descendants are rows like any other: editable, navigable, and keyed by
 * `BacklinkRow.key` rather than block id (a block that cites the page and
 * sits under another citing block is drawn twice).
 */
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { Backlink, BlockNode } from "@outl/shared/api/types";

vi.mock("@outl/shared/api/commands", () => ({
  backlinkRowOp: vi.fn(),
  createBacklinkSibling: vi.fn(),
  editBacklinkBlock: vi.fn(),
  editBlock: vi.fn(),
  listTemplates: vi.fn(() => Promise.resolve([])),
  openRef: vi.fn(),
  pageBacklinks: vi.fn(),
  pluginList: vi.fn(() => Promise.resolve([])),
  resolveEmbeds: vi.fn(() => Promise.resolve([])),
  resolvePageEmbeds: vi.fn(() => Promise.resolve([])),
  searchBlocks: vi.fn(() => Promise.resolve([])),
  searchEmojis: vi.fn(() => Promise.resolve([])),
  searchPages: vi.fn(() => Promise.resolve([])),
  searchPersons: vi.fn(() => Promise.resolve([])),
  setBacklinksOrder: vi.fn(),
  toggleTodo: vi.fn(),
}));
vi.mock("@outl/shared/plugins/transformer-registry", () => ({
  runTransform: vi.fn(),
  transformerFor: vi.fn(() => null),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ readText: vi.fn() }));

import { backlinkRowOp, resolveEmbeds } from "@outl/shared/api/commands";

import { InlineBacklinks } from "./InlineBacklinks";
import { appState, setAppState } from "../lib/store";
import { isEditing } from "../lib/mode";

function block(id: string, children: BlockNode[] = []): BlockNode {
  return {
    id,
    text: `text of ${id}`,
    todo: null,
    tokens: [{ kind: "plain", value: `text of ${id}` }],
    collapsed: false,
    properties: [],
    children,
  };
}

function link(node: BlockNode): Backlink {
  return {
    block_id: node.id,
    todo: null,
    ancestors: [],
    source_page: { id: "pg", slug: "journal", title: "Journal", kind: "page" },
    source_block: node,
    source_block_path: [],
  };
}

const tree = (): Backlink[] => [
  link(block("a", [block("c1", [block("gc1")]), block("c2")])),
  link(block("b")),
];

let dispose: (() => void) | undefined;
function mount(backlinks: Backlink[]): HTMLElement {
  setAppState({
    page: { id: "tag", slug: "project", title: "project", kind: "page" },
    backlinksOpen: true,
    backlinks,
    backlinksLoading: false,
    editingBacklink: null,
    selectedBacklinkBlockId: null,
    selectedBlockId: null,
    focusBlockId: null,
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <InlineBacklinks />, host);
  return host;
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  vi.clearAllMocks();
  setAppState({
    page: null,
    backlinks: [],
    editingBacklink: null,
    selectedBacklinkBlockId: null,
  });
});

const textButton = (host: HTMLElement, id: string) =>
  Array.from(host.querySelectorAll("button")).find(
    (b) => b.textContent === `text of ${id}`,
  ) as HTMLButtonElement;

describe("the subtree under a citing block", () => {
  it("draws every descendant, with one li per citing block", () => {
    const host = mount(tree());

    for (const id of ["a", "c1", "gc1", "c2", "b"]) {
      expect(textButton(host, id)).toBeDefined();
    }
    expect(host.querySelectorAll("li")).toHaveLength(2);
    expect(host.querySelector("header")?.textContent).toContain("2 refs");
  });

  it("edits a child in place, and only that child", () => {
    const host = mount(tree());

    textButton(host, "c1").click();

    expect(host.querySelectorAll("textarea")).toHaveLength(1);
    expect(appState.editingBacklink).toEqual({
      pageId: "pg",
      blockId: "c1",
      key: "a>c1",
    });
    expect(appState.selectedBacklinkBlockId).toBe("a>c1");
  });

  it("toggling a child's bullet targets the child", async () => {
    vi.mocked(backlinkRowOp).mockResolvedValue({} as never);
    const host = mount(tree());
    const bullet = host
      .querySelector('[data-backlink-child="a>c2"] [data-todo]') as HTMLButtonElement;

    bullet.click();

    expect(backlinkRowOp).toHaveBeenCalledWith("pg", "c2", "toggle-todo");
  });

  it("a block cited and also nested opens one editor, in the copy clicked", () => {
    const host = mount([link(block("a", [block("x")])), link(block("x"))]);

    const copies = Array.from(host.querySelectorAll("button")).filter(
      (b) => b.textContent === "text of x",
    );
    expect(copies).toHaveLength(2);
    copies[0].click(); // the nested copy, drawn first

    expect(host.querySelectorAll("textarea")).toHaveLength(1);
    expect(appState.editingBacklink?.key).toBe("a>x");
    expect(
      host.querySelector('[data-backlink-child="a>x"] textarea'),
    ).not.toBeNull();
  });

  it("a background refetch leaves the draft of an open child alone", async () => {
    const host = mount(tree());
    textButton(host, "c1").click();
    const area = host.querySelector("textarea") as HTMLTextAreaElement;
    area.value = "typed, not committed";
    area.dispatchEvent(new Event("input", { bubbles: true }));
    expect(isEditing()).toBe(true);

    // A refetch replaces every object, and the child's text moved on disk.
    const fresh = tree();
    fresh[0].source_block.children[0].text = "peer rewrote c1";
    setAppState("backlinks", fresh);
    await Promise.resolve();

    const after = host.querySelector("textarea") as HTMLTextAreaElement;
    expect(after).toBe(area);
    expect(after.value).toBe("typed, not committed");
    expect(appState.editingBacklink?.key).toBe("a>c1");
  });

  // Reported from a real graph: a child that is only `!((blk-…))` showed
  // the raw handle, because the embed map was filled from the open page's
  // outline and these rows are other pages' blocks.
  it("resolves a block embed that sits in a child row", async () => {
    vi.mocked(resolveEmbeds).mockResolvedValueOnce({
      "blk-qs4fmz": {
        handle: "blk-qs4fmz",
        text: "the embedded source",
        page_slug: "elsewhere",
        status: null,
        children: [],
      },
    } as never);
    const embed: BlockNode = {
      ...block("e"),
      text: "!((blk-qs4fmz))",
      tokens: [{ kind: "embed", value: "blk-qs4fmz" }] as BlockNode["tokens"],
    };
    const host = mount([link(block("a", [embed]))]);
    expect(host.textContent).toContain("!blk-qs4fmz");

    await vi.waitFor(() =>
      expect(host.textContent).toContain("the embedded source"),
    );
    expect(host.textContent).not.toContain("!blk-qs4fmz");
  });
});
