/**
 * What sits below the outline, and the one state where none of it does.
 *
 * Every section here answers a question about the **page**: who links
 * to it, what lives under its namespace, which blocks reach it through
 * a descendant. Zoomed into a block, the header is that block — so the
 * page's references rendered under a block title read as the block's,
 * which is a wrong answer rather than a noisy one.
 */
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { Backlink, NamespaceChild } from "@outl/shared/api/types";

import { PageSections } from "./PageSections";
import { setAppState } from "../lib/store";

vi.mock("@outl/shared/api/commands", () => ({
  openRef: vi.fn(),
  pageBacklinks: vi.fn(),
  setBacklinksOrder: vi.fn(),
  toggleTodo: vi.fn(),
}));

let dispose: (() => void) | undefined;

function backlink(id: string): Backlink {
  return {
    block_id: id,
    todo: null,
    ancestors: [],
    source_page: {
      id: "page-1",
      slug: "other",
      title: "Other page",
      kind: "page",
    },
    source_block: {
      id,
      text: "points at this page",
      todo: null,
      tokens: [{ kind: "plain", value: "points at this page" }],
      collapsed: false,
      properties: [],
      children: [],
    },
    source_block_path: [],
  };
}

function child(slug: string): NamespaceChild {
  return {
    page: { id: `pg-${slug}`, slug, title: slug, kind: "page" },
    depth: 1,
    label: slug,
  };
}

/** A page with something to show in all three sections. */
function mount(zoomed: boolean): HTMLElement {
  setAppState({
    page: { id: "pg-1", slug: "os", title: "os", kind: "page" },
    backlinksOpen: true,
    backlinksLoading: false,
    backlinks: [backlink("blk-ref")],
    backlinksOrder: "newest",
    namespaceChildren: [child("os-linux")],
    namespaceMentions: [backlink("blk-mention")],
    namespaceMentionsTotal: 1,
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => <PageSections zoomed={zoomed} onOpenPage={vi.fn()} />,
    host,
  );
  return host;
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

describe("PageSections", () => {
  it("shows the page's sections on the full page", () => {
    const host = mount(false);

    expect(host.textContent).toContain("Backlinks");
    expect(host.textContent).toContain("os-linux");
  });

  /**
   * The bug this file exists for: zooming is a request for one subtree
   * and nothing else, and the sections kept answering for the page the
   * block happens to live on.
   */
  it("shows none of them while zoomed into a block", () => {
    const host = mount(true);

    expect(host.textContent).not.toContain("Backlinks");
    expect(host.textContent).not.toContain("os-linux");
    expect(host.textContent).not.toContain("points at this page");
  });
});
