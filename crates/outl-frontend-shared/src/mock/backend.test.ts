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
