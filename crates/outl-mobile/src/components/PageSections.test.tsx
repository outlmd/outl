/**
 * What renders below the outline while the `page_backlinks` scan is
 * still running.
 *
 * The scan is `O(blocks-in-workspace)` and lands after the first paint
 * (`UX.md` → Never block a keystroke). The rule for a section fed by
 * such a reply is `lateSectionVisible`'s (`@outl/shared/outline`): it
 * never appears and then disappears, and it never answers a question
 * it has not been told the answer to.
 *
 * Both halves were live here. On a regular page the card mounted
 * immediately and said "Linked from 0" / "No backlinks yet" while the
 * scan ran, then contradicted itself; on every page a navigation kept
 * the *previous* page's rows on screen, because Solid's
 * `createResource` holds its last value through a source change.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { PageBacklinks } from "@outl/shared/api/types";

import { PageSections } from "./PageSections";

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

function reply(count: number): PageBacklinks {
  return {
    backlinks: Array.from({ length: count }, (_, i) => ({
      block_id: `b${i}`,
      todo: null,
      ancestors: [],
      source_page: {
        id: "p1",
        slug: "journal",
        title: "Journal",
        kind: "journal" as const,
      },
      source_block: {
        id: `b${i}`,
        text: "cites [[project]]",
        todo: null,
        tokens: [{ kind: "plain" as const, value: "cites " }],
        collapsed: false,
        properties: [],
        children: [],
      },
      source_block_path: [],
    })),
    backlinks_order: "newest",
    namespace_children: [],
    namespace_backlinks: [],
    namespace_backlinks_total: 0,
  };
}

function mount(
  backlinks: PageBacklinks | undefined,
  pageKind: "page" | "journal",
): HTMLElement {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => (
      <PageSections
        backlinks={backlinks}
        pageKind={pageKind}
        onToggleOrder={vi.fn()}
        onOpenPage={vi.fn()}
      />
    ),
    host,
  );
  return host;
}

describe("the backlinks card never answers before the scan does", () => {
  it("draws nothing on a regular page while the reply is in flight", () => {
    const host = mount(undefined, "page");

    // "No backlinks yet" / "Linked from 0" is a wrong answer, not a
    // pending one — and it is one this page then takes back.
    expect(host.querySelector("section")).toBeNull();
    expect(host.textContent).not.toContain("No backlinks yet");
  });

  it("draws nothing on a journal while the reply is in flight", () => {
    const host = mount(undefined, "journal");
    expect(host.querySelector("section")).toBeNull();
  });

  it("shows the empty state on a page once the scan says zero", () => {
    const host = mount(reply(0), "page");

    // The discoverability box earns its space only as an answer: a
    // newcomer learns pages can cite each other.
    expect(host.textContent).toContain("No backlinks yet");
  });

  it("stays absent on a journal that the scan says has none", () => {
    const host = mount(reply(0), "journal");
    expect(host.querySelector("section")).toBeNull();
  });

  it("shows the rows once the scan finds them", () => {
    const host = mount(reply(2), "journal");
    expect(host.textContent).toContain("Linked from 2");
  });
});

/**
 * The one line that makes the prop's `undefined` contract true lives in
 * `Journal.tsx`, which no component test renders — it needs haptics, a
 * Tauri bridge and a WKWebView-shaped textarea, and mocking all three
 * would pin the mocks. So the JSX is read, the way
 * `BlockRow.recursion.test.ts` reads its own.
 *
 * Solid's `createResource` keeps its last value through a source
 * change. Hand that straight to `<PageSections />` and a navigation
 * leaves the page the user just left answering for the one they opened:
 * its backlinks, its nested pages, its namespace mentions, for the
 * whole `O(blocks-in-workspace)` scan.
 */
describe("Journal hands PageSections an answer for the page on screen", () => {
  it("blanks the reply while the scan runs", () => {
    const source = readFileSync(
      join(import.meta.dirname, "Journal.tsx"),
      "utf8",
    );
    expect(source).toContain(
      "backlinks={backlinks.loading ? undefined : backlinks()}",
    );
  });
});
