/**
 * A "Linked from" row draws the citing block with the outline's own
 * read branch (`<BlockReadBody>`), so the block is the same thing in
 * the row as on its page.
 *
 * The bug this pins: the row branched only "table, else inline tokens"
 * inside a `<p>`, so a ```` ```call:x ```` fence read as one line of
 * raw backticks and a block of several lines ran together.
 * `BacklinksSection.parity.test.ts` stops a second renderer coming back.
 */
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { Backlink, BlockNode } from "@outl/shared/api/types";

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

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  vi.clearAllMocks();
  document.body.innerHTML = "";
});

function node(text: string, over: Partial<BlockNode> = {}): BlockNode {
  return {
    id: "b1",
    text,
    todo: null,
    tokens: [{ kind: "plain", value: text }],
    collapsed: false,
    properties: [],
    children: [],
    ...over,
  };
}

function backlink(n: BlockNode): Backlink {
  return {
    block_id: n.id,
    todo: null,
    ancestors: [],
    source_page: { id: "p1", slug: "journal", title: "Journal", kind: "journal" },
    source_block: n,
    source_block_path: [],
  };
}

function mount(
  backlinks: Backlink[],
  nav: { onRefClick?: (target: string) => void } = {},
): HTMLElement {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => (
      <BacklinksSection
        backlinks={backlinks}
        onJump={() => {}}
        order="newest"
        onToggleOrder={() => {}}
        onEditCommitted={() => {}}
        onError={() => {}}
        onRefClick={nav.onRefClick}
      />
    ),
    host,
  );
  return host;
}

const row = (host: HTMLElement) =>
  host.querySelector<HTMLElement>("[data-backlink-row]")!;

describe("a backlink row renders through the outline's BlockReadBody", () => {
  it("draws a call: fence as highlighted code, not a line of backticks", () => {
    const host = mount([backlink(node("```call:x\na: 1\n```"))]);

    const code = row(host).querySelector("pre.outl-codeblock");
    expect(code).not.toBeNull();
    expect(code!.textContent).toContain("a: 1");
    expect(row(host).textContent).not.toContain("```");
  });

  it("keeps the line breaks of a multi-line block", () => {
    const host = mount([backlink(node("first line\nsecond line"))]);
    const prose = row(host).querySelector("p")!;

    expect(prose.className).toContain("whitespace-pre-wrap");
    expect(prose.textContent).toBe("first line\nsecond line");
  });

  it("a tap on a ref navigates and does not also open the editor", () => {
    const onRefClick = vi.fn();
    const host = mount(
      [
        backlink(
          node("see [[other]]", {
            tokens: [
              { kind: "plain", value: "see " },
              { kind: "ref", value: "other" },
            ],
          }),
        ),
      ],
      { onRefClick },
    );
    const ref = row(host).querySelector<HTMLElement>("[role='button']")!;

    ref.click();

    expect(onRefClick).toHaveBeenCalledWith("other");
    expect(host.querySelector("textarea")).toBeNull();
  });

  it("a tap on the text still opens the editor", () => {
    const host = mount([backlink(node("plain words"))]);

    row(host).querySelector("p")!.click();

    expect(host.querySelector("textarea")).not.toBeNull();
  });
});
