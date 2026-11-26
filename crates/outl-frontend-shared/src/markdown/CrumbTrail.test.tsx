import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { InlineToken } from "../api/types";
import { CrumbTrail, crumbTrailText, type TrailCrumb } from "./CrumbTrail";

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

function mount(
  crumbs: TrailCrumb[],
  handlers: {
    onRefClick?: (target: string) => void;
    onTagClick?: (tag: string) => void;
  } = {},
  /** A Solid `onClick` on the parent, the way a row wires it: Solid
   *  delegates, so a native listener on the host would see the event
   *  before `stopPropagation` runs and prove nothing. */
  onParentClick?: () => void,
): HTMLElement {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => (
      <div onClick={() => onParentClick?.()}>
        <CrumbTrail crumbs={crumbs} {...handlers} />
      </div>
    ),
    host,
  );
  return host;
}

/** The element `MarkdownInline` drew for a ref or tag with this text. */
function chip(host: HTMLElement, text: string): HTMLElement {
  const el = Array.from(host.querySelectorAll<HTMLElement>("*")).find(
    (e) => e.children.length === 0 && e.textContent?.includes(text),
  );
  if (!el) throw new Error(`no element showing ${text}`);
  return el;
}

/** The shape the backend ships: a crumb carries its own tokens, since
 *  no client runs a tokenizer of its own. */
function crumb(text: string, tokens: InlineToken[]): TrailCrumb {
  return { text, tokens };
}

/** The real trail from the bug report, minus the length. */
const ALINHAMENTO: TrailCrumb = crumb(
  "#alinhamento [[buser/team/vendas]] internet 0800",
  [
    { kind: "tag", value: "#alinhamento" },
    { kind: "plain", value: " " },
    { kind: "ref", value: "buser/team/vendas" },
    { kind: "plain", value: " internet 0800" },
  ],
);

describe("CrumbTrail", () => {
  it("renders a ref as its name, never as the bracketed source", () => {
    const host = mount([ALINHAMENTO]);

    // The whole defect: the panel's one line about *where* a reference
    // lives was the one line written in markup.
    expect(host.textContent).toContain("buser/team/vendas");
    expect(host.textContent).not.toContain("[[");
    expect(host.textContent).not.toContain("]]");
  });

  it("renders a tag with its `#`, so a tag still reads as a tag", () => {
    const host = mount([ALINHAMENTO]);

    expect(host.textContent).toContain("#alinhamento");
  });

  it("keeps a link's label and drops the URL", () => {
    const host = mount([
      crumb("mail [email](https://mail.google.com/mail?authuser=someone)", [
        { kind: "plain", value: "mail " },
        {
          kind: "link",
          value: "email",
          href: "https://mail.google.com/mail?authuser=someone",
        },
      ]),
    ]);

    expect(host.textContent).toContain("email");
    // A breadcrumb is one truncated line; the URL swallowed all of it.
    expect(host.textContent).not.toContain("mail.google.com/mail");
  });

  it("separates crumbs with the one separator every client draws", () => {
    const host = mount([
      crumb("parent", [{ kind: "plain", value: "parent" }]),
      crumb("child", [{ kind: "plain", value: "child" }]),
    ]);

    expect(host.textContent).toBe("parent › child");
  });

  it("is inert by default — a crumb's link does nothing", () => {
    // The row already has one destination (the citing block), so the
    // trail wires no handler. `MarkdownInline` renders a handler-less
    // link as a plain span rather than a dead button; a ref and a tag
    // keep their own affordance, which is why this asks about the
    // link rather than counting `role="button"`.
    const host = mount([
      crumb("[email](https://example.com)", [
        { kind: "link", value: "email", href: "https://example.com" },
      ]),
    ]);

    expect(host.querySelectorAll("[tabindex]")).toHaveLength(0);
    expect(host.querySelectorAll('[role="button"]')).toHaveLength(0);
  });

  it("with a handler, a ref and a tag navigate and do not reach the parent", () => {
    const onRef = vi.fn();
    const onTag = vi.fn();
    const parent = vi.fn();
    const host = mount(
      [ALINHAMENTO],
      { onRefClick: onRef, onTagClick: onTag },
      parent,
    );

    chip(host, "buser/team/vendas").click();
    chip(host, "#alinhamento").click();

    expect(onRef).toHaveBeenCalledWith("buser/team/vendas");
    expect(onTag).toHaveBeenCalledWith("#alinhamento");
    expect(parent).not.toHaveBeenCalled();
  });

  it("without a handler, a click on a ref reaches the parent", () => {
    const parent = vi.fn();
    const host = mount([ALINHAMENTO], {}, parent);

    chip(host, "buser/team/vendas").click();

    expect(parent).toHaveBeenCalledTimes(1);
  });

  it("keeps a link unwired even when ref and tag handlers are passed", () => {
    const host = mount(
      [
        crumb("[email](https://example.com)", [
          { kind: "link", value: "email", href: "https://example.com" },
        ]),
      ],
      { onRefClick: vi.fn(), onTagClick: vi.fn() },
    );

    expect(host.querySelectorAll("[tabindex]")).toHaveLength(0);
    expect(host.querySelectorAll('[role="button"]')).toHaveLength(0);
  });

  it("renders nothing for a block at page-root level", () => {
    expect(mount([]).textContent).toBe("");
  });
});

describe("crumbTrailText", () => {
  it("is the plain reading a `title=` tooltip can hold", () => {
    expect(crumbTrailText([ALINHAMENTO])).toBe(
      "#alinhamento [[buser/team/vendas]] internet 0800",
    );
  });

  it("joins with the same separator the rendered trail draws", () => {
    expect(
      crumbTrailText([
        crumb("a", [{ kind: "plain", value: "a" }]),
        crumb("b", [{ kind: "plain", value: "b" }]),
      ]),
    ).toBe("a › b");
  });
});
