import { beforeEach, describe, expect, it, vi } from "vitest";

import type { MergePagesReport, RenamePageReport } from "../api/page-merge";
import type { PageMeta, PageView } from "../api/types";

const renamePage = vi.fn();
const mergePages = vi.fn();
vi.mock("../api/page-merge", () => ({
  renamePage: (...a: unknown[]) => renamePage(...a),
  mergePages: (...a: unknown[]) => mergePages(...a),
}));

import {
  mergeConfirmMessage,
  mergeInto,
  mergeSummary,
  nameTakenMessage,
  renameOrMerge,
  renameSummary,
} from "./rename-merge";

const meta = (slug: string, title = slug): PageMeta => ({ id: `id-${slug}`, slug, title, kind: "page" });
const view = (slug: string): PageView => ({
  page: meta(slug),
  outline: [],
  backlinks: [],
  backlinks_order: "newest",
  page_properties: [],
});

const renameReport = (over: Partial<RenamePageReport> = {}): RenamePageReport => ({
  old_slug: "old",
  old_title: "Old",
  rewritten_blocks: 0,
  skipped_namespace_refs: 0,
  ...over,
});

const mergeReport = (over: Partial<MergePagesReport> = {}): MergePagesReport => ({
  source_slug: "a",
  source_title: "A",
  target_slug: "b",
  target_title: "B",
  moved_blocks: 0,
  rewritten_blocks: 0,
  skipped_namespace_refs: 0,
  property_conflicts: [],
  aliases_added: [],
  already_merged: false,
  ...over,
});

beforeEach(() => {
  renamePage.mockReset();
  mergePages.mockReset();
});

describe("renameOrMerge", () => {
  it("returns the renamed page without asking anything", async () => {
    const report = renameReport({ rewritten_blocks: 3 });
    renamePage.mockResolvedValue({ status: "renamed", view: view("new"), report });
    const confirm = vi.fn();

    const out = await renameOrMerge("old", "New", confirm);

    expect(renamePage).toHaveBeenCalledWith("old", "New");
    expect(out).toEqual({ kind: "renamed", view: view("new"), report });
    expect(confirm).not.toHaveBeenCalled();
    expect(mergePages).not.toHaveBeenCalled();
  });

  it("merges into the existing page when the name is taken and the user accepts", async () => {
    const existing = meta("taken", "Taken");
    const report = mergeReport({ moved_blocks: 2 });
    renamePage.mockResolvedValue({ status: "exists", existing });
    mergePages.mockResolvedValue({ view: view("taken"), report });
    const confirm = vi.fn().mockResolvedValue(true);

    const out = await renameOrMerge("page-a", "Taken", confirm);

    expect(confirm).toHaveBeenCalledWith(existing);
    expect(mergePages).toHaveBeenCalledWith("page-a", "taken");
    expect(out).toEqual({ kind: "merged", view: view("taken"), report });
  });

  it("cancels without merging when the user declines", async () => {
    renamePage.mockResolvedValue({ status: "exists", existing: meta("taken") });
    const confirm = vi.fn().mockResolvedValue(false);

    const out = await renameOrMerge("page-a", "Taken", confirm);

    expect(out).toEqual({ kind: "cancelled" });
    expect(mergePages).not.toHaveBeenCalled();
  });

  it("propagates a rename error", async () => {
    renamePage.mockRejectedValue("journal pages cannot be renamed");
    const confirm = vi.fn();

    await expect(renameOrMerge("2026-01-01", "x", confirm)).rejects.toBe("journal pages cannot be renamed");
    expect(confirm).not.toHaveBeenCalled();
  });

  it("propagates a merge error after the user accepted", async () => {
    renamePage.mockResolvedValue({ status: "exists", existing: meta("taken") });
    mergePages.mockRejectedValue("cannot merge a page into itself");

    await expect(renameOrMerge("page-a", "Taken", async () => true)).rejects.toBe(
      "cannot merge a page into itself",
    );
  });

  it("propagates an error thrown by the confirmation", async () => {
    renamePage.mockResolvedValue({ status: "exists", existing: meta("taken") });

    await expect(
      renameOrMerge("page-a", "Taken", async () => {
        throw new Error("dialog failed");
      }),
    ).rejects.toThrow("dialog failed");
    expect(mergePages).not.toHaveBeenCalled();
  });
});

