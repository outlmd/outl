/**
 * A "Linked from" row draws the citing block **with the outline's own
 * renderer**, so whatever the block is on its page — a fence, a quote,
 * several lines, a property — it is the same thing in the row.
 *
 * The bug this pins: the row branched only "table, else inline tokens",
 * so a ```` ```call:x ```` fence read as one line of raw backticks and
 * its Run was unreachable. `BacklinkRow.parity.test.ts` stops the
 * second renderer coming back; this stops the one renderer being wired
 * to the wrong page.
 */
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type {
  Backlink,
  BlockNode,
  PageBacklinks,
  PageView,
  RunCodeBlockReply,
} from "@outl/shared/api/types";

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

import { openRef, pageBacklinks, runCodeBlock } from "@outl/shared/api/commands";

import { InlineBacklinks } from "./InlineBacklinks";
import { appState, setAppState } from "../lib/store";

const OPEN_PAGE = { id: "tpl-page", slug: "template-x", title: "template-x", kind: "page" } as const;
const SOURCE_PAGE = { id: "journal-page", slug: "2026-11-26", title: "2026-11-26", kind: "journal" } as const;

function node(id: string, text: string, over: Partial<BlockNode> = {}): BlockNode {
  return {
    id,
    text,
    todo: null,
    tokens: [{ kind: "plain", value: text }],
    collapsed: false,
    properties: [],
    children: [],
    ...over,
  };
}

function link(n: BlockNode, sourcePage: Backlink["source_page"] = SOURCE_PAGE): Backlink {
  return {
    block_id: n.id,
    todo: null,
    ancestors: [],
    source_page: sourcePage,
    source_block: n,
    source_block_path: [],
  };
}

function view(page: PageView["page"]): PageView {
  return { page, outline: [], backlinks: [], backlinks_order: "newest" };
}

function reply(backlinks: Backlink[]): PageBacklinks {
  return {
    backlinks,
    backlinks_order: "newest",
    namespace_children: [],
    namespace_backlinks: [],
    namespace_backlinks_total: 0,
  };
}

const CALL_FENCE = "```call:x\na: 1\n```";

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

function runButton(host: HTMLElement): HTMLButtonElement {
  const found = Array.from(host.querySelectorAll("button")).find((b) =>
    b.textContent?.includes("Run"),
  );
  if (!found) throw new Error("the fence's Run button did not render");
  return found;
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
    lastError: null,
  });
  document.body.innerHTML = "";
});

describe("a backlink row renders through the outline's BlockBody", () => {
  it("draws a call: fence as a code fence, not a line of backticks", () => {
    const host = mount([link(node("b-call", CALL_FENCE))]);
    const body = host.querySelector("[data-backlink-body]")!;

    expect(body.querySelector("pre.outl-codeblock")).not.toBeNull();
    expect(body.textContent).toContain("call:x");
    expect(body.textContent).not.toContain("```");
    expect(runButton(host)).toBeDefined();
  });

  it("runs the fence on the source page and refetches this page's backlinks", async () => {
    const ran: RunCodeBlockReply = {
      language: "call:x",
      result_ok: null,
      error: null,
      view: view(SOURCE_PAGE),
    };
    vi.mocked(runCodeBlock).mockResolvedValue(ran);
    vi.mocked(pageBacklinks).mockResolvedValue(
      reply([link(node("b-call", CALL_FENCE))]),
    );
    const host = mount([link(node("b-call", CALL_FENCE))]);

    runButton(host).click();

    await vi.waitFor(() => {
      // The block lives on the journal, not on the page on screen.
      expect(runCodeBlock).toHaveBeenCalledWith("journal-page", "b-call");
      expect(pageBacklinks).toHaveBeenCalledWith("template-x");
    });
    // The reply is a view of the journal: installing it would swap the
    // page the user is reading for the one they ran from.
    expect(appState.page?.id).toBe("tpl-page");
  });

  it("surfaces a runtime error on the status line", async () => {
    vi.mocked(runCodeBlock).mockResolvedValue({
      language: "call:x",
      result_ok: null,
      error: "unknown template",
      view: view(SOURCE_PAGE),
    });
    vi.mocked(pageBacklinks).mockResolvedValue(reply([]));
    const host = mount([link(node("b-call", CALL_FENCE))]);

    runButton(host).click();

    await vi.waitFor(() => {
      expect(appState.lastError).toBe("call:x: unknown template");
    });
  });

  it("an orphan row reports instead of running against a guessed page", async () => {
    const host = mount([link(node("b-call", CALL_FENCE), null)]);

    runButton(host).click();

    await vi.waitFor(() => {
      expect(appState.lastError).toMatch(/no page/);
    });
    expect(runCodeBlock).not.toHaveBeenCalled();
  });

  it("keeps the line breaks of a multi-line block", () => {
    const host = mount([link(node("b-lines", "first line\nsecond line"))]);
    const prose = host.querySelector("[data-backlink-body] .cursor-text")!;

    expect(prose.className).toContain("whitespace-pre-wrap");
    expect(prose.textContent).toBe("first line\nsecond line");
  });

  it("follows a ref inside the row without opening the editor", async () => {
    vi.mocked(openRef).mockResolvedValue(view({
      id: "other",
      slug: "other",
      title: "other",
      kind: "page",
    }));
    const host = mount([
      link(
        node("b-ref", "see [[other]]", {
          tokens: [
            { kind: "plain", value: "see " },
            { kind: "ref", value: "other" },
          ],
        }),
      ),
    ]);
    const ref = Array.from(
      host.querySelectorAll<HTMLElement>("[data-backlink-body] *"),
    ).find((el) => el.textContent === "other" && el.children.length === 0)!;

    ref.click();

    await vi.waitFor(() => expect(openRef).toHaveBeenCalledWith("other"));
    expect(host.querySelector("textarea")).toBeNull();
  });

  it("draws properties read-only: no editor control in the row", () => {
    const host = mount([
      link(node("b-prop", "status:: waiting", { properties: [["status", "waiting"]] })),
    ]);
    const body = host.querySelector("[data-backlink-body]")!;

    expect(body.textContent).toContain("waiting");
    expect(body.querySelector("button")).toBeNull();
    expect(body.querySelector("input")).toBeNull();
  });
});
