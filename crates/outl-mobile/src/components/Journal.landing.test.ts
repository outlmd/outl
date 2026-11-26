import { beforeEach, describe, expect, it, vi } from "vitest";

import type { BlockNode, PageView } from "@outl/shared/api/types";

const commands = vi.hoisted(() => ({
  openJournalFor: vi.fn(),
  openPageBySlug: vi.fn(),
}));
vi.mock("@outl/shared/api/commands", () => commands);

import { landedMarks } from "../lib/landed";
import { createBacklinkLanding, type LandingLink } from "./Journal.landing";

function node(id: string, children: BlockNode[] = [], collapsed = false): BlockNode {
  return { id, children, collapsed } as unknown as BlockNode;
}
function view(slug: string, outline: BlockNode[], id = `pg-${slug}`): PageView {
  return { page: { id, slug, title: slug, kind: "page" }, outline } as unknown as PageView;
}
const link = (over: Partial<LandingLink> = {}): LandingLink => ({
  block_id: "cite",
  ancestors: [{ id: "parent" } as never],
  source_page: { id: "pg-src", slug: "src", title: "src", kind: "page" } as never,
  ...over,
});

function setup(current?: () => string | undefined) {
  // Painting a view puts that page on screen, as `Journal.tsx` does.
  let onScreen: string | undefined = "pg-cur";
  const d = {
    withError: <T,>(fn: () => Promise<T>) => fn().catch(() => undefined),
    unfold: vi.fn(),
    openView: vi.fn((v: PageView) => {
      onScreen = v.page.id;
    }),
    notify: vi.fn(),
    currentPageId: current ?? (() => onScreen),
    reveal: vi.fn(),
    nextFrame: (cb: () => void) => cb(),
  };
  return {
    d,
    l: createBacklinkLanding(d),
    leave: () => {
      onScreen = "pg-elsewhere";
    },
  };
}

const flat = () => view("src", [node("parent", [node("cite")])], "pg-src");
const folded = () => view("src", [node("gp", [node("parent", [node("cite")])], true)], "pg-src");

beforeEach(() => {
  vi.clearAllMocks();
});

