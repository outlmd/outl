import { createRoot } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type {
  Backlink,
  PageBacklinks,
  PageView,
  TodoState,
} from "@outl/shared/api/types";
import { backlinkRowOp, openRef, pageBacklinks } from "@outl/shared/api/commands";

import { installBacklinksEffect } from "../lib/backlinks-effect";
import { applyPageState } from "../lib/page-view";
import { InlineBacklinks } from "./InlineBacklinks";
import { trackBacklinks } from "../lib/backlinks-fetch";
import { appState, setAppState } from "../lib/store";

// A row opens `<BacklinkRowEditor />` now, which mounts the shared
// `<BlockEditor />` — hence the autocomplete and clipboard stubs below
// alongside the section's own commands.
vi.mock("@outl/shared/api/commands", () => ({
  backlinkRowOp: vi.fn(),
  createBacklinkSibling: vi.fn(),
  editBacklinkBlock: vi.fn(),
  editBlock: vi.fn(),
  listTemplates: vi.fn(() => Promise.resolve([])),
  openRef: vi.fn(),
  pageBacklinks: vi.fn(),
  pluginList: vi.fn(() => Promise.resolve([])),
  resolveEmbeds: vi.fn(() => Promise.resolve([])),
  resolvePageEmbeds: vi.fn(() => Promise.resolve([])),
  searchBlocks: vi.fn(() => Promise.resolve([])),
  searchEmojis: vi.fn(() => Promise.resolve([])),
  searchPages: vi.fn(() => Promise.resolve([])),
  searchPersons: vi.fn(() => Promise.resolve([])),
  setBacklinksOrder: vi.fn(),
  toggleTodo: vi.fn(),
}));
vi.mock("@outl/shared/plugins/transformer-registry", () => ({
  runTransform: vi.fn(),
  transformerFor: vi.fn(() => null),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  readText: vi.fn(),
}));

/** The real navigation effect, owned beside the rendered section so a
 *  commit travels the whole chain the user sees. */
let disposeEffect: (() => void) | undefined;
function installEffect(): void {
  createRoot((d) => {
    disposeEffect = d;
    installBacklinksEffect();
  });
}

let dispose: (() => void) | undefined;

function backlink(
  id: string,
  todo: TodoState | null,
  ancestors: Backlink["ancestors"] = [],
): Backlink {
  return {
    block_id: id,
    todo,
    ancestors,
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
    // A row left in edit mode by an earlier test re-opens its editor
    // here the moment a matching `block_id` lands, and the editor
    // autofocuses — which is how it stole focus from the standalone
    // textarea in "leaves the editor alone while a first load runs".
    editingBacklink: null,
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <InlineBacklinks />, host);
  return host;
}

/** The source-page group header — the only control that navigates. */
function groupHeader(host: HTMLElement): HTMLButtonElement {
  const found = Array.from(host.querySelectorAll("button")).find((button) =>
    button.textContent?.includes("Journal"),
  );
  if (!found) throw new Error("source-page header did not render");
  return found as HTMLButtonElement;
}

