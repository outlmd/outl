/**
 * The breadcrumb above a backlink row is a tap target: it opens the
 * source page at the crumb's block. A `[[ref]]` / `#tag` inside it
 * navigates on its own and must not also trigger the jump.
 */
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { Backlink } from "@outl/shared/api/types";

import { BacklinksSection } from "./BacklinksSection";

vi.mock("../lib/haptics", () => ({ haptic: vi.fn() }));
vi.mock("@outl/shared/api/commands", () => ({
  resolveEmbeds: vi.fn(() => Promise.resolve({})),
  resolvePageEmbeds: vi.fn(() => Promise.resolve({})),
}));

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

const ancestors: Backlink["ancestors"] = [
  {
    id: "crumb-1",
    text: "#whatsapp [[prazos]]",
    tokens: [
      { kind: "tag", value: "#whatsapp" },
      { kind: "plain", value: " " },
      { kind: "ref", value: "prazos" },
    ],
  },
];

function link(over: Partial<Backlink> = {}): Backlink {
  return {
    block_id: "b1",
    todo: null,
    ancestors,
    source_page: { id: "p1", slug: "journal", title: "Journal", kind: "journal" },
    source_block: {
      id: "b1",
      text: "cites",
      todo: null,
      tokens: [{ kind: "plain", value: "cites" }],
      collapsed: false,
      properties: [],
      children: [],
    },
    source_block_path: [],
    ...over,
  } as Backlink;
}

function mount(l: Backlink, extra: Record<string, unknown> = {}) {
  const spies = {
    onJumpToCrumb: vi.fn(),
    onRefClick: vi.fn(),
    onTagClick: vi.fn(),
  };
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => (
      <BacklinksSection
        backlinks={[l]}
        onJump={vi.fn()}
        order="newest"
        onToggleOrder={vi.fn()}
        onEditCommitted={vi.fn()}
        onError={vi.fn()}
        {...spies}
        {...extra}
      />
    ),
    host,
  );
  return { host, ...spies };
}

const crumb = (host: HTMLElement) =>
  host.querySelector<HTMLElement>("[data-backlink-crumb]");

describe("backlink breadcrumb tap", () => {
  it("calls onJumpToCrumb with the link, and the box is a 44pt target", () => {
    const l = link();
    const { host, onJumpToCrumb } = mount(l);
    const el = crumb(host)!;
    expect(el.getAttribute("role")).toBe("link");
    expect(el.className).toContain("min-h-11");
    expect(el.className).toContain("text-[12px]");
    el.click();
    expect(onJumpToCrumb).toHaveBeenCalledWith(l);
  });

  it("Enter on the focused crumb jumps like a tap", () => {
    const l = link();
    const { host, onJumpToCrumb } = mount(l);
    crumb(host)!.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }),
    );
    expect(onJumpToCrumb).toHaveBeenCalledWith(l);
  });

  it("a tap on a tag or ref inside the trail navigates and does not jump", () => {
    const { host, onJumpToCrumb, onRefClick, onTagClick } = mount(link());
    const tokens = [...crumb(host)!.querySelectorAll<HTMLElement>("[role=button]")];
    expect(tokens.length).toBe(2);
    for (const el of tokens) el.click();
    expect(onTagClick).toHaveBeenCalledWith("#whatsapp");
    expect(onRefClick).toHaveBeenCalledWith("prazos");
    expect(onJumpToCrumb).not.toHaveBeenCalled();
  });

  it("an orphan row (no source page) stays inert", () => {
    const { host, onJumpToCrumb } = mount(link({ source_page: null }));
    expect(crumb(host)).toBeNull();
    expect(onJumpToCrumb).not.toHaveBeenCalled();
  });
});
