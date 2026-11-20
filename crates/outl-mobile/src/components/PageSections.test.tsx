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
  zoomed = false,
): HTMLElement {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => (
      <PageSections
        backlinks={backlinks}
        pageKind={pageKind}
        zoomed={zoomed}
        onToggleOrder={vi.fn()}
        onEditCommitted={vi.fn()}
        onError={vi.fn()}
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

  // The other side of that line: an edit committed through a backlink
  // row has to re-ask for the rows **without** re-running the resource,
  // or the `undefined` above unmounts the card the user just typed
  // into. `Journal.backlinks.ts` owns the swap; this is the wiring.
  it("re-asks for the rows after a commit through a row", () => {
    const source = readFileSync(
      join(import.meta.dirname, "Journal.tsx"),
      "utf8",
    );
    expect(source).toContain("onEditCommitted={backlinkCard.committed}");
  });

  // The refusal's last mile. A cross-page write can be rejected (a
  // stale block id, a source page that stopped syncing), and mobile has
  // no status line — `UX.md`'s channel table gives it the toast. Delete
  // this prop and every component test still passes while the refusal
  // vanishes, which is why the line itself is pinned.
  it("routes a refusal from a backlink row into the toast", () => {
    const source = readFileSync(
      join(import.meta.dirname, "Journal.tsx"),
      "utf8",
    );
    expect(source).toContain("onError={backlinkCard.refused}");
  });

  // The other half of the collapse: the section reports *which* page a
  // reply describes, and the host is where both page ids exist, so
  // `backlinkCommitPlan` runs there. A section taking `openPageId` is
  // the leak this replaced.
  it("keeps the open page's identity out of the section", () => {
    const source = readFileSync(
      join(import.meta.dirname, "PageSections.tsx"),
      "utf8",
    );
    expect(source).not.toContain("openPageId");
    expect(source).not.toContain("onApplySourceView");
  });
});

/**
 * Zooming into a block is a request for that subtree and nothing else.
 *
 * Every section here answers a question about the **page**, so under a
 * zoom — where the header is a block — the page's backlinks read as
 * that block's. That is a wrong answer, not a noisy one. The desktop
 * hides the same three (plus the page's `key:: value` row, which this
 * client draws from `Journal.tsx`).
 */
describe("none of the page's sections survive a zoom", () => {
  it("hides the backlinks card the scan already answered for", () => {
    const host = mount(reply(2), "journal", true);

    expect(host.textContent).not.toContain("Linked from 2");
    expect(host.querySelector("section")).toBeNull();
  });

  it("hides the empty-state card a regular page would otherwise keep", () => {
    // `keepWhenEmpty` is what earns that box its space on a page, and
    // the zoom outranks it: discoverability is a page-level job.
    const host = mount(reply(0), "page", true);

    expect(host.textContent).not.toContain("No backlinks yet");
  });

  it("still draws them on the full page", () => {
    const host = mount(reply(2), "journal", false);
    expect(host.textContent).toContain("Linked from 2");
  });
});

/**
 * Two more facts that live in `Journal.tsx` and therefore get the
 * source-pin treatment above: what it hands `zoomed`, and the page's
 * own `key:: value` row, which is page metadata sitting above a block
 * title while zoomed.
 */
describe("Journal scopes the page's own surfaces to the full page", () => {
  const source = () =>
    readFileSync(join(import.meta.dirname, "Journal.tsx"), "utf8");

  it("tells PageSections when the outline is zoomed", () => {
    expect(source()).toContain("zoomed={focusView() !== null}");
  });

  it("keeps the page property row out of a zoom", () => {
    expect(source()).toContain('<Show when={!focusView()}>\n          <BlockProperties');
  });
});

/**
 * `<PageSections />` hands the card its callbacks, and a prop it
 * declares but does not forward compiles: both are optional on the
 * child. `onEditCommitted` and `onError` are exactly that shape — drop
 * either from the `<BacklinksSection>` call and every row edit still
 * writes, but the host never re-asks for the rows, or a refusal never
 * reaches the toast.
 *
 * Read from the JSX, the way `BlockRow.recursion.test.ts` reads its own:
 * every prop this component accepts is either referenced inside the
 * `<BacklinksSection>` call or listed below with a reason.
 */
describe("PageSections forwards what the card needs", () => {
  const SOURCE = readFileSync(
    join(import.meta.dirname, "PageSections.tsx"),
    "utf8",
  );

  /** Props the card is right not to receive: this component consumes them. */
  const NOT_FORWARDED: Record<string, string> = {
    pageKind: "decides whether the empty card shows (`keepWhenEmpty`)",
    blank: "drives the first-run tips, which sit above the card",
    zoomed: "hides every section, the card included",
  };

  function declaredProps(): string[] {
    const start = SOURCE.indexOf("export interface PageSectionsProps {");
    expect(start, "interface PageSectionsProps not found").toBeGreaterThan(-1);
    const body = SOURCE.slice(start, SOURCE.indexOf("\n}", start));
    return [...body.matchAll(/^\s{2}(\w+)\??:/gm)].map((m) => m[1]);
  }

  function cardCall(): string {
    const start = SOURCE.indexOf("<BacklinksSection");
    expect(start, "<BacklinksSection> not found").toBeGreaterThan(-1);
    return SOURCE.slice(start, SOURCE.indexOf("/>", start));
  }

  it("passes every prop it accepts to the card, or says why not", () => {
    const call = cardCall();
    const missing = declaredProps().filter(
      (name) => !call.includes(`props.${name}`) && !(name in NOT_FORWARDED),
    );
    expect(
      missing,
      `these props stop at PageSections: ${missing.join(", ")}. ` +
        "Forward them to <BacklinksSection>, or add a row to NOT_FORWARDED.",
    ).toEqual([]);
  });

  it("reads a non-trivial prop list", () => {
    // A guard on the guard: a regex that stops matching would pass the
    // test above vacuously.
    expect(declaredProps().length).toBeGreaterThan(6);
  });

  it("hands the card the commit and the refusal by name", () => {
    const call = cardCall();
    expect(call).toContain("onEditCommitted={props.onEditCommitted}");
    expect(call).toContain("onError={props.onError}");
  });
});