/** Settle the `trackBacklinks` promise chain (then → catch → finally). */
function flush(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

/** A reply for the page `mount` opens, as a mutation returns it. */
function openPageView(): PageView {
  return {
    ...pageView(),
    page: { id: "tag-page", slug: "project", title: "project", kind: "page" },
  };
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
  disposeEffect?.();
  disposeEffect = undefined;
  vi.clearAllMocks();
  setAppState({
    page: null,
    backlinks: [],
    backlinksOpen: true,
    backlinksOrder: "newest",
    backlinksLoading: false,
    editingBacklink: null,
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
    vi.mocked(backlinkRowOp).mockResolvedValue(pageView());
    vi.mocked(pageBacklinks).mockResolvedValue(
      backlinksReply([backlink("todo", "DONE")]),
    );
    const host = mount([backlink("todo", "TODO")]);

    (host.querySelector('[data-todo="TODO"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      // The cross-page command, not `toggle_todo`: a click on the
      // bullet of a row a peer has trashed is the commonest way to
      // reach the refusal, and `toggle_todo` reports it as a ULID.
      expect(backlinkRowOp).toHaveBeenCalledWith(
        "page-1",
        "todo",
        "toggle-todo",
      );
      // Refetch the *current* page's backlinks, not a re-open: the
      // slug didn't change, so OutlineView's per-slug effect won't fire.
      expect(pageBacklinks).toHaveBeenCalledWith("project");
    });
    expect(host.querySelector('[data-todo="DONE"]')?.textContent).toBe("▣");
    expect(openRef).not.toHaveBeenCalled();
  });

  it("starts a plain backlink as TODO", async () => {
    vi.mocked(backlinkRowOp).mockResolvedValue(pageView());
    vi.mocked(pageBacklinks).mockResolvedValue(
      backlinksReply([backlink("plain", "TODO")]),
    );
    const host = mount([backlink("plain", null)]);

    (host.querySelector('[data-todo="none"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(backlinkRowOp).toHaveBeenCalledWith(
        "page-1",
        "plain",
        "toggle-todo",
      );
      expect(host.querySelector('[data-todo="TODO"]')?.textContent).toBe("▢");
    });
  });

  it("edits the cited block on a text click, without navigating", () => {
    const host = mount([backlink("todo", "TODO")]);
    // The prose of the outline's `<BlockBody />`, which the row renders.
    const textButton = Array.from(host.querySelectorAll<HTMLElement>("[data-backlink-body] .cursor-text")).find((el) =>
      el.textContent?.includes("task"),
    );

    if (!textButton) throw new Error("backlink text button did not render");
    textButton.click();

    // A row used to be a destination, so acting on one cost the user
    // the very view that assembled these references.
    expect(host.querySelector("textarea")).not.toBeNull();
    expect(openRef).not.toHaveBeenCalled();
    expect(backlinkRowOp).not.toHaveBeenCalled();
  });

  it("still opens the source page from the group header", async () => {
    vi.mocked(openRef).mockResolvedValue(pageView());
    const host = mount([backlink("todo", "TODO")]);

    groupHeader(host).click();

    // The "open the page" half of the gesture split — the only half
    // that navigates, and the one the row's click gave up.
    await vi.waitFor(() => {
      expect(openRef).toHaveBeenCalledWith("journal");
    });
    expect(host.querySelector("textarea")).toBeNull();
  });

  /**
   * The group header is the **only** door to the source page now —
   * `Enter` and the row click both became edit gestures — and it was
   * the one path that did not go through `applyPageState`.
   *
   * It wrote `page` + outline and nothing else, which is exactly the
   * second `applyView` this client paid to delete. So the new page
   * arrived wearing the previous one's `key:: value` chips, its parse
   * warnings, and its **"this page isn't syncing" banner** — and that
   * last one is not cosmetic: `<PageAheadOfLogBanner />` claims a page
   * has stopped converging with the user's other devices, so carrying
   * it over names a healthy page as broken, over a page that is fine,
   * until some later reply happens to run through the real writer.
   *
   * Pinned on the **resulting state**, not on `openRef` having been
   * called: the test that existed asserted the call and nothing about
   * what it installed, which is why the body survived the branch that
   * made this path the only one.
   */
  it("installs the whole reply when the group header opens the source page", async () => {
    // The page on screen is unhealthy and carries its own chips; the
    // page being opened is healthy and has neither.
    applyPageState({
      ...openPageView(),
      page_properties: [["type", "person"]],
      warnings: [{ kind: "unrecognized_block_marker", line: 3, raw: "~~ huh" }],
      md_ahead_of_log: {
        path: "journals/2026-11-19.md",
        lines: 2,
        sample: "- only ever on disk",
      },
      md_ahead_of_log_checked: true,
    });
    expect(appState.mdAheadOfLog).not.toBeUndefined();
    vi.mocked(openRef).mockResolvedValue({
      ...pageView(),
      md_ahead_of_log_checked: true,
    });
    const host = mount([backlink("todo", "TODO")]);

    groupHeader(host).click();

    await vi.waitFor(() => {
      expect(appState.page?.slug).toBe("journal");
    });
    // None of the previous page's claims may follow the user over.
    expect(appState.mdAheadOfLog).toBeUndefined();
    expect(appState.pageProperties).toEqual([]);
    expect(appState.parseWarnings).toEqual([]);
    // And it is a navigation, so the zoom and the row cursor go too.
    expect(appState.focusBlockId).toBeNull();
    expect(appState.selectedBacklinkBlockId).toBeNull();
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

    const done = trackBacklinks();

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

    const done = trackBacklinks();

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

    await trackBacklinks();

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

    const done = trackBacklinks();
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

    const done = trackBacklinks();
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

    const done = trackBacklinks();
    // The user is looking at a section that is about to change: the
    // mark is owed here, and only here.
    expect(host.querySelector("[data-backlinks-pending]")).not.toBeNull();

    settle(backlinksReply([backlink("todo", "DONE")]));
    await done;
    expect(host.querySelector("[data-backlinks-pending]")).toBeNull();
  });

  /**
   * The reported blink, end to end: the effect that decides when to
   * fetch, the store write it performs, and this section's own
   * visibility rule, in one assertion against the DOM.
   *
   * Each half was covered and the chain was not. `backlinks-effect.test.ts`
   * proves the effect no longer fires on a commit; this proves what that
   * was costing — the section leaving the page and coming back.
   */
  it("stays on screen while the user creates and deletes blocks", async () => {
    vi.mocked(pageBacklinks).mockResolvedValue(
      backlinksReply([backlink("todo", "TODO")]),
    );
    const host = mount([]);
    installEffect();
    await flush();
    expect(host.querySelector("section")).not.toBeNull();

    // Three mutation replies for the page already on screen: a fresh
    // `PageMeta` each time, same slug and title, as `create_block` /
    // `delete_block` hand back.
    for (const _ of [0, 1, 2]) {
      applyPageState(openPageView());
      expect(host.querySelector("section")).not.toBeNull();
      expect(host.querySelector('[data-todo="TODO"]')).not.toBeNull();
    }

    // One fetch, for the open. Not one per commit.
    expect(pageBacklinks).toHaveBeenCalledTimes(1);
  });

  /**
   * The same promise under a **held** chord, which is the shape the
   * dedupe had to be built without breaking.
   *
   * A held `Alt+↑` or `Tab` on a row commits once per key repeat, and
   * each commit re-asks. Coalescing those asks is only half the job: the
   * half that reaches the user is that the section keeps the **same**
   * `<section>` node throughout. An unmount and remount inside one tick
   * is still an unmount — the DOM node is destroyed, and a
   * `<BacklinkRowEditor />` open inside it loses its focus and the draft
   * the user was typing. That is what `emptyBacklinks()` on a refetch
   * used to cause, and what commit `4817abd2` exists to undo.
   */
  it("keeps the same section node through a burst of commits", async () => {
    const scans: Array<(r: PageBacklinks) => void> = [];
    vi.mocked(pageBacklinks).mockImplementation(
      () => new Promise<PageBacklinks>((resolve) => scans.push(resolve)),
    );
    const host = mount([backlink("todo", "TODO")]);
    const before = host.querySelector("section");
    expect(before).not.toBeNull();

    // Ten repeats, each re-asking the way a committed chord does.
    for (let i = 0; i < 10; i++) void trackBacklinks();
    await flush();

    // Node identity, not merely presence: `toBe` is what refuses a
    // remount that `not.toBeNull()` would call a pass.
    expect(host.querySelector("section")).toBe(before);
    expect(host.querySelector('[data-todo="TODO"]')).not.toBeNull();
    expect(host.querySelector("[data-backlinks-pending]")).not.toBeNull();

    // One scan in flight; the trailing one runs when it lands.
    expect(scans).toHaveLength(1);
    scans[0](backlinksReply([backlink("todo", "DONE")]));
    await flush();
    expect(scans).toHaveLength(2);
    expect(host.querySelector("section")).toBe(before);

    scans[1](backlinksReply([backlink("todo", "DONE")]));
    await flush();
    expect(host.querySelector("section")).toBe(before);
    expect(host.querySelector('[data-todo="DONE"]')).not.toBeNull();
    expect(host.querySelector("[data-backlinks-pending]")).toBeNull();
  });
});

