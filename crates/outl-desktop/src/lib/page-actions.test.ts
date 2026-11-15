/**
 * What rename and merge owe the user: a taken name asks before it
 * merges, a merge always asks, declining writes nothing, and what a
 * merge dropped (property conflicts) is on screen afterwards.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { mergePages, renamePage } from "@outl/shared/api/page-merge";
import type { PageMeta, PageView } from "@outl/shared/api/types";

import { mergePageFlow, renamePageFlow } from "./page-actions";
import { appState, setAppState } from "./store";

vi.mock("@outl/shared/api/page-merge", () => ({
  renamePage: vi.fn(),
  mergePages: vi.fn(),
}));

const meta = (slug: string, title = slug): PageMeta => ({
  id: `page-${slug}`,
  slug,
  title,
  kind: "page",
});

const view = (slug: string): PageView => ({
  page: meta(slug),
  outline: [],
  backlinks: [],
  backlinks_order: "newest",
});

const mergeReport = (over: Record<string, unknown> = {}) => ({
  source_slug: "a",
  source_title: "A",
  target_slug: "b",
  target_title: "B",
  moved_blocks: 3,
  rewritten_blocks: 1,
  skipped_namespace_refs: 0,
  property_conflicts: [],
  aliases_added: [],
  already_merged: false,
  ...over,
});

let confirm: ReturnType<typeof vi.fn>;

beforeEach(() => {
  confirm = vi.fn();
  vi.stubGlobal("confirm", confirm);
  setAppState({ lastError: null, notice: null });
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

describe("renamePageFlow", () => {
  it("renames without asking when the name is free", async () => {
    vi.mocked(renamePage).mockResolvedValue({
      status: "renamed",
      view: view("new-name"),
      report: { old_slug: "a", old_title: "A", rewritten_blocks: 2, skipped_namespace_refs: 0 },
    });
    const out = await renamePageFlow(meta("a", "A"), "  New name ");
    expect(out?.page.slug).toBe("new-name");
    expect(confirm).not.toHaveBeenCalled();
    expect(renamePage).toHaveBeenCalledWith("a", "New name");
    expect(appState.notice?.join(" ")).toMatch(/Renamed/);
  });

  it("asks before merging when the name is taken, and merges on accept", async () => {
    vi.mocked(renamePage).mockResolvedValue({ status: "exists", existing: meta("b", "B") });
    vi.mocked(mergePages).mockResolvedValue({ view: view("b"), report: mergeReport() });
    confirm.mockReturnValue(true);
    const out = await renamePageFlow(meta("a", "A"), "B");
    expect(confirm).toHaveBeenCalledTimes(1);
    expect(confirm.mock.calls[0][0]).toContain("B");
    expect(mergePages).toHaveBeenCalledWith("a", "b");
    expect(out?.page.slug).toBe("b");
  });

  it("writes nothing when the user declines the merge", async () => {
    vi.mocked(renamePage).mockResolvedValue({ status: "exists", existing: meta("b", "B") });
    confirm.mockReturnValue(false);
    const out = await renamePageFlow(meta("a", "A"), "B");
    expect(out).toBeNull();
    expect(mergePages).not.toHaveBeenCalled();
    expect(appState.notice).toBeNull();
  });

  it("surfaces a backend refusal as an error and navigates nowhere", async () => {
    vi.mocked(renamePage).mockRejectedValue(new Error("cannot rename a journal"));
    const out = await renamePageFlow(meta("a", "A"), "X");
    expect(out).toBeNull();
    expect(appState.lastError).toBe("cannot rename a journal");
  });
});

describe("mergePageFlow", () => {
  it("always confirms, and a decline calls no write command", async () => {
    confirm.mockReturnValue(false);
    const out = await mergePageFlow(meta("a", "A"), meta("b", "B"));
    expect(out).toBeNull();
    expect(confirm).toHaveBeenCalledTimes(1);
    expect(mergePages).not.toHaveBeenCalled();
  });

  it("merges after the confirm and shows the property conflicts", async () => {
    confirm.mockReturnValue(true);
    vi.mocked(mergePages).mockResolvedValue({
      view: view("b"),
      report: mergeReport({
        property_conflicts: [{ key: "status", kept: "open", dropped: "done" }],
      }),
    });
    const out = await mergePageFlow(meta("a", "A"), meta("b", "B"));
    expect(mergePages).toHaveBeenCalledWith("a", "b");
    expect(out?.page.slug).toBe("b");
    const shown = appState.notice?.join("\n") ?? "";
    expect(shown).toContain("status");
    expect(shown).toContain("open");
    expect(shown).toContain("done");
  });

  it("surfaces a backend refusal as an error", async () => {
    confirm.mockReturnValue(true);
    vi.mocked(mergePages).mockRejectedValue(new Error("cannot merge a page into itself"));
    const out = await mergePageFlow(meta("a", "A"), meta("b", "B"));
    expect(out).toBeNull();
    expect(appState.lastError).toBe("cannot merge a page into itself");
  });
});
