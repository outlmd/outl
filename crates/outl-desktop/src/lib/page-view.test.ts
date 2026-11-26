import { readFileSync } from "node:fs";
import { join } from "node:path";

import { afterEach, describe, expect, it, vi } from "vitest";

import type { MdAheadOfLog, PageMeta, PageView } from "@outl/shared/api/types";

import { resolveEmbeds } from "@outl/shared/api/commands";

import { applyPageState, resolveBacklinkTransclusions } from "./page-view";
import { appState, setAppState, setOutline } from "./store";

vi.mock("@outl/shared/api/commands", () => ({
  resolveEmbeds: vi.fn(() => Promise.resolve({})),
  resolvePageEmbeds: vi.fn(() => Promise.resolve({})),
}));

function page(id: string): PageMeta {
  return { id, slug: `slug-${id}`, kind: "page" } as unknown as PageMeta;
}

function view(id: string, extra: Partial<PageView> = {}): PageView {
  return {
    page: page(id),
    outline: [],
    backlinks: [],
    backlinks_order: "newest",
    ...extra,
  } as unknown as PageView;
}

const ahead: MdAheadOfLog = {
  lines: 3,
  sample: ["a line the log never saw"],
} as unknown as MdAheadOfLog;

describe("applyPageState", () => {
  afterEach(() => {
    setAppState({
      page: null,
      pageProperties: [],
      parseWarnings: [],
      mdAheadOfLog: undefined,
      lastError: null,
    } as never);
    setOutline([]);
  });

  it("drops the previous page's properties when another page lands", () => {
    applyPageState(view("p1", { page_properties: [["type", "person"]] }));
    expect(appState.pageProperties).toEqual([["type", "person"]]);

    applyPageState(view("p2"));
    expect(appState.pageProperties).toEqual([]);
  });

  it("drops the previous page's parse warnings when another page lands", () => {
    applyPageState(
      view("p1", {
        warnings: [{ line: 1, raw: "# heading", kind: "unrecognized_block_marker" }],
      } as Partial<PageView>),
    );
    expect(appState.parseWarnings).toHaveLength(1);

    applyPageState(view("p2"));
    expect(appState.parseWarnings).toEqual([]);
  });

  it("keeps the ahead-of-log notice across a reply about the same page", () => {
    applyPageState(view("p1", { md_ahead_of_log: ahead } as Partial<PageView>));
    expect(appState.mdAheadOfLog).toEqual(ahead);

    // A mutation reply never carries the flag — the banner must survive it.
    applyPageState(view("p1"));
    expect(appState.mdAheadOfLog).toEqual(ahead);
  });

  it("drops the ahead-of-log notice when another page lands", () => {
    applyPageState(view("p1", { md_ahead_of_log: ahead } as Partial<PageView>));
    applyPageState(view("p2"));
    expect(appState.mdAheadOfLog).toBeUndefined();
  });

  it("clears the notice once a reply that ran the check says the page is healthy", () => {
    applyPageState(view("p1", { md_ahead_of_log: ahead } as Partial<PageView>));
    applyPageState(view("p1", { md_ahead_of_log_checked: true } as Partial<PageView>));
    expect(appState.mdAheadOfLog).toBeUndefined();
  });

  it("surfaces a projection error", () => {
    applyPageState(view("p1", { projection_error: "disk is full" } as Partial<PageView>));
    expect(appState.lastError).toBe("disk is full");
  });
});

/**
 * The regression this module exists to end: the shell had a second
 * `applyView` that wrote `page` and the outline and **nothing else**, so
 * every page opened from the sidebar, the picker, Today, a task row or a
 * deep link inherited the previous page's property chips, parse warnings
 * and "this page isn't syncing" banner.
 *
 * Read from the source rather than rendered: both callers live inside
 * components that need a Tauri bridge to mount, and the defect is in
 * which function they call, not in what the component draws.
 */
describe("one owner for a page view's state", () => {
  function source(file: string): string {
    return readFileSync(join(import.meta.dirname, "..", file), "utf8");
  }

  // `OutlineView.tsx`'s `applyView` moved to `lib/outline-navigation.ts`,
  // shared with the backlink rows, so that file is the writer it reaches.
  it.each(["components/AppShell.tsx", "lib/outline-navigation.ts"])(
    "%s applies a page view through applyPageState",
    (file) => {
      expect(source(file)).toContain("applyPageState(");
    },
  );

  it("OutlineView.tsx applies its replies through the shared applyView", () => {
    expect(source("components/OutlineView.tsx")).toMatch(
      /import \{[^}]*\bapplyView\b[^}]*\} from "\.\.\/lib\/outline-navigation"/,
    );
  });

  it.each([
    "components/AppShell.tsx",
    "components/OutlineView.tsx",
    "lib/outline-navigation.ts",
  ])("%s writes none of a page view's fields itself", (file) => {
    const src = source(file);
    // A second writer of any of these is the bug coming back.
    expect(src).not.toMatch(/\bpageProperties:/);
    expect(src).not.toMatch(/\bparseWarnings:/);
    expect(src).not.toMatch(/\bmdAheadOfLog:/);
  });
});

describe("embed maps", () => {
  // The merge exists so a page resolve cannot wipe the "Linked from" rows'
  // handles. It must not turn a deleted source into an immortal entry.
  it("drops a handle that was asked for and not answered", async () => {
    const node = {
      id: "n",
      text: "",
      todo: null,
      tokens: [{ kind: "embed", value: "blk-aaaaaa" }],
      collapsed: false,
      properties: [],
      children: [],
    } as unknown as import("@outl/shared/api/types").BlockNode;

    setAppState("embeds", {
      "blk-aaaaaa": { text: "old text" },
      "blk-keep01": { text: "kept" },
    } as never);
    vi.mocked(resolveEmbeds).mockResolvedValueOnce({});
    resolveBacklinkTransclusions([node]);
    await vi.waitFor(() => expect(appState.embeds["blk-aaaaaa"]).toBeUndefined());

    expect(appState.embeds["blk-keep01"]).toBeDefined();
  });
});
