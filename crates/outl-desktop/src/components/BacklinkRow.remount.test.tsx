/**
 * A backlinks refetch must not remount the body of a row whose block did
 * not change.
 *
 * Every refetch replaces every node object (toggling a TODO in the panel,
 * a Run, a fold, a peer reload). The row's body was keyed on that object,
 * so every row remounted every time: each image re-read its asset over
 * IPC and blinked through its loading chip, and each `call:` fence asked
 * `listTemplates` again, an O(pages) walk under the workspace lock, once
 * per fence (invariant 15). A body whose text changed still has to
 * remount, because `<BlockBody />` decides its shape at mount.
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
  openPageBySlug: vi.fn(),
  openRef: vi.fn(),
  pageBacklinks: vi.fn(),
  pluginList: vi.fn(() => Promise.resolve([])),
  resolveEmbeds: vi.fn(() => Promise.resolve([])),
  resolvePageEmbeds: vi.fn(() => Promise.resolve([])),
  runAutoRunBlocks: vi.fn(() => Promise.resolve({ ran: 0 })),
  runCodeBlock: vi.fn(),
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

import { listTemplates } from "@outl/shared/api/commands";

import { InlineBacklinks } from "./InlineBacklinks";
import { setAppState } from "../lib/store";

const OPEN_PAGE = { id: "tpl-page", slug: "template-x", title: "template-x", kind: "page" } as const;
const SOURCE_PAGE = { id: "journal-page", slug: "2026-11-26", title: "2026-11-26", kind: "journal" } as const;

const FENCE = "```call:x\na: 1\n```";

function node(text: string, over: Partial<BlockNode> = {}): BlockNode {
  return {
    id: "b-call",
    text,
    todo: null,
    tokens: [{ kind: "plain", value: text }],
    collapsed: false,
    properties: [],
    children: [],
    ...over,
  };
}

/** A fresh object every call, the way every `page_backlinks` reply is. */
function link(n: BlockNode): Backlink {
  return {
    block_id: n.id,
    todo: null,
    ancestors: [],
    source_page: SOURCE_PAGE,
    source_block: n,
    source_block_path: [],
  };
}

let dispose: (() => void) | undefined;

function mount(backlinks: Backlink[]): HTMLElement {
  setAppState({
    page: OPEN_PAGE,
    backlinksOpen: true,
    backlinks,
    backlinksOrder: "newest",
    editingBacklink: null,
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <InlineBacklinks />, host);
  return host;
}

function bodyOf(host: HTMLElement): Element {
  const found = host.querySelector('[data-backlink-row="b-call"] [data-backlink-body]');
  const first = found?.firstElementChild;
  if (!first) throw new Error("row b-call drew no body");
  return first;
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  vi.clearAllMocks();
  setAppState({
    page: null,
    backlinks: [],
    backlinksOpen: true,
    backlinksLoading: false,
    editingBacklink: null,
  });
  document.body.innerHTML = "";
});

describe("a backlinks refetch and the row's body", () => {
  it("keeps the mounted body when the block's text did not change", async () => {
    const host = mount([link(node(FENCE))]);
    await vi.waitFor(() => expect(listTemplates).toHaveBeenCalledTimes(1));
    const before = bodyOf(host);

    // Same block, new objects: what a refetch for an unrelated row hands back.
    setAppState("backlinks", [link(node(FENCE, { collapsed: false }))]);
    await Promise.resolve();

    expect(bodyOf(host)).toBe(before);
    expect(listTemplates).toHaveBeenCalledTimes(1);
  });

  it("remounts the body when the block's text changed", async () => {
    const host = mount([link(node(FENCE))]);
    const before = bodyOf(host);

    setAppState("backlinks", [link(node("now plain prose"))]);

    await vi.waitFor(() => expect(bodyOf(host)).not.toBe(before));
    expect(host.textContent).toContain("now plain prose");
  });

  it("still reads the live node for what the text does not decide", async () => {
    const host = mount([link(node("prose"))]);
    const before = bodyOf(host);

    setAppState("backlinks", [
      link(node("prose", { properties: [["status", "doing"]] })),
    ]);

    await vi.waitFor(() => expect(host.textContent).toContain("doing"));
    expect(bodyOf(host)).toBe(before);
  });
});
