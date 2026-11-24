/**
 * The host half of editing a backlink: what happens to the card once a
 * commit on another page has landed.
 *
 * The rule under test is that the rows are **swapped in place**. The
 * card is fed by an `O(blocks-in-workspace)` scan and `Journal` hands
 * `undefined` down while the resource is loading, so re-running the
 * resource would unmount the section the user just typed into for the
 * length of that scan — the one thing a commit must not do to the
 * section it commits from.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { PageBacklinks, PageMeta, PageView } from "@outl/shared/api/types";

import { createBacklinkCard } from "./Journal.backlinks";

vi.mock("../lib/haptics", () => ({ haptic: vi.fn() }));

const commands = vi.hoisted(() => ({
  pageBacklinks: vi.fn(),
  setBacklinksOrder: vi.fn(),
}));
vi.mock("@outl/shared/api/commands", () => commands);

const reply = { backlinks: [], backlinks_order: "newest" } as unknown as PageBacklinks;

function card(
  over: {
    order?: "newest" | "oldest";
    view?: PageView | null;
    openPageId?: string;
  } = {},
) {
  const mutate = vi.fn();
  const onError = vi.fn();
  const onNotice = vi.fn();
  const applyView = vi.fn();
  const api = createBacklinkCard({
    view: () =>
      over.view === undefined
        ? ({
            page: { id: over.openPageId ?? "open-page", slug: "journal" },
          } as unknown as PageView)
        : over.view,
    order: () => over.order ?? "newest",
    mutate,
    applyView,
    onError,
    onNotice,
  });
  return { api, mutate, applyView, onError, onNotice };
}

/** What a mutation through a backlink row reports back. */
const commit = (sourcePageId = "p1") => ({
  sourcePageId,
  view: { page: { id: sourcePageId, slug: "other" } } as unknown as PageView,
});

const tick = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  vi.clearAllMocks();
});

describe("the backlinks card after a commit through one of its rows", () => {
  it("swaps fresh rows in rather than re-running the resource", async () => {
    commands.pageBacklinks.mockResolvedValue(reply);
    const c = card();

    await c.api.committed(commit());

    expect(commands.pageBacklinks).toHaveBeenCalledWith("journal");
    expect(c.mutate).toHaveBeenCalledWith(reply);
  });

  it("says so when the re-ask fails", async () => {
    commands.pageBacklinks.mockRejectedValue(new Error("workspace not loaded"));
    const c = card();

    await c.api.committed(commit());

    // The write landed, so the card now describes the graph as it was
    // before it. Stale rows with nothing said is the defect.
    expect(c.mutate).not.toHaveBeenCalled();
    expect(c.onError).toHaveBeenCalledWith("workspace not loaded");
  });

  it("asks for nothing while no page is open", async () => {
    const c = card({ view: null });

    await c.api.committed(commit());

    expect(commands.pageBacklinks).not.toHaveBeenCalled();
  });

  it("resolves only once the fresh rows are in", async () => {
    let landScan: (r: PageBacklinks) => void = () => {};
    commands.pageBacklinks.mockReturnValue(
      new Promise<PageBacklinks>((resolve) => (landScan = resolve)),
    );
    const c = card();
    let settled = false;

    void c.api.committed(commit()).then(() => (settled = true));
    await tick();

    // The section awaits this to decide when to close its editor. A
    // fire-and-forget `committed` is what let the row render its
    // pre-edit tokens in the gap.
    expect(settled).toBe(false);
    landScan(reply);
    await tick();
    expect(settled).toBe(true);
  });

  it("clears a stale refusal once a re-ask succeeds", async () => {
    commands.pageBacklinks.mockResolvedValue(reply);
    const c = card();

    await c.api.committed(commit());

    // `withError` clears the previous message before it runs, and the
    // toast this feeds is the same one. Without it a refusal outlives
    // every later success.
    expect(c.onError).toHaveBeenCalledWith(null);
  });
});

/**
 * Whether a cross-page reply may be installed as the open page's view
 * is `backlinkCommitPlan`'s answer, and this is the only place that
 * holds both page ids — which is why the section reports the fact and
 * decides nothing about it.
 */
describe("what a cross-page reply is allowed to touch", () => {
  it("does not install a reply describing another page", async () => {
    commands.pageBacklinks.mockResolvedValue(reply);
    const c = card({ openPageId: "open-page" });

    await c.api.committed(commit("p1"));

    // Installing it would replace the outline the user is reading
    // with someone else's.
    expect(c.applyView).not.toHaveBeenCalled();
    expect(c.mutate).toHaveBeenCalledWith(reply);
  });

  it("installs it when the backlink cites the open page", async () => {
    commands.pageBacklinks.mockResolvedValue(reply);
    const c = card({ openPageId: "p1" });
    const landed = commit("p1");

    await c.api.committed(landed);

    // A page can cite itself, and discarding the reply there drops a
    // genuine refresh of the outline.
    expect(c.applyView).toHaveBeenCalledWith(landed.view);
  });
});

/**
 * A block created through a backlink row cites nothing, so it is a
 * backlink of no page and no row here will ever render it. Saying where
 * it went is the whole affordance — but only when it went somewhere
 * else, which is why the sentence is composed here rather than in the
 * section: this is the only place that knows both page ids.
 */
