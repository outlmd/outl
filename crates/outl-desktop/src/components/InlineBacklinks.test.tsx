import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type {
  Backlink,
  PageBacklinks,
  PageView,
  TodoState,
} from "@outl/shared/api/types";
import {
  openRef,
  pageBacklinks,
  toggleTodo,
} from "@outl/shared/api/commands";

import { InlineBacklinks } from "./InlineBacklinks";
import { setAppState, trackBacklinks } from "../lib/store";

vi.mock("@outl/shared/api/commands", () => ({
  openRef: vi.fn(),
  pageBacklinks: vi.fn(),
  setBacklinksOrder: vi.fn(),
  toggleTodo: vi.fn(),
}));

let dispose: (() => void) | undefined;

function backlink(id: string, todo: TodoState | null): Backlink {
  return {
    block_id: id,
    todo,
    ancestors: [],
    source_page: { id: "page-1", slug: "journal", title: "Journal", kind: "page" },
    source_block: {
      id,
      text: "task #project",
      todo,
      tokens: [
        { kind: "plain", value: "task " },
        { kind: "tag", value: "#project" },
      ],
      collapsed: false,
      properties: [],
      children: [],
    },
    source_block_path: [],
  };
}

function mount(backlinks: Backlink[]): HTMLElement {
  setAppState({
    page: { id: "tag-page", slug: "project", title: "project", kind: "page" },
    backlinksOpen: true,
    backlinks,
    backlinksOrder: "newest",
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <InlineBacklinks />, host);
  return host;
}

function pageView(): PageView {
  return {
    page: { id: "page-1", slug: "journal", title: "Journal", kind: "page" },
    outline: [],
    // Always empty from the open commands — backlinks are fetched
    // lazily via `pageBacklinks`, never carried by a `PageView`.
    backlinks: [],
    backlinks_order: "newest",
  };
}

function backlinksReply(backlinks: Backlink[]): PageBacklinks {
  return {
    backlinks,
    backlinks_order: "newest",
    namespace_children: [],
    namespace_backlinks: [],
    namespace_backlinks_total: 0,
  };
}

/**
 * A textarea standing in for the block the user is typing in, with the
 * caret in it. Nothing about a backlinks fetch may reach it.
 */
function focusedEditor(): HTMLTextAreaElement {
  const editor = document.createElement("textarea");
  editor.value = "typing while backlinks load";
  document.body.appendChild(editor);
  editor.focus();
  return editor;
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  vi.clearAllMocks();
  setAppState({
    page: null,
    backlinks: [],
    backlinksOpen: true,
    backlinksOrder: "newest",
    backlinksLoading: false,
  });
  document.body.innerHTML = "";
});

describe("InlineBacklinks task state — #144", () => {
  it("renders task indicators in tag backlink listings", () => {
    const host = mount([
      backlink("todo", "TODO"),
      backlink("done", "DONE"),
      backlink("plain", null),
    ]);

    expect(host.querySelector('[data-todo="TODO"]')?.textContent).toBe("▢");
    expect(host.querySelector('[data-todo="DONE"]')?.textContent).toBe("▣");
    expect(host.querySelector('[data-todo="none"]')?.textContent).toBe("•");
    expect(host.querySelectorAll("[data-todo]")).toHaveLength(3);
    expect(host.querySelector('[data-todo="DONE"]')?.nextElementSibling?.className).toContain(
      "line-through",
    );
  });

  it("toggles the source task and refreshes the current tag view", async () => {
    vi.mocked(toggleTodo).mockResolvedValue(pageView());
    vi.mocked(pageBacklinks).mockResolvedValue(
      backlinksReply([backlink("todo", "DONE")]),
    );
    const host = mount([backlink("todo", "TODO")]);

    (host.querySelector('[data-todo="TODO"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(toggleTodo).toHaveBeenCalledWith("page-1", "todo");
      // Refetch the *current* page's backlinks, not a re-open: the
      // slug didn't change, so OutlineView's per-slug effect won't fire.
      expect(pageBacklinks).toHaveBeenCalledWith("project");
    });
    expect(host.querySelector('[data-todo="DONE"]')?.textContent).toBe("▣");
    expect(openRef).not.toHaveBeenCalled();
  });

  it("starts a plain backlink as TODO", async () => {
    vi.mocked(toggleTodo).mockResolvedValue(pageView());
    vi.mocked(pageBacklinks).mockResolvedValue(
      backlinksReply([backlink("plain", "TODO")]),
    );
    const host = mount([backlink("plain", null)]);

    (host.querySelector('[data-todo="none"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(toggleTodo).toHaveBeenCalledWith("page-1", "plain");
      expect(host.querySelector('[data-todo="TODO"]')?.textContent).toBe("▢");
    });
  });

  it("keeps text clicks navigating to the source block", async () => {
    vi.mocked(openRef).mockResolvedValue(pageView());
    const host = mount([backlink("todo", "TODO")]);
    const textButton = Array.from(host.querySelectorAll("button")).find((button) =>
      button.textContent?.includes("task"),
    );

    if (!textButton) throw new Error("backlink text button did not render");
    textButton.click();

    await vi.waitFor(() => {
      expect(openRef).toHaveBeenCalledWith("journal");
    });
    expect(toggleTodo).not.toHaveBeenCalled();
  });
});

