/**
 * Opening a page you are already on.
 *
 * The zoom used to be dropped by one signal: `applyView` noticing a
 * different slug. Today's journal is usually the open page, so Today,
 * the week strip's current day, a `[[ref]]` pointing back here and a
 * deep link to this page all carried the same slug — and the zoom they
 * were meant to leave stayed up, with nothing on screen moving.
 *
 * These drive `createNavigation` directly, which is the whole reason it
 * is a module: the behaviour used to live in two closures inside
 * `Journal.tsx`, where no test reaches it.
 */
import { describe, expect, it, vi } from "vitest";

import type { PageView } from "@outl/shared/api/types";

import { createNavigation } from "./Journal.navigation";

function view(slug = "2026-11-16"): PageView {
  return {
    page: { id: "pg-1", slug, title: "Today", kind: "journal" },
    outline: [],
  } as unknown as PageView;
}

function harness() {
  const calls = {
    applyView: vi.fn(),
    setFocusBlockId: vi.fn(),
    setSelection: vi.fn(),
  };
  return { nav: createNavigation(calls), calls };
}

describe("createNavigation", () => {
  it("drops the zoom even when the view is the page already open", () => {
    const { nav, calls } = harness();

    nav.openView(view());

    expect(calls.setFocusBlockId).toHaveBeenCalledWith(null);
    expect(calls.applyView).toHaveBeenCalledOnce();
  });

  it("drops a range selection with it", () => {
    // Its block ids are the other page's, and on the same page the user
    // asked to go somewhere rather than to keep a selection.
    const { nav, calls } = harness();

    nav.openView(view());

    expect(calls.setSelection).toHaveBeenCalledWith(null);
  });

  it("resets before the view lands, never after", () => {
    // The other order renders one frame of the new page through the old
    // page's zoom.
    const order: string[] = [];
    const nav = createNavigation({
      applyView: () => order.push("applyView"),
      setFocusBlockId: () => order.push("reset"),
      setSelection: () => {},
    });

    nav.openView(view());

    expect(order).toEqual(["reset", "applyView"]);
  });

  it("exposes the reset on its own, for the paths that set their own zoom", () => {
    // The reminder tap opens a page and then zooms into the block the
    // reminder is on; it needs the reset without `openView`'s ordering.
    const { nav, calls } = harness();

    nav.resetForNavigation();

    expect(calls.setFocusBlockId).toHaveBeenCalledWith(null);
    expect(calls.applyView).not.toHaveBeenCalled();
  });
});
