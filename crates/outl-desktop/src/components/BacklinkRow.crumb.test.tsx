/**
 * A backlink's breadcrumb line is a link to the page the citing block
 * lives on: pointer, role, tab stop, click, Enter that stops at
 * the row, and `[[ref]]` / `#tag` that navigate on their own. An orphan
 * row stays inert text.
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
  openRef: vi.fn(() => Promise.reject(new Error("stop"))),
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
const jump = vi.hoisted(() => ({ openBacklinkContext: vi.fn() }));
vi.mock("../lib/backlink-jump", () => jump);
const nav = vi.hoisted(() => ({
  handleRefClick: vi.fn(),
  handleTagClick: vi.fn(),
  handleLinkClick: vi.fn(),
  handleOpenPage: vi.fn(),
  applyView: vi.fn(),
}));
vi.mock("../lib/outline-navigation", () => nav);

import { InlineBacklinks } from "./InlineBacklinks";
import { appState, setAppState } from "../lib/store";

const OPEN = { id: "open", slug: "topic", title: "topic", kind: "page" } as const;
const SRC = { id: "src", slug: "2026-11-24", title: "2026-11-24", kind: "journal" } as const;

const cited: BlockNode = {
  id: "cite",
  text: "cites [[topic]]",
  todo: null,
  tokens: [{ kind: "plain", value: "cites" }],
  collapsed: false,
  properties: [],
  children: [],
};

function link(source: Backlink["source_page"] = SRC): Backlink {
  return {
    block_id: "cite",
    todo: null,
    ancestors: [
      {
        id: "parent",
        text: "#whatsapp [[buser/vendas]]",
        tokens: [
          { kind: "tag", value: "#whatsapp" },
          { kind: "plain", value: " " },
          { kind: "ref", value: "buser/vendas" },
        ],
      },
    ],
    source_page: source,
    source_block: cited,
    source_block_path: [],
  };
}

let dispose: (() => void) | undefined;
function mount(l: Backlink): HTMLElement {
  setAppState({ page: OPEN, backlinksOpen: true, backlinks: [l], backlinksOrder: "newest" } as never);
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <InlineBacklinks />, host);
  return host;
}
const crumb = (h: HTMLElement) => h.querySelector<HTMLElement>("[data-backlink-crumb]");

afterEach(() => {
  dispose?.();
  dispose = undefined;
  vi.clearAllMocks();
  setAppState({ page: null, backlinks: [], lastError: null } as never);
  document.body.innerHTML = "";
});

describe("a backlink's breadcrumb line", () => {
  it("is a focusable link with a pointer", () => {
    const el = crumb(mount(link()));
    expect(el).not.toBeNull();
    expect(el?.getAttribute("role")).toBe("link");
    expect(el?.getAttribute("tabindex")).toBe("0");
    expect(el?.className).toContain("cursor-pointer");
    expect(el?.className).toContain("hover:");
  });

  it("opens the page at its context on click", () => {
    const l = link();
    crumb(mount(l))?.click();
    expect(jump.openBacklinkContext).toHaveBeenCalledTimes(1);
    expect(jump.openBacklinkContext.mock.calls[0][0].block_id).toBe("cite");
  });

  it("a ref inside goes to the ref handler, not the jump", () => {
    const host = mount(link());
    const ref = Array.from(host.querySelectorAll<HTMLElement>("[data-backlink-crumb] *")).find(
      (e) => e.children.length === 0 && e.textContent?.includes("buser/vendas"),
    );
    ref?.click();
    expect(nav.handleRefClick).toHaveBeenCalledWith("buser/vendas");
    expect(jump.openBacklinkContext).not.toHaveBeenCalled();
  });

  it("Enter jumps and does not reach a window keydown listener", () => {
    const seen = vi.fn();
    window.addEventListener("keydown", seen);
    try {
      const el = crumb(mount(link()));
      const ev = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
      el?.dispatchEvent(ev);
      expect(jump.openBacklinkContext).toHaveBeenCalledTimes(1);
      expect(ev.defaultPrevented).toBe(true);
      expect(seen).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener("keydown", seen);
    }
  });

  it("Space is not bound: it stays the scroll key, as on a native link", () => {
    const el = crumb(mount(link()));
    const ev = new KeyboardEvent("keydown", { key: " ", bubbles: true, cancelable: true });
    el?.dispatchEvent(ev);
    expect(jump.openBacklinkContext).not.toHaveBeenCalled();
    expect(ev.defaultPrevented).toBe(false);
  });

  it("an orphan row's line is inert", () => {
    const host = mount(link(null));
    expect(crumb(host)).toBeNull();
    expect(host.querySelector('[role="link"]')).toBeNull();
    expect(appState.backlinks.length).toBe(1);
  });
});