describe("mergeInto", () => {
  it("wraps the merge reply", async () => {
    const report = mergeReport();
    mergePages.mockResolvedValue({ view: view("b"), report });

    expect(await mergeInto("a", "b")).toEqual({ kind: "merged", view: view("b"), report });
    expect(mergePages).toHaveBeenCalledWith("a", "b");
  });

  it("propagates errors", async () => {
    mergePages.mockRejectedValue("no such page");
    await expect(mergeInto("a", "b")).rejects.toBe("no such page");
  });
});

describe("wording", () => {
  it("asks before every merge, naming both pages", () => {
    const msg = mergeConfirmMessage("Alpha", "Beta");
    expect(msg).toContain('Merge "Alpha" into "Beta"?');
    expect(msg).toContain("cannot be undone");
  });

  it("asks the name-taken question", () => {
    expect(nameTakenMessage("Ideas")).toBe(
      'A page named "Ideas" already exists. Merge this page into "Ideas"?',
    );
  });

  it("pluralises the rename summary", () => {
    expect(renameSummary(renameReport({ rewritten_blocks: 0 }))).toBe("Renamed. 0 links updated.");
    expect(renameSummary(renameReport({ rewritten_blocks: 1 }))).toBe("Renamed. 1 link updated.");
    expect(renameSummary(renameReport({ rewritten_blocks: 2 }))).toBe("Renamed. 2 links updated.");
  });

  it("mentions skipped namespace links in a rename only when there are some", () => {
    expect(renameSummary(renameReport({ skipped_namespace_refs: 1 }))).toBe(
      "Renamed. 0 links updated. 1 namespace link left unchanged.",
    );
    expect(renameSummary(renameReport({ skipped_namespace_refs: 3 }))).toContain("3 namespace links left unchanged");
    expect(renameSummary(renameReport())).not.toContain("namespace");
  });

  it("summarises a plain merge in two lines", () => {
    expect(mergeSummary(mergeReport({ moved_blocks: 1, rewritten_blocks: 1 }))).toEqual([
      '1 block moved to "B".',
      "1 link updated.",
    ]);
    expect(mergeSummary(mergeReport({ moved_blocks: 4, rewritten_blocks: 2 }))).toEqual([
      '4 blocks moved to "B".',
      "2 links updated.",
    ]);
  });

  it("lists aliases added, property conflicts and skipped namespace links when present", () => {
    const lines = mergeSummary(
      mergeReport({
        moved_blocks: 2,
        rewritten_blocks: 5,
        aliases_added: ["A"],
        property_conflicts: [
          { key: "status", kept: "open", dropped: "done" },
          { key: "owner", kept: "Ana", dropped: "Rui" },
        ],
        skipped_namespace_refs: 2,
      }),
    );
    expect(lines).toEqual([
      '2 blocks moved to "B".',
      "5 links updated.",
      '1 alias added: "A".',
      'Property conflict: kept "open", dropped "done" (status).',
      'Property conflict: kept "Ana", dropped "Rui" (owner).',
      "2 namespace links left unchanged.",
    ]);
  });

  it("pluralises the alias line", () => {
    const lines = mergeSummary(mergeReport({ aliases_added: ["A", "A2"] }));
    expect(lines).toContain('2 aliases added: "A", "A2".');
  });

  it("says so when a retry found nothing left to move", () => {
    const lines = mergeSummary(mergeReport({ already_merged: true }));
    expect(lines[0]).toBe('Already merged into "B".');
  });

  it("uses no em or en dashes in any sentence", () => {
    const all = [
      mergeConfirmMessage("A", "B"),
      nameTakenMessage("X"),
      renameSummary(renameReport({ skipped_namespace_refs: 2 })),
      ...mergeSummary(
        mergeReport({
          aliases_added: ["A"],
          property_conflicts: [{ key: "k", kept: "1", dropped: "2" }],
          skipped_namespace_refs: 1,
        }),
      ),
    ].join("\n");
    expect(all).not.toMatch(/[\u2013\u2014]/);
  });
});
