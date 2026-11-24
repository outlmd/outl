/**
 * The subtree under a citing block: rendered, and editable row by row.
 *
 * Every row is keyed on `BacklinkRow.key`, never on the block id, because
 * a block that cites the page and also sits under another citing block
 * is drawn twice — and two rows sharing one editor slot is the bug.
 */
import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

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
vi.mock("../lib/haptics", () => ({ haptic: vi.fn() }));

let dispose: (() => void) | undefined;
beforeEach(() => vi.clearAllMocks());
afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

function node(id: string, children: BlockNode[] = []): BlockNode {
  return {
    id,
    text: `text ${id}`,
    todo: null,
    tokens: [{ kind: "plain", value: `text ${id}` }],
    collapsed: false,
    properties: [],
    children,
  } as unknown as BlockNode;
}

function link(id: string, children: BlockNode[] = []): Backlink {
  return {
    block_id: id,
    todo: null,
    ancestors: [],
    source_page: { id: "p1", slug: "journal", title: "Journal", kind: "journal" },
    source_block: node(id, children),
    source_block_path: [],
  } as unknown as Backlink;
}

const view = () => ({ page: { id: "p1", slug: "journal" } }) as unknown as PageView;

function mount(backlinks: Backlink[]) {
  const onEditCommitted = vi.fn();
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => (
      <BacklinksSection
        backlinks={backlinks}
        onJump={vi.fn()}
        order="newest"
        onToggleOrder={vi.fn()}
        onEditCommitted={(c) => onEditCommitted(c)}
        onError={vi.fn()}
      />
    ),
    host,
  );
  return { host, onEditCommitted };
}

const byKey = (host: HTMLElement, key: string) =>
  host.querySelector<HTMLElement>(`[data-backlink-key="${key}"]`)!;
const tick = () => new Promise((r) => setTimeout(r, 0));

async function longPress(el: HTMLElement) {
  el.dispatchEvent(
    new PointerEvent("pointerdown", { bubbles: true, clientX: 1, clientY: 1 }),
  );
  await new Promise((r) => setTimeout(r, 520));
  el.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
  el.click();
}

async function tapMenu(label: string) {
  const hit = [...document.querySelectorAll("[data-backlink-menu] button")].find(
    (b) => b.textContent?.trim() === label,
  ) as HTMLElement;
  hit.click();
  await tick();
}

function type(ta: HTMLTextAreaElement, value: string) {
  ta.value = value;
  ta.dispatchEvent(new InputEvent("input", { bubbles: true }));
}

describe("backlink subtree rows", () => {
  it("renders the children under the citing block, indented", () => {
    const h = mount([link("a", [node("c1", [node("c2")])])]);

    expect(h.host.textContent).toContain("text a");
    expect(h.host.textContent).toContain("text c1");
    expect(h.host.textContent).toContain("text c2");
    expect(
      h.host.querySelector('[data-backlink-depth="2"]'),
    ).not.toBeNull();
  });

  it("a long press on a child opens the sheet for that child, not the parent", async () => {
    commands.backlinkRowOp.mockResolvedValue(view());
    const h = mount([link("a", [node("c1")])]);

    await longPress(byKey(h.host, "a>c1"));
    await tapMenu("Indent");

    expect(commands.backlinkRowOp).toHaveBeenCalledTimes(1);
    expect(commands.backlinkRowOp).toHaveBeenCalledWith("p1", "c1", "indent");
  });

  it("editing a child commits with the child's id", async () => {
    commands.editBacklinkBlock.mockResolvedValue(view());
    const h = mount([link("a", [node("c1")])]);

    byKey(h.host, "a>c1").click();
    const tas = h.host.querySelectorAll("textarea");
    expect(tas).toHaveLength(1);
    type(tas[0], "edited child");
    tas[0].dispatchEvent(new FocusEvent("blur"));
    await tick();

    expect(commands.editBacklinkBlock).toHaveBeenCalledWith("p1", "c1", "edited child");
  });

  it("a block drawn twice gets two keys and two independent editors", async () => {
    // `x` cites the page and also sits under citing block `a`.
    const h = mount([link("a", [node("x")]), link("x")]);

    const own = byKey(h.host, "x");
    const inside = byKey(h.host, "a>x");
    expect(own).not.toBe(inside);

    inside.click();
    expect(h.host.querySelectorAll("textarea")).toHaveLength(1);
    // The own row is still the read-only button: one editor, not two.
    expect(byKey(h.host, "x")).not.toBeNull();
    expect(byKey(h.host, "a>x")).toBeNull();
  });

  it("deleting a child that has children asks first, with the child's count", async () => {
    commands.backlinkRowOp.mockResolvedValue(view());
    const h = mount([link("a", [node("c1", [node("c2"), node("c3")])])]);

    await longPress(byKey(h.host, "a>c1"));
    await tapMenu("Delete");

    expect(document.querySelector('[role="alertdialog"]')?.textContent).toContain(
      "2 children",
    );
    expect(commands.backlinkRowOp).not.toHaveBeenCalled();

    [...document.querySelectorAll('[role="alertdialog"] button')]
      .find((b) => b.textContent?.trim() === "Delete")!
      .dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await tick();

    expect(commands.backlinkRowOp).toHaveBeenCalledWith("p1", "c1", "delete");
  });

  it("a sibling created from a child reports its depth so the host stays quiet", async () => {
    commands.createBacklinkSibling.mockResolvedValue({ view: view(), new_id: "n" });
    const h = mount([link("a", [node("c1")])]);

    await longPress(byKey(h.host, "a>c1"));
    await tapMenu("New block below");

    expect(commands.createBacklinkSibling).toHaveBeenCalledWith("p1", "c1", "below");
    expect(h.onEditCommitted.mock.calls[0][0].created.depth).toBe(1);
  });

  // These rows are other pages' blocks, so the open page's embed map never
  // held their handles: a child that is only `!((blk-…))` showed the raw
  // handle until the panel started resolving its own.
  it("resolves a block embed that sits in a child row", async () => {
    commands.resolveEmbeds.mockResolvedValueOnce({
      "blk-qs4fmz": {
        handle: "blk-qs4fmz",
        text: "the embedded source",
        page_slug: "elsewhere",
        status: null,
        children: [],
      },
    });
    const embed = {
      ...node("e"),
      text: "!((blk-qs4fmz))",
      tokens: [{ kind: "embed", value: "blk-qs4fmz" }],
    } as unknown as BlockNode;
    const h = mount([link("a", [embed])]);
    expect(h.host.textContent).toContain("!blk-qs4fmz");

    await vi.waitFor(() =>
      expect(h.host.textContent).toContain("the embedded source"),
    );
    expect(h.host.textContent).not.toContain("!blk-qs4fmz");
  });
});