describe("saying where a created block landed", () => {
  const created = (sourcePageId: string) => ({
    ...commit(sourcePageId),
    created: {
      page: { id: sourcePageId, title: "Monday" } as PageMeta,
      side: "below" as const,
      depth: 0,
    },
  });

  it("names the page and the route back to it", async () => {
    commands.pageBacklinks.mockResolvedValue(reply);
    const c = card({ openPageId: "open-page" });

    await c.api.committed(created("p1"));

    expect(c.onNotice).toHaveBeenCalledWith(
      expect.stringContaining("Monday"),
    );
    expect(c.onNotice).toHaveBeenCalledWith(
      expect.stringContaining("to type in it"),
    );
  });

  it("says nothing when the page cites itself", async () => {
    commands.pageBacklinks.mockResolvedValue(reply);
    const c = card({ openPageId: "p1" });

    await c.api.committed(created("p1"));

    // The new block is in the outline on screen, so "open Monday to
    // type in it" would be a lie — the user is already there.
    expect(c.onNotice).not.toHaveBeenCalled();
    expect(c.applyView).toHaveBeenCalled();
  });

  it("stays quiet for a mutation that created nothing", async () => {
    commands.pageBacklinks.mockResolvedValue(reply);
    const c = card();

    await c.api.committed(commit("p1"));

    expect(c.onNotice).not.toHaveBeenCalled();
  });
});

/**
 * **Both callbacks land on one signal in production**, and that is the
 * thing under test here.
 *
 * `Journal.tsx` wires `onError: (m) => setError(m)` *and*
 * `onNotice: (m) => setError(m)` — one toast is this client's whole
 * message channel. So a test that hands them two separate spies has
 * tested a system that does not ship: it cannot see `reload`'s
 * `onError(null)` (which clears a stale refusal before the re-ask, and
 * runs synchronously, before its first `await`) wiping a notice written
 * a line earlier, in the same tick, before the toast ever renders.
 *
 * Third time on this branch a test passed by mocking apart the thing it
 * was checking. The shape to copy is this one: mirror the wiring, then
 * assert on what the user would actually be left looking at.
 */
describe("the notice survives the channel it shares with refusals", () => {
  /** One signal, both callbacks — exactly `Journal.tsx`'s wiring. */
  function sharedToastCard(openPageId: string) {
    let toast: string | null = null;
    const api = createBacklinkCard({
      view: () =>
        ({ page: { id: openPageId, slug: "journal" } }) as unknown as PageView,
      order: () => "newest",
      mutate: vi.fn(),
      applyView: vi.fn(),
      onError: (m) => (toast = m),
      onNotice: (m) => (toast = m),
    });
    return { api, toast: () => toast };
  }

  const created = {
    sourcePageId: "p1",
    view: {} as unknown as PageView,
    created: {
      page: { id: "p1", title: "Monday" } as PageMeta,
      side: "below" as const,
      depth: 0,
    },
  };

  it("is still on screen once committed resolves", async () => {
    commands.pageBacklinks.mockResolvedValue(reply);
    const c = sharedToastCard("open-page");

    await c.api.committed(created);

    // The create happened on another page and nothing here will ever
    // show the new block, so this sentence is the entire affordance.
    expect(c.toast()).toContain("Monday");
  });

  it("yields to a refusal from the re-ask, which is the more urgent one", async () => {
    commands.pageBacklinks.mockRejectedValue(new Error("workspace not loaded"));
    const c = sharedToastCard("open-page");

    await c.api.committed(created);

    // One channel, so one message: the card is now stale, which the
    // user has to know before they trust the rows in front of them.
    expect(c.toast()).toBe("workspace not loaded");
  });
});

describe("the card's direction toggle", () => {
  it("flips newest to oldest and keeps the rows up", async () => {
    commands.setBacklinksOrder.mockResolvedValue(reply);
    const c = card({ order: "newest" });

    c.api.toggleOrder();
    await tick();

    expect(commands.setBacklinksOrder).toHaveBeenCalledWith(
      "oldest",
      "journal",
    );
    expect(c.mutate).toHaveBeenCalledWith(reply);
  });

  it("flips oldest back to newest", async () => {
    commands.setBacklinksOrder.mockResolvedValue(reply);
    const c = card({ order: "oldest" });

    c.api.toggleOrder();
    await tick();

    expect(commands.setBacklinksOrder).toHaveBeenCalledWith(
      "newest",
      "journal",
    );
  });
});

describe("the direction toggle", () => {
  it("resolves only once the re-sorted rows are in", async () => {
    // The card holds its one-write guard for as long as this promise, so
    // a toggle that returns early releases it mid re-ask.
    let land: (r: PageBacklinks) => void = () => {};
    commands.setBacklinksOrder.mockImplementation(
      () => new Promise<PageBacklinks>((resolve) => (land = resolve)),
    );
    const c = card();
    let settled = false;
    const done = Promise.resolve(c.api.toggleOrder()).then(() => {
      settled = true;
    });
    await tick();
    expect(settled).toBe(false);
    expect(c.mutate).not.toHaveBeenCalled();

    land(reply);
    await done;
    expect(c.mutate).toHaveBeenCalledWith(reply);
  });
});
