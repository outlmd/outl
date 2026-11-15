import { describe, expect, it } from "vitest";
import type { PageView } from "../api/types";
import { createMockBackend, MockWorkspace } from "./index";
import { wantsMock } from "./index";

const today = new Date(2026, 9, 11);
const fresh = () => createMockBackend(new MockWorkspace(today), today);

describe("mock backend", () => {
  it("opens today's journal with fixture blocks", () => {
    const view = fresh()("open_today_journal") as PageView;
    expect(view.page.slug).toBe("2026-10-11");
    expect(view.page.kind).toBe("journal");
    expect(view.outline.length).toBeGreaterThan(0);
  });

  it("creates an empty journal for a day it has never seen", () => {
    const view = fresh()("open_journal_for", { slug: "2026-01-02" }) as PageView;
    expect(view.page.slug).toBe("2026-01-02");
  });

  it("edits a block and retokenizes it", () => {
    const call = fresh();
    const view = call("open_today_journal") as PageView;
    const id = view.outline[0].id;
    const next = call("edit_block", { pageId: view.page.id, id, text: "TODO ping [[outl]]" }) as PageView;
    expect(next.outline[0].todo).toBe("TODO");
    expect(next.outline[0].tokens).toContainEqual({ kind: "ref", value: "outl" });
  });

  it("finds backlinks from refs and tags across pages", () => {
    const reply = fresh()("page_backlinks", { slug: "outl" }) as { backlinks: unknown[] };
    expect(reply.backlinks.length).toBe(2);
  });

  it("answers null for a command it does not know", () => {
    expect(fresh()("not_a_command")).toBeNull();
  });

  it("renames a page, and answers exists when the name is taken", () => {
    const call = fresh();
    const renamed = call("rename_page", { page: "design", newName: "Visual System" }) as {
      status: string;
      view: PageView;
    };
    expect(renamed.status).toBe("renamed");
    expect(renamed.view.page.slug).toBe("visual-system");
    expect(renamed.view.page.title).toBe("Visual System");

    const taken = call("rename_page", { page: "visual-system", newName: "outl" }) as {
      status: string;
      existing: { slug: string };
    };
    expect(taken).toMatchObject({ status: "exists", existing: { slug: "outl" } });
  });

  it("merges one page into another and drops the source", () => {
    const call = fresh();
    const reply = call("merge_pages", { source: "design", target: "outl" }) as {
      view: PageView;
      report: { moved_blocks: number; source_slug: string };
    };
    expect(reply.view.page.slug).toBe("outl");
    expect(reply.report).toMatchObject({ source_slug: "design", moved_blocks: 1 });
    expect(reply.view.outline.some((b) => b.text === "Notes on the visual system")).toBe(true);
    expect((call("list_all_pages") as { slug: string }[]).some((p) => p.slug === "design")).toBe(false);
  });

  it("refuses to rename or merge a journal", () => {
    const call = fresh();
    expect(() => call("rename_page", { page: "2026-10-11", newName: "x" })).toThrow(/journal/);
    expect(() => call("merge_pages", { source: "2026-10-11", target: "outl" })).toThrow(/journal/);
  });

  it("serves every preset palette", () => {
    const names = fresh()("list_themes") as string[];
    expect(names).toContain("outl");
    expect(names).toHaveLength(11);
  });
});

describe("wantsMock", () => {
  it("reads the query string", () => {
    expect(wantsMock("?mock=1")).toBe(true);
    expect(wantsMock("?mock")).toBe(true);
    expect(wantsMock("?mock=0")).toBe(false);
    expect(wantsMock("")).toBe(false);
  });
});
