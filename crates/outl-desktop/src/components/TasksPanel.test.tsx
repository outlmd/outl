import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { openPageBySlug, todaySlug } from "@outl/shared/api/commands";
import { listOpenTasks, type TaskHit } from "@outl/shared/api/overview";
import type { PageView } from "@outl/shared/api/types";

import { TasksPanel } from "./TasksPanel";
import { appState, setAppState } from "../lib/store";

vi.mock("@outl/shared/api/commands", () => ({
  openPageBySlug: vi.fn(),
  todaySlug: vi.fn(),
}));
vi.mock("@outl/shared/api/overview", () => ({
  listOpenTasks: vi.fn(),
}));

let dispose: (() => void) | undefined;

function task(id: string, slug: string, text: string, status: "todo" | "doing" = "todo"): TaskHit {
  return { id, source_slug: slug, status, text };
}

function view(slug: string): PageView {
  return {
    page: { id: `page-${slug}`, slug, title: slug, kind: "page" },
    outline: [],
    backlinks: [],
    backlinks_order: "newest",
  };
}

async function mount(tasks: TaskHit[], applyView = vi.fn()) {
  // Fresh objects per call, like the real command, so a refetch really replaces the rows.
  vi.mocked(listOpenTasks).mockImplementation(async () => ({ total: tasks.length, tasks: [...tasks] }));
  vi.mocked(todaySlug).mockResolvedValue("2026-10-23");
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <TasksPanel applyView={applyView} setError={vi.fn()} />, host);
  setAppState("tasksOpen", true);
  await vi.waitFor(() => expect(listOpenTasks).toHaveBeenCalled());
  await new Promise((r) => setTimeout(r, 0));
  return { host, applyView };
}

beforeEach(() => {
  setAppState({ tasksOpen: false, selectedBlockId: null });
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("TasksPanel", () => {
  it("lists the rows it is given, grouped by page", async () => {
    const { host } = await mount([
      task("a", "projects/outl", "ship the panel"),
      task("b", "projects/outl", "write the test"),
      task("c", "reading", "finish the paper"),
    ]);
    const headings = [...host.querySelectorAll("h3")].map((h) => h.textContent);
    expect(headings).toEqual(["projects/outl", "reading"]);
    const rows = [...host.querySelectorAll("button[data-task-row]")].map((b) => b.textContent);
    expect(rows).toEqual(["ship the panel", "write the test", "finish the paper"]);
  });

  it("renders one heading per page, even when a page's rows are not adjacent", async () => {
    // The backend sorts every started task ahead of every TODO, so a
    // page carrying both arrives as two runs, and two "Today" headings
    // in one list reads as a rendering bug. What grouping does to an
    // order is `@outl/shared/tasks`' own test; this is the rendering
    // half — one `<h3>`, named the way the journal names its day.
    const { host } = await mount([
      task("a", "2026-10-23", "review PR", "doing"),
      task("b", "2026-10-22", "write RFC", "doing"),
      task("c", "2026-10-23", "reply to the thread"),
      task("d", "2026-10-23", "book the flight"),
    ]);
    const headings = [...host.querySelectorAll("h3")].map((h) => h.textContent);
    expect(headings).toEqual(["Today, October 23, 2026", "Yesterday, October 22, 2026"]);
  });

  it("names a journal page by its heading", async () => {
    const { host } = await mount([task("a", "2026-10-22", "call back")]);
    expect(host.querySelector("h3")?.textContent).toBe("Yesterday, October 22, 2026");
  });

  it("jumps to the task's block when a row is picked", async () => {
    vi.mocked(openPageBySlug).mockResolvedValue(view("reading"));
    const { host, applyView } = await mount([task("c", "reading", "finish the paper")]);
    host.querySelector<HTMLButtonElement>("button[data-task-row]")!.click();
    await vi.waitFor(() => expect(applyView).toHaveBeenCalled());
    expect(openPageBySlug).toHaveBeenCalledWith("reading");
    expect(appState.selectedBlockId).toBe("c");
    expect(appState.tasksOpen).toBe(false);
  });

  it("says so when nothing is open", async () => {
    const { host } = await mount([]);
    expect(host.textContent).toContain("Nothing open");
  });

  it("closes on Escape", async () => {
    await mount([task("a", "reading", "x")]);
    // Keyboard-only: the event comes from wherever focus really is.
    await vi.waitFor(() => expect(document.activeElement?.hasAttribute("data-task-row")).toBe(true));
    document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(appState.tasksOpen).toBe(false);
  });

  it("puts focus on the first row again after a close and reopen", async () => {
    const { host } = await mount([task("a", "reading", "x"), task("b", "reading", "y")]);
    await vi.waitFor(() => expect(document.activeElement).toBe(host.querySelector("button[data-task-row]")));
    setAppState("tasksOpen", false);
    setAppState("tasksOpen", true);
    await vi.waitFor(() => expect(listOpenTasks).toHaveBeenCalledTimes(2));
    // Let the fresh rows land and settle: asserting earlier passes on the
    // stale rows that are about to be replaced.
    await new Promise((r) => setTimeout(r, 20));
    expect(document.activeElement).toBe(host.querySelector("button[data-task-row]"));
  });

  it("puts focus on the first row when the list arrives", async () => {
    const { host } = await mount([task("a", "reading", "x"), task("b", "reading", "y")]);
    await vi.waitFor(() => expect(document.activeElement).toBe(host.querySelector("button[data-task-row]")));
  });

  it("keeps the panel open and reports the error when the page is gone", async () => {
    vi.mocked(openPageBySlug).mockRejectedValue(new Error("page not found"));
    const setError = vi.fn();
    vi.mocked(listOpenTasks).mockResolvedValue({ total: 1, tasks: [task("c", "gone", "orphan")] });
    vi.mocked(todaySlug).mockResolvedValue("2026-10-23");
    const host = document.createElement("div");
    document.body.appendChild(host);
    dispose = render(() => <TasksPanel applyView={vi.fn()} setError={setError} />, host);
    setAppState("tasksOpen", true);
    await vi.waitFor(() => expect(host.querySelector("button[data-task-row]")).not.toBeNull());
    host.querySelector<HTMLButtonElement>("button[data-task-row]")!.click();
    await vi.waitFor(() => expect(setError).toHaveBeenCalledWith("page not found"));
    expect(appState.tasksOpen).toBe(true);
  });

  it("does not fetch while closed", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    dispose = render(() => <TasksPanel applyView={vi.fn()} setError={vi.fn()} />, host);
    await new Promise((r) => setTimeout(r, 0));
    expect(listOpenTasks).not.toHaveBeenCalled();
  });
});