/**
 * `UX.md` → Never block a keystroke: the backlinks scan is
 * `O(blocks-in-workspace)`, so it loads behind the page, and the only
 * thing on screen that may say so is the backlinks section itself.
 *
 * The product regression these pin: the page went into a load state
 * while backlinks resolved, which took the editor and the caret with
 * it. The section owning its own pending state is what makes a global
 * one unnecessary.
 */
describe("backlinks load behind the page", () => {
  it("leaves the editor alone while a first load runs", async () => {
    let settle: (r: PageBacklinks) => void = () => {};
    vi.mocked(pageBacklinks).mockReturnValue(
      new Promise<PageBacklinks>((resolve) => {
        settle = resolve;
      }),
    );
    const editor = focusedEditor();
    const host = mount([]);

    const done = trackBacklinks(pageBacklinks("project"));

    // The input the user is in is untouched: mounted, enabled, caret kept.
    expect(document.activeElement).toBe(editor);
    expect(editor.disabled).toBe(false);

    settle(backlinksReply([backlink("todo", "TODO")]));
    await done;

    expect(host.querySelector('[data-todo="TODO"]')?.textContent).toBe("▢");
    expect(document.activeElement).toBe(editor);
  });

  it("keeps the rows it already has while a refetch runs", async () => {
    let settle: (r: PageBacklinks) => void = () => {};
    vi.mocked(pageBacklinks).mockReturnValue(
      new Promise<PageBacklinks>((resolve) => {
        settle = resolve;
      }),
    );
    const host = mount([backlink("todo", "TODO")]);

    const done = trackBacklinks(pageBacklinks("project"));

    // Blanking the list to show a placeholder would make a refetch read
    // as "this page lost its references".
    expect(host.querySelector('[data-todo="TODO"]')).not.toBeNull();
    expect(host.querySelector("[data-backlinks-pending]")).not.toBeNull();

    settle(backlinksReply([backlink("todo", "DONE")]));
    await done;
    expect(host.querySelector('[data-todo="DONE"]')).not.toBeNull();
  });

  it("clears the pending state when the fetch fails", async () => {
    vi.mocked(pageBacklinks).mockRejectedValue(new Error("page not found"));
    // Mounted with rows, so the assertion is about the mark and not
    // about a section that was never on screen in the first place.
    const host = mount([backlink("todo", "TODO")]);

    await trackBacklinks(pageBacklinks("project"));

    // A swallowed failure that leaves the mark up is a section that
    // loads forever; the rows it already had stay.
    expect(host.querySelector("[data-backlinks-pending]")).toBeNull();
    expect(host.querySelector('[data-todo="TODO"]')).not.toBeNull();
  });
});

/**
 * The blink (`UX.md` → Never block a keystroke, root `CLAUDE.md` →
 * "Putting the page in a load state for work that is not the page").
 *
 * The section used to mount the moment the scan started and unmount
 * when it came back empty, so every page with no references drew 95px
 * of header + skeleton and then took it away. Measured in the running
 * desktop client: two geometry changes, ~1.2s apart, on a page that
 * ends with nothing.
 *
 * The rule these pin is `lateSectionVisible`'s
 * (`@outl/shared/outline`): a section fed by a reply that lands after
 * the first paint never appears and then disappears.
 */
describe("the backlinks section never appears and then disappears", () => {
  it("draws nothing while a first load runs on a page with no backlinks", async () => {
    let settle: (r: PageBacklinks) => void = () => {};
    vi.mocked(pageBacklinks).mockReturnValue(
      new Promise<PageBacklinks>((resolve) => {
        settle = resolve;
      }),
    );
    const host = mount([]);

    const done = trackBacklinks(pageBacklinks("project"));
    expect(host.querySelector("section")).toBeNull();

    settle(backlinksReply([]));
    await done;
    expect(host.querySelector("section")).toBeNull();
  });

  it("appears once, complete, when the first load finds rows", async () => {
    let settle: (r: PageBacklinks) => void = () => {};
    vi.mocked(pageBacklinks).mockReturnValue(
      new Promise<PageBacklinks>((resolve) => {
        settle = resolve;
      }),
    );
    const host = mount([]);

    const done = trackBacklinks(pageBacklinks("project"));
    expect(host.querySelector("section")).toBeNull();

    settle(backlinksReply([backlink("todo", "TODO")]));
    await done;

    expect(host.querySelector("section")).not.toBeNull();
    // A count, never `0 refs` standing in for a pending scan.
    expect(host.querySelector("header")?.textContent).toContain("1 ref");
  });

  it("says it is refetching only once there are rows to say it about", async () => {
    let settle: (r: PageBacklinks) => void = () => {};
    vi.mocked(pageBacklinks).mockReturnValue(
      new Promise<PageBacklinks>((resolve) => {
        settle = resolve;
      }),
    );
    const host = mount([backlink("todo", "TODO")]);

    const done = trackBacklinks(pageBacklinks("project"));
    // The user is looking at a section that is about to change: the
    // mark is owed here, and only here.
    expect(host.querySelector("[data-backlinks-pending]")).not.toBeNull();

    settle(backlinksReply([backlink("todo", "DONE")]));
    await done;
    expect(host.querySelector("[data-backlinks-pending]")).toBeNull();
  });
});
