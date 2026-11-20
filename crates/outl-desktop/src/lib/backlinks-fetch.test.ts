/**
 * How many times a burst of commits may ask for the page's backlinks,
 * and which answer is allowed on screen.
 *
 * The cost this pins: `pageBacklinks` is an `O(blocks-in-workspace)`
 * scan, and every cross-page gesture commits with
 * `BacklinkRefresh::DropAll`, so each ask reads and parses every `.md`
 * in the workspace. A **held** `Alt+↑` or `Tab` on a backlink row is one
 * commit per key repeat, which used to be one un-deduped scan per key
 * repeat — latency was a wash either way, and the two were ten whole
 * index rebuilds of CPU apart
 * (`outl-tauri-shared/tests/backlink_commit_cost.rs`).
 *
 * Three properties, and the second and third are why this is not just a
 * call-count test: a coalescer that drops the *last* ask, or that draws
 * an answer for a page the user has left, is a stale-rows bug — worse
 * than slow rows, because nothing on screen says so.
 */
import { createRenderEffect, createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { pageBacklinks } from "@outl/shared/api/commands";
import type {
  Backlink,
  NamespaceChild,
  PageBacklinks,
  PageMeta,
} from "@outl/shared/api/types";

vi.mock("@outl/shared/api/commands", () => ({ pageBacklinks: vi.fn() }));

import { trackBacklinks } from "./backlinks-fetch";
import { appState, setAppState } from "./store";

function page(slug: string, title = slug): PageMeta {
  // A fresh object every call, like every reply's — which is exactly
  // what a by-reference guard would read as "a different page".
  return { id: `id-${slug}`, slug, title, kind: "page" } as PageMeta;
}

function row(id: string): Backlink {
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

function reply(ids: string[]): PageBacklinks {
  return {
    backlinks: ids.map(row),
    backlinks_order: "newest",
    namespace_children: [] as NamespaceChild[],
    namespace_backlinks: [] as Backlink[],
    namespace_backlinks_total: 0,
  } as unknown as PageBacklinks;
}

/** A reply the test holds the resolver for — "the scan is still running". */
function deferred() {
  let settle: (r: PageBacklinks) => void = () => {};
  const promise = new Promise<PageBacklinks>((resolve) => {
    settle = resolve;
  });
  return { promise, settle };
}

/** Drain every pending microtask, so no assertion depends on counting
 *  the `await` hops inside the chain. */
function flush(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

/**
 * Every value `backlinksLoading` took, in order.
 *
 * A **render** effect, not `createEffect`, because that is what the
 * section reads it through: `<Show when={lateSectionVisible(…)}>`
 * compiles to one, so it re-evaluates on the synchronous store write.
 * An `after-the-tick` effect coalesces a flicker inside one tick, which
 * is exactly the flicker worth refusing — the section unmounting and
 * remounting in the same tick is still an unmount, and a
 * `<BacklinkRowEditor />` inside it loses its DOM node, its focus and
 * the draft the user was typing.
 */
function recordLoading(): { values: boolean[]; dispose: () => void } {
  const values: boolean[] = [];
  let dispose = () => {};
  createRoot((d) => {
    dispose = d;
    createRenderEffect(() => values.push(appState.backlinksLoading));
  });
  return { values, dispose };
}

const ids = () => appState.backlinks.map((b) => b.block_id);

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(pageBacklinks).mockResolvedValue(reply([]));
  setAppState({
    page: page("project"),
    backlinks: [] as Backlink[],
    backlinksLoading: false,
  });
});

afterEach(async () => {
  await flush();
  // The chain's own liveness, so this needs no test-only hatch into the
  // module: a test that leaves a scan in flight parks the queue for the
  // next one, and fails here under its own name instead.
  expect(appState.backlinksLoading, "a scan was left in flight").toBe(false);
});

describe("a burst of commits is one scan in flight plus one queued", () => {
  it("puts nine of a held chord's ten repeats on no wire at all", async () => {
    const first = deferred();
    const second = deferred();
    vi.mocked(pageBacklinks)
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);

    const done = trackBacklinks();
    for (let i = 0; i < 9; i++) void trackBacklinks();

    // The scan already running answers for all of them.
    expect(pageBacklinks).toHaveBeenCalledTimes(1);

    first.settle(reply(["from-first-scan"]));
    await flush();

    // **The branch that does something.** A guard that merely refused a
    // second scan would stop here, and the rows would describe the
    // graph as it was before the other nine mutations. Each repeat is a
    // real commit, so the last answer has to be fetched.
    expect(pageBacklinks).toHaveBeenCalledTimes(2);

    second.settle(reply(["from-last-scan"]));
    await done;

    // Ten asks, two scans — and the queue is empty rather than holding
    // eight more.
    expect(pageBacklinks).toHaveBeenCalledTimes(2);
    expect(ids()).toEqual(["from-last-scan"]);
  });

  it("raises the pending mark once for the whole burst, not once per scan", async () => {
    const first = deferred();
    const second = deferred();
    vi.mocked(pageBacklinks)
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);
    setAppState("backlinks", [row("already-here")]);
    const seen = recordLoading();

    const done = trackBacklinks();
    void trackBacklinks();

    first.settle(reply(["already-here"]));
    await flush();
    expect(pageBacklinks).toHaveBeenCalledTimes(2);

    second.settle(reply(["already-here"]));
    await done;
    seen.dispose();

    // One raise, one lower — never `true, false, true, false`. Lowered
    // between the two scans, `lateSectionVisible` reads the section as
    // answered and unmounts it, in the same tick, taking any row editor
    // open inside it with it.
    expect(seen.values).toEqual([false, true, false]);
    // And the rows it already had stay up throughout: nothing here
    // empties them, which is the other way the section vanishes.
    expect(ids()).toEqual(["already-here"]);
  });
});

