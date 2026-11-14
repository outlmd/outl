import { render } from "solid-js/web";
import { afterEach, describe, expect, it } from "vitest";

import type { PageMeta } from "@outl/shared/api/types";

import { PageTitle } from "./PageTitle";
import { setAppState } from "../lib/store";

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  setAppState("todaySlug", null);
  document.body.innerHTML = "";
});

function journal(slug: string): PageMeta {
  return { id: `page-${slug}`, slug, title: slug, kind: "journal" };
}

function mount(page: PageMeta) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <PageTitle page={page} />, host);
  return host;
}

describe("PageTitle", () => {
  it("calls the open journal Today while it is today", () => {
    setAppState("todaySlug", "2026-11-13");
    expect(mount(journal("2026-11-13")).textContent).toContain("Today");
  });

  /**
   * The regression: a window left open overnight. Nothing navigates,
   * nothing is clicked — only the day clock moves — and the header has
   * to stop claiming the page on screen is today.
   */
  it("stops calling it Today once the day rolls over under an untouched window", () => {
    setAppState("todaySlug", "2026-11-13");
    const host = mount(journal("2026-11-13"));
    expect(host.textContent).toContain("Today");

    setAppState("todaySlug", "2026-11-14");

    expect(host.textContent).not.toContain("Today");
    expect(host.textContent).toContain("Yesterday");
    expect(host.textContent).toContain("November 13, 2026");
  });

  it("gives every day its weekday while today is still unknown", () => {
    expect(mount(journal("2026-11-13")).textContent).toContain("Friday");
  });

  it("shows a regular page's title, whatever day it is", () => {
    setAppState("todaySlug", "2026-11-14");
    const page: PageMeta = { id: "p", slug: "inbox", title: "Inbox", kind: "page" };
    expect(mount(page).textContent).toBe("Inbox");
  });
});
