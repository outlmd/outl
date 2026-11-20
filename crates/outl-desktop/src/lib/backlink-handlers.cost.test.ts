/**
 * What a **burst** of gestures on a backlink row costs.
 *
 * Its own sibling of `backlink-handlers.test.ts` for the reason
 * `backlink-commands.test.ts` is one: a different question. That file
 * asks what a keystroke on a row *targets* — the wrong page, the wrong
 * screen, silence — and this one asks what ten of them in a row *spend*,
 * which is root [`CLAUDE.md`](../../../../CLAUDE.md) invariant 15's
 * third test: **is it cheap once.** It has to be asked of the call site
 * rather than of the function, because a signature cannot bound how
 * many times a caller asks.
 *
 * "Once per commit" is not a bound on a **held** key. `Alt+↑` and `Tab`
 * repeat, every repeat is a real commit, and every commit drops the
 * cached backlink index (`BacklinkRefresh::DropAll`), so each ask reads
 * and parses every `.md` in the workspace. Ten repeats used to be ten of
 * those: latency was a wash (30.0 / 34.1 ms per keystroke either way)
 * and the two were ten whole index rebuilds of CPU apart
 * (`outl-tauri-shared/tests/backlink_commit_cost.rs` carries the numbers
 * and why the refresh policy is not the thing to change).
 *
 * Driven through the real `backlinkRowOps`, never through
 * `trackBacklinks`: what this pins is that the gesture inherits the
 * bound *without counting anything itself*, which is the reason the
 * dedupe lives with the one owner of the fetch and not at the five call
 * sites. `backlinks-fetch.test.ts` is the other half — what the owner
 * does with the queue, and which answer it lets through.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Backlink, PageBacklinks, PageView } from "@outl/shared/api/types";

vi.mock("@outl/shared/api/commands", () => ({
  backlinkRowOp: vi.fn(),
  createBacklinkSibling: vi.fn(),
  pageBacklinks: vi.fn(),
}));

import { backlinkRowOp, pageBacklinks } from "@outl/shared/api/commands";

import { backlinkRowOps } from "./backlink-handlers";
import type { HandlerCtx } from "./handler-context";
import { appState, setAppState } from "./store";

const SOURCE = { id: "pg-source", slug: "source", title: "Source", kind: "page" } as const;

function block(id: string) {
  return {
    id,
    text: "cites [[today]]",
    todo: null,
    tokens: [],
    collapsed: false,
    properties: [],
    children: [],
  };
}

/** The reply of a mutation sent to the source page. */
function sourceView(): PageView {
  return {
    page: { ...SOURCE },
    outline: [block("blk-a")],
    backlinks: [],
    backlinks_order: "newest",
  } as unknown as PageView;
}

function row(id: string): Backlink {
  return {
    block_id: id,
    todo: null,
    source_page: { ...SOURCE },
    source_block: block(id),
    source_block_path: [0],
    ancestors: [],
  } as unknown as Backlink;
}

function backlinksReply(rows: Backlink[]): PageBacklinks {
  return {
    backlinks: rows,
    backlinks_order: "newest",
    namespace_backlinks: [] as Backlink[],
    namespace_backlinks_total: 0,
    namespace_children: [],
  } as unknown as PageBacklinks;
}

/** Only what `backlinkRowOps` reads: the reply sink, the error sink and
 *  `safeCall`. The rest of `HandlerCtx` is the outline's. */
function ctx(): HandlerCtx {
  const deps = { applyView: vi.fn(), setError: vi.fn() };
  return {
    deps,
    safeCall: async <T>(p: Promise<T>) => {
      try {
        return await p;
      } catch (e) {
        deps.setError(e instanceof Error ? e.message : String(e));
        return undefined;
      }
    },
  } as unknown as HandlerCtx;
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(backlinkRowOp).mockResolvedValue(sourceView());
  vi.mocked(pageBacklinks).mockResolvedValue(backlinksReply([row("blk-a")]));
  setAppState({
    page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
    backlinks: [row("blk-a"), row("blk-b")],
    backlinksOpen: true,
    backlinksLoading: false,
    // `backlinkRowIds()` reads these: a zoom or an outline cursor left
    // behind takes the cursor out of the section for every test after.
    focusBlockId: null,
    selectedBlockId: null,
    selectedBacklinkBlockId: "blk-a",
  });
});

describe("a held chord on a backlink row", () => {
  it("commits ten times and asks for the rows twice", async () => {
    // The scan a real workspace takes ~280 ms over, against a ~30 ms key
    // repeat: in flight for the whole burst.
    let settleScan: (r: PageBacklinks) => void = () => {};
    const running = new Promise<PageBacklinks>((resolve) => {
      settleScan = resolve;
    });
    vi.mocked(pageBacklinks)
      .mockReturnValueOnce(running)
      .mockResolvedValue(backlinksReply([row("blk-a")]));

    const ops = backlinkRowOps(ctx());
    await Promise.all(Array.from({ length: 10 }, () => ops.run("move-up")));

    // The mutation itself is never coalesced — only the question about
    // what cites this page.
    expect(backlinkRowOp).toHaveBeenCalledTimes(10);
    expect(pageBacklinks).toHaveBeenCalledTimes(1);

    settleScan(backlinksReply([row("blk-a")]));
    await new Promise((resolve) => setTimeout(resolve, 0));

    // The trailing ask runs, so the last repeat's rows are fetched
    // rather than dropped, and the queue is empty rather than holding
    // eight more.
    expect(pageBacklinks).toHaveBeenCalledTimes(2);
    expect(appState.backlinksLoading).toBe(false);
  });

  it("asks only about the page on screen, however fast the repeats land", async () => {
    const ops = backlinkRowOps(ctx());
    await Promise.all(Array.from({ length: 4 }, () => ops.run("toggle-todo")));
    await new Promise((resolve) => setTimeout(resolve, 0));

    // Never the source page. A gesture aimed at another page's block
    // must not re-point this section at that page's backlinks.
    for (const call of vi.mocked(pageBacklinks).mock.calls) {
      expect(call).toEqual(["today"]);
    }
  });
});
