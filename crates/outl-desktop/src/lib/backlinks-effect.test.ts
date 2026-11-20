import { createRoot } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  pageBacklinks,
  resolveEmbeds,
  resolvePageEmbeds,
} from "@outl/shared/api/commands";
import type {
  Backlink,
  PageBacklinks,
  PageMeta,
  PageView,
} from "@outl/shared/api/types";

import { installBacklinksEffect } from "./backlinks-effect";
import { applyPageState } from "./page-view";
import { appState, setAppState } from "./store";

vi.mock("@outl/shared/api/commands", () => ({
  pageBacklinks: vi.fn(),
  resolveEmbeds: vi.fn(),
  resolvePageEmbeds: vi.fn(),
}));

function page(slug: string): PageMeta {
  // A fresh object every call, which is the point: every reply carries
  // one, commits included.
  return { id: `id-${slug}`, slug, title: slug, kind: "page" } as PageMeta;
}

/** What a mutation (`create_block`, `delete_block`, `edit_block`) hands back. */
function view(slug: string): PageView {
  return {
    page: page(slug),
    outline: [],
    backlinks: [],
    backlinks_order: "newest",
    page_properties: [],
    warnings: [],
  } as unknown as PageView;
}

function backlink(id: string): Backlink {
  return {
    block_id: id,
    todo: null,
    ancestors: [],
    source_page: { id: "p", slug: "other", title: "other", kind: "page" },
    source_block: {
      id,
      text: "cite",
      todo: null,
      tokens: [],
      collapsed: false,
      properties: [],
      children: [],
    },
    source_block_path: [],
  } as unknown as Backlink;
}

function reply(backlinks: Backlink[]): PageBacklinks {
  return {
    backlinks,
    backlinks_order: "newest",
    namespace_children: [],
    namespace_backlinks: [],
    namespace_backlinks_total: 0,
  } as unknown as PageBacklinks;
}

let dispose: () => void = () => {};

/**
 * A reply the test holds the resolver for.
 *
 * A bare `new Promise(() => {})` is tempting for "the scan is still
 * running" and is a state production cannot reach: `invoke` always
 * settles, and `trackBacklinks` deliberately starts no second scan
 * while one is in flight — so a scan that never settles parks the whole
 * queue, and the *next* test in this file sees zero calls instead of
 * its own. The `afterEach` below is what makes that fail where it
 * happens rather than one test later.
 */
function deferred(): {
  promise: Promise<PageBacklinks>;
  settle: (r: PageBacklinks) => void;
} {
  let settle: (r: PageBacklinks) => void = () => {};
  const promise = new Promise<PageBacklinks>((resolve) => {
    settle = resolve;
  });
  return { promise, settle };
}

afterEach(async () => {
  // Drain whatever is queued before asserting on the flag: the chain
  // lowers it in its own `finally`, one microtask after the last reply.
  await Promise.resolve();
  // `backlinksLoading` is the chain's own liveness, so this needs no
  // test-only hatch into the module: a test that leaves a scan in
  // flight fails here, under its own name.
  expect(appState.backlinksLoading, "a scan was left in flight").toBe(false);
  dispose();
  dispose = () => {};
  setAppState({ page: null, backlinks: [] as Backlink[] });
  vi.mocked(pageBacklinks).mockReset();
});

function mount(slug: string | null) {
  vi.mocked(resolveEmbeds).mockResolvedValue([] as never);
  vi.mocked(resolvePageEmbeds).mockResolvedValue([] as never);
  setAppState({ page: slug ? page(slug) : null, backlinks: [] as Backlink[] });
  createRoot((d) => {
    dispose = d;
    installBacklinksEffect();
  });
}

describe("installBacklinksEffect", () => {
  it("fetches once for the page that is open", () => {
    vi.mocked(pageBacklinks).mockResolvedValue(reply([]));
    mount("today");

    expect(pageBacklinks).toHaveBeenCalledTimes(1);
    expect(pageBacklinks).toHaveBeenCalledWith("today");
  });

  /**
   * The blink the user reported: creating or deleting a block took the
   * whole backlinks section off screen for the length of a workspace
   * scan, then put it back unchanged.
   *
   * `applyPageState` writes the page as **one object** — the shape
   * `setState` merges key by key — so `page` is set to a new `PageMeta`
   * and its node fires even though the slug did not change.
   * `on()` does not compare values; it only makes the dependency
   * explicit. So the effect re-ran, `emptyBacklinks()` took the rows
   * away, and `lateSectionVisible` unmounted the section until the
   * refetch landed.
   *
   * (A path write — `setAppState("page", …)` — merges into the
   * existing object instead and never showed this, which is why the
   * test has to go through the real `applyPageState`.)
   */
  it("does not refetch when a commit replays the same page", async () => {
    vi.mocked(pageBacklinks).mockResolvedValue(reply([backlink("b1")]));
    mount("today");
    await Promise.resolve();

    applyPageState(view("today"));
    applyPageState(view("today"));

    expect(pageBacklinks).toHaveBeenCalledTimes(1);
  });

  it("keeps the rows on screen across a commit", async () => {
    vi.mocked(pageBacklinks).mockResolvedValue(reply([backlink("b1")]));
    mount("today");
    await Promise.resolve();
    expect(appState.backlinks).toHaveLength(1);

    applyPageState(view("today"));

    // Blanking here is what unmounts the section — the blink itself.
    expect(appState.backlinks).toHaveLength(1);
  });

  it("clears and refetches when the user navigates to another page", async () => {
    vi.mocked(pageBacklinks).mockResolvedValue(reply([backlink("b1")]));
    mount("today");
    await Promise.resolve();

    const running = deferred();
    vi.mocked(pageBacklinks).mockReturnValue(running.promise);
    applyPageState(view("other"));

    // Page A's rows must not sit under page B's title while the scan runs.
    expect(appState.backlinks).toHaveLength(0);
    expect(pageBacklinks).toHaveBeenCalledTimes(2);
    expect(pageBacklinks).toHaveBeenLastCalledWith("other");

    running.settle(reply([]));
  });

  it("refetches the same slug after the page went away (workspace switch)", async () => {
    vi.mocked(pageBacklinks).mockResolvedValue(reply([]));
    mount("today");
    await Promise.resolve();

    // `resetPageState` writes `page: null` before the new workspace's
    // journal lands, and both workspaces call today the same thing.
    setAppState({ page: null });
    applyPageState(view("today"));

    expect(pageBacklinks).toHaveBeenCalledTimes(2);
  });

  /**
   * The half this client never had, inherited from the shared key:
   * `namespace_children` hangs off `title::`, so a rename changes which
   * pages nest under this one while the slug stays put.
   */
  it("refetches when the title changes under the same slug", async () => {
    vi.mocked(pageBacklinks).mockResolvedValue(reply([]));
    mount("os-linux");
    await Promise.resolve();

    const renamed = view("os-linux");
    renamed.page = { ...renamed.page, title: "os/linux" };
    applyPageState(renamed);

    expect(pageBacklinks).toHaveBeenCalledTimes(2);
  });

  it("asks for nothing while no page is open", () => {
    vi.mocked(pageBacklinks).mockResolvedValue(reply([]));
    mount(null);

    expect(pageBacklinks).not.toHaveBeenCalled();
  });
});