describe("InlineBacklinks breadcrumb", () => {
  /** The trail from the bug report: a tag, a namespaced ref and a mail
   *  link, in the branch a `#fup` reference sits under. */
  const ancestors: Backlink["ancestors"] = [
    {
      id: "crumb-1",
      text: "#alinhamento [[buser/team/vendas]] internet 0800 [email](https://mail.google.com/mail?authuser=someone)",
      tokens: [
        { kind: "tag", value: "#alinhamento" },
        { kind: "plain", value: " " },
        { kind: "ref", value: "buser/team/vendas" },
        { kind: "plain", value: " internet 0800 " },
        {
          kind: "link",
          value: "email",
          href: "https://mail.google.com/mail?authuser=someone",
        },
      ],
    },
  ];

  it("renders the ancestor trail instead of its markdown source", () => {
    const host = mount([backlink("ref", null, ancestors)]);

    expect(host.textContent).toContain("buser/team/vendas");
    expect(host.textContent).toContain("#alinhamento");
    expect(host.textContent).toContain("email");
    // The three the raw source showed, which is the whole defect.
    expect(host.textContent).not.toContain("[[");
    expect(host.textContent).not.toContain("](");
    expect(host.textContent).not.toContain("mail.google.com");
  });

  it("keeps the plain source as the tooltip, where markup cannot go", () => {
    const host = mount([backlink("ref", null, ancestors)]);

    // The trail's own row, not the header's sort button and not the
    // `title={href}` MarkdownInline puts on the link inside it.
    expect(host.querySelector("[data-backlink-row] > [title]")?.getAttribute("title")).toBe(
      ancestors[0].text,
    );
  });
});