describe("only the open page's answer reaches the screen", () => {
  it("drops a reply for a page the user navigated away from", async () => {
    const running = deferred();
    vi.mocked(pageBacklinks).mockReturnValueOnce(running.promise);
    setAppState("page", page("page-a"));

    const done = trackBacklinks();
    setAppState("page", page("page-b"));
    running.settle(reply(["cites-page-a"]));
    await done;

    // Page A's rows under page B's title is the stale-rows bug: the
    // rows are navigation affordances, and nothing on screen says they
    // belong to the page the user left.
    expect(ids()).toEqual([]);
  });

  it("drops a reply whose page was renamed under it", async () => {
    const running = deferred();
    vi.mocked(pageBacklinks).mockReturnValueOnce(running.promise);
    setAppState("page", page("os-linux", "os-linux"));

    const done = trackBacklinks();
    // `namespace_children` hang off `title::`, so this reply describes a
    // namespace that no longer exists even though the slug is unchanged.
    setAppState("page", page("os-linux", "os/linux"));
    running.settle(reply(["stale-namespace"]));
    await done;

    expect(ids()).toEqual([]);
  });

  it("installs a reply the open page merely re-published", async () => {
    const running = deferred();
    vi.mocked(pageBacklinks).mockReturnValueOnce(running.promise);
    setAppState("page", page("project"));

    const done = trackBacklinks();
    // Every mutation reply assigns a fresh `PageMeta` with the same slug
    // and title. Compared by reference this reads as a navigation, and
    // the rows of every commit would be thrown away.
    setAppState("page", page("project"));
    running.settle(reply(["still-mine"]));
    await done;

    expect(ids()).toEqual(["still-mine"]);
  });
});

describe("one ask behaves as it did before it was coalesced", () => {
  it("fills every backlinks slice from the reply and lowers the mark", async () => {
    vi.mocked(pageBacklinks).mockResolvedValue({
      backlinks: [row("b1")],
      backlinks_order: "oldest",
      namespace_children: [{ slug: "project/sub", title: "sub" }],
      namespace_backlinks: [row("nested")],
      namespace_backlinks_total: 7,
    } as unknown as PageBacklinks);

    await trackBacklinks();

    expect(ids()).toEqual(["b1"]);
    expect(appState.backlinksOrder).toBe("oldest");
    expect(appState.namespaceChildren).toHaveLength(1);
    expect(appState.namespaceMentions.map((b) => b.block_id)).toEqual(["nested"]);
    expect(appState.namespaceMentionsTotal).toBe(7);
    expect(appState.backlinksLoading).toBe(false);
  });

  it("keeps the rows and clears the mark when a scan fails", async () => {
    setAppState("backlinks", [row("already-here")]);
    vi.mocked(pageBacklinks).mockRejectedValue(new Error("page not found"));

    await trackBacklinks();

    // Swallowed, as it always was: a section that loads forever is the
    // worse failure, and the rows it has beat none.
    expect(appState.backlinksLoading).toBe(false);
    expect(ids()).toEqual(["already-here"]);
  });

  it("runs the queued scan even after the one ahead of it failed", async () => {
    const first = deferred();
    vi.mocked(pageBacklinks)
      .mockReturnValueOnce(first.promise)
      .mockResolvedValueOnce(reply(["from-last-scan"]));

    const done = trackBacklinks();
    void trackBacklinks();
    // A rejection inside the loop may not abandon what is queued behind
    // it — one failed scan would otherwise lose the last mutation's rows
    // with the mark already down.
    first.settle(Promise.reject(new Error("boom")) as unknown as PageBacklinks);
    await done;

    expect(ids()).toEqual(["from-last-scan"]);
  });

  it("asks nothing, and marks nothing, while no page is open", async () => {
    setAppState("page", null);

    const seen = recordLoading();
    await trackBacklinks();
    seen.dispose();

    expect(pageBacklinks).not.toHaveBeenCalled();
    // Not merely `false` by the time anyone awaits — never raised at
    // all. Starting a chain that immediately finds an empty queue
    // raises and lowers it inside one synchronous block, which is
    // invisible to an assertion after the fact and visible to the
    // `<Show>` that reads it.
    expect(seen.values).toEqual([false]);
  });
});