describe("createBacklinkLanding", () => {
  it("nothing folded: no unfold, one paint, lands on the parent", async () => {
    commands.openPageBySlug.mockResolvedValue(flat());
    const { d, l } = setup();
    await l.jump(link());
    expect(d.unfold).not.toHaveBeenCalled();
    expect(d.openView).toHaveBeenCalledTimes(1);
    expect(d.reveal).toHaveBeenCalledWith("parent");
    expect(landedMarks.isSaved("parent")).toBe(true);
  });

  it("folded: unfolds from the citing block and paints the unfolded view", async () => {
    commands.openPageBySlug.mockResolvedValue(folded());
    const after = view("src", [node("gp", [node("parent", [node("cite")])])], "pg-src");
    const { d, l } = setup();
    d.unfold.mockResolvedValue(after);
    await l.jump(link());
    expect(d.unfold).toHaveBeenCalledWith("pg-src", "cite");
    expect(d.openView).toHaveBeenCalledTimes(1);
    expect(d.openView).toHaveBeenCalledWith(after);
  });

  it("crumb gone: falls back to the citing block's parent", async () => {
    commands.openPageBySlug.mockResolvedValue(flat());
    const { d, l } = setup();
    await l.jump(link({ ancestors: [{ id: "deleted" } as never] }));
    expect(d.reveal).toHaveBeenCalledWith("parent");
  });

  it("everything gone: opens the page and says so, writes nothing", async () => {
    commands.openPageBySlug.mockResolvedValue(view("src", [node("other")], "pg-src"));
    const { d, l } = setup();
    await l.jump(link());
    expect(d.unfold).not.toHaveBeenCalled();
    expect(d.openView).toHaveBeenCalledTimes(1);
    expect(d.notify).toHaveBeenCalledWith(expect.stringContaining("no longer on src"));
    expect(d.reveal).not.toHaveBeenCalled();
  });

  it("journal kinds go through openJournalFor", async () => {
    commands.openJournalFor.mockResolvedValue(flat());
    const { l } = setup();
    await l.jump(
      link({ source_page: { id: "j", slug: "2026-11-24", title: "x", kind: "journal" } as never }),
    );
    expect(commands.openJournalFor).toHaveBeenCalledWith("2026-11-24");
    expect(commands.openPageBySlug).not.toHaveBeenCalled();
  });

  it("open rejects: nothing is painted", async () => {
    commands.openPageBySlug.mockRejectedValue(new Error("boom"));
    const { d, l } = setup();
    await l.jump(link());
    expect(d.openView).not.toHaveBeenCalled();
  });

  it("unfold rejects: the opened view is shown, nothing is lit, and it says so", async () => {
    const opened = folded();
    commands.openPageBySlug.mockResolvedValue(opened);
    const { d, l } = setup();
    d.unfold.mockRejectedValue(new Error("nope"));
    await l.jump(link());
    expect(d.openView).toHaveBeenCalledWith(opened);
    expect(d.reveal).not.toHaveBeenCalled();
    expect(d.notify).toHaveBeenCalledWith(expect.stringContaining("folded on src"));
  });

  it("the unfold reply keeps the opened page's ahead-of-log notice", async () => {
    const notice = { path: "/x.md", lines: 2 };
    commands.openPageBySlug.mockResolvedValue({
      ...folded(),
      md_ahead_of_log: notice,
      md_ahead_of_log_checked: true,
    });
    const { d, l } = setup();
    d.unfold.mockResolvedValue(view("src", [node("parent", [node("cite")])], "pg-src"));
    await l.jump(link());
    expect(d.openView.mock.calls[0][0]).toMatchObject({ md_ahead_of_log: notice });
  });

  it("a live top-level citing block is lit, not reported as gone", async () => {
    commands.openPageBySlug.mockResolvedValue(view("src", [node("cite")], "pg-src"));
    const { d, l } = setup();
    await l.jump(link({ ancestors: [] }));
    expect(d.reveal).toHaveBeenCalledWith("cite");
    expect(d.notify).not.toHaveBeenCalled();
  });

  it("scrolls first, then marks, on the next frame", async () => {
    commands.openPageBySlug.mockResolvedValue(flat());
    const order: string[] = [];
    const mark = vi.spyOn(landedMarks, "mark").mockImplementation(() => void order.push("mark"));
    const { d, l } = setup();
    d.reveal.mockImplementation(() => void order.push("reveal"));
    await l.jump(link());
    mark.mockRestore();
    expect(order).toEqual(["reveal", "mark"]);
  });

  it("leaving the page within the frame neither scrolls nor marks", async () => {
    commands.openPageBySlug.mockResolvedValue(view("src", [node("parent", [node("cite")])], "pg-src2"));
    const pending: Array<() => void> = [];
    const mark = vi.spyOn(landedMarks, "mark").mockImplementation(() => {});
    const d = setup();
    const l = createBacklinkLanding({ ...d.d, nextFrame: (cb) => pending.push(cb) });
    await l.jump(link());
    d.leave();
    pending.forEach((cb) => cb());
    const marked = mark.mock.calls.length;
    mark.mockRestore();
    expect(d.d.reveal).not.toHaveBeenCalled();
    expect(marked).toBe(0);
  });

  it("overlapping jumps: only the second paints", async () => {
    let release!: (v: PageView) => void;
    commands.openPageBySlug
      .mockImplementationOnce(() => new Promise((r) => (release = r)))
      .mockResolvedValueOnce(flat());
    const { d, l } = setup();
    const first = l.jump(link());
    await l.jump(link());
    release(flat());
    await first;
    expect(d.openView).toHaveBeenCalledTimes(1);
  });

  it("page changed by another path mid-flight: nothing paints", async () => {
    commands.openPageBySlug.mockResolvedValue(flat());
    let cur = "pg-cur";
    const { d, l } = setup(() => cur);
    const p = l.jump(link());
    cur = "pg-elsewhere";
    await p;
    expect(d.openView).not.toHaveBeenCalled();
  });

  it("an orphan link (no source page) does nothing", async () => {
    const { d, l } = setup();
    await l.jump(link({ source_page: null }));
    expect(d.openView).not.toHaveBeenCalled();
  });
});
