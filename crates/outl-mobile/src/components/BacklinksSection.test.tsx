import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { Backlink } from "@outl/shared/api/types";

import { BacklinksSection } from "./BacklinksSection";

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

/** The trail from the bug report: a tag, a namespaced ref and a mail
 *  link, in the branch the citing block sits under. */
const ancestors: Backlink["ancestors"] = [
  {
    id: "crumb-1",
    text: "#alinhamento [[buser/team/vendas]] internet 0800 [email](https://mail.google.com/mail?authuser=someone)",
    tokens: [
      { kind: "tag", value: "#alinhamento" },
      { kind: "plain", value: " " },
      { kind: "ref", value: "buser/team/vendas" },
      { kind: "plain", value: " internet 0800 " },
      {
        kind: "link",
        value: "email",
        href: "https://mail.google.com/mail?authuser=someone",
      },
    ],
  },
];

function backlink(): Backlink {
  return {
    block_id: "b1",
    todo: null,
    ancestors,
    source_page: { id: "p1", slug: "journal", title: "Journal", kind: "journal" },
    source_block: {
      id: "b1",
      text: "cites [[project]]",
      todo: null,
      tokens: [{ kind: "plain", value: "cites " }],
      collapsed: false,
      properties: [],
      children: [],
    },
    source_block_path: [],
  };
}

function mount(backlinks: Backlink[]): HTMLElement {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => (
      <BacklinksSection
        backlinks={backlinks}
        onJump={vi.fn()}
        order="newest"
        onToggleOrder={vi.fn()}
      />
    ),
    host,
  );
  return host;
}

describe("BacklinksSection breadcrumb", () => {
  // Parity with the desktop: both clients render the trail through
  // `<CrumbTrail />`, so neither can start showing the raw source
  // while the other renders it.
  it("renders the ancestor trail instead of its markdown source", () => {
    const host = mount([backlink()]);

    expect(host.textContent).toContain("buser/team/vendas");
    expect(host.textContent).toContain("#alinhamento");
    expect(host.textContent).toContain("email");
    expect(host.textContent).not.toContain("[[");
    expect(host.textContent).not.toContain("](");
    expect(host.textContent).not.toContain("mail.google.com");
  });
});
