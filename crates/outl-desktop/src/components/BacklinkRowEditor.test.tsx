/**
 * A backlink row is a place to work, not only a destination.
 *
 * The row shows a block that lives on **another** page, so every
 * assertion here is about a mutation addressed somewhere other than the
 * page on screen. Three of them are the whole reason the surface needed
 * a design:
 *
 * - the command is `editBacklinkBlock`, never `editBlock` — the two
 *   differ in how they recover from a block id the tree no longer has,
 *   and `editBlock`'s recovery appends the user's text to the end of a
 *   page they are not looking at;
 * - the reply describes the **source** page, so it reaches the open
 *   page's state only when the two are the same page (which a page
 *   citing itself makes reachable);
 * - the backlinks list is re-asked once per commit, never per
 *   keystroke, because `pageBacklinks` is an `O(blocks-in-workspace)`
 *   scan (root `CLAUDE.md` invariant 15).
 *
 * The section itself is rendered, not the editor in isolation: the
 * gesture that opens it is the row's own click, and a test that mounted
 * the editor directly would pass with the row still navigating away.
 */
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type {
  Backlink,
  BlockNode,
  PageBacklinks,
  PageView,
} from "@outl/shared/api/types";
import {
  editBacklinkBlock,
  editBlock,
  openRef,
  pageBacklinks,
} from "@outl/shared/api/commands";

vi.mock("@outl/shared/api/commands", () => ({
  editBacklinkBlock: vi.fn(),
  editBlock: vi.fn(),
  listTemplates: vi.fn().mockResolvedValue([]),
  openRef: vi.fn(),
  pageBacklinks: vi.fn(),
  pluginList: vi.fn().mockResolvedValue([]),
  resolveEmbeds: vi.fn().mockResolvedValue([]),
  resolvePageEmbeds: vi.fn().mockResolvedValue([]),
  searchBlocks: vi.fn().mockResolvedValue([]),
  searchEmojis: vi.fn().mockResolvedValue([]),
  searchPages: vi.fn().mockResolvedValue([]),
  searchPersons: vi.fn().mockResolvedValue([]),
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

import { InlineBacklinks } from "./InlineBacklinks";
import { appState, setAppState, setOutline } from "../lib/store";

/** The page the user is reading. */
const OPEN_PAGE = {
  id: "open-page",
  slug: "project",
  title: "project",
  kind: "page" as const,
};

/** The page the citing block lives on. */
const SOURCE_PAGE = {
  id: "source-page",
  slug: "journal",
  title: "Journal",
  kind: "page" as const,
};

function block(id: string, text: string): BlockNode {
  return {
    id,
    text,
    todo: null,
    tokens: [{ kind: "plain", value: text }],
    collapsed: false,
    properties: [],
    children: [],
  };
}

function backlink(
  over: { source_page?: Backlink["source_page"] } = {},
): Backlink {
  return {
    block_id: "src-1",
    todo: null,
    ancestors: [],
    source_page: SOURCE_PAGE,
    source_block: block("src-1", "cites this page"),
    source_block_path: [],
    ...over,
  };
}

function backlinksReply(links: Backlink[]): PageBacklinks {
  return {
    backlinks: links,
    backlinks_order: "newest",
    namespace_children: [],
    namespace_backlinks: [],
    namespace_backlinks_total: 0,
  };
}

/** What `edit_backlink_block` resolves to: a view of the SOURCE page. */
function sourceView(): PageView {
  return {
    page: SOURCE_PAGE,
    outline: [block("src-1", "cites this page, edited")],
    backlinks: [],
    backlinks_order: "newest",
  };
}

let dispose: (() => void) | undefined;

function mount(links: Backlink[]): HTMLElement {
  setAppState({
    page: OPEN_PAGE,
    backlinksOpen: true,
    backlinksLoading: false,
    backlinks: links,
    backlinksOrder: "newest",
    selectedBlockId: null,
    selectedBacklinkBlockId: null,
    editingBlockId: null,
    editingBacklink: null,
    lastError: null,
  });
  setOutline([block("own-1", "a block of the open page")]);
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <InlineBacklinks />, host);
  return host;
}

/** The row's own text target — the one carrying the edit gesture. */
function row(host: HTMLElement): HTMLElement {
  const found = Array.from(host.querySelectorAll("button")).find((b) =>
    b.textContent?.includes("cites this page"),
  );
  if (!found) throw new Error("backlink row did not render");
  return found;
}

function editor(host: HTMLElement): HTMLTextAreaElement {
  const ta = host.querySelector("textarea");
  if (!ta) throw new Error("backlink row editor did not open");
  return ta;
}

/** Type into the open editor the way the user does. */
function type(ta: HTMLTextAreaElement, value: string): void {
  ta.value = value;
  ta.dispatchEvent(new Event("input", { bubbles: true }));
}

/** Esc — commit and leave Insert. */
function escape(ta: HTMLTextAreaElement): void {
  ta.dispatchEvent(
    new KeyboardEvent("keydown", {
      key: "Escape",
      bubbles: true,
      cancelable: true,
    }),
  );
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  vi.clearAllMocks();
  setAppState({
    page: null,
    backlinks: [],
    backlinksOpen: true,
    editingBlockId: null,
    editingBacklink: null,
    selectedBlockId: null,
    selectedBacklinkBlockId: null,
    lastError: null,
  });
  setOutline([]);
  document.body.innerHTML = "";
});

describe("editing a backlink where it is cited", () => {
  it("a click on a backlink row opens an editor instead of navigating", () => {
    const host = mount([backlink()]);

    row(host).click();

    expect(editor(host).value).toBe("cites this page");
    // The view that assembled these references is what the user came
    // for; acting on one must not cost it.
    expect(openRef).not.toHaveBeenCalled();
    expect(appState.page?.id).toBe(OPEN_PAGE.id);
  });

  it("commits to the source page, not the open page", async () => {
    vi.mocked(editBacklinkBlock).mockResolvedValue(sourceView());
    vi.mocked(pageBacklinks).mockResolvedValue(backlinksReply([backlink()]));
    const host = mount([backlink()]);

    row(host).click();
    type(editor(host), "cites this page, edited");
    escape(editor(host));

    await vi.waitFor(() => {
      expect(editBacklinkBlock).toHaveBeenCalledWith(
        SOURCE_PAGE.id,
        "src-1",
        "cites this page, edited",
      );
    });
  });

  it("uses the strict command so a stale id cannot append", async () => {
    vi.mocked(editBacklinkBlock).mockResolvedValue(sourceView());
    vi.mocked(pageBacklinks).mockResolvedValue(backlinksReply([backlink()]));
    const host = mount([backlink()]);

    row(host).click();
    type(editor(host), "edited");
    escape(editor(host));

    await vi.waitFor(() => {
      expect(editBacklinkBlock).toHaveBeenCalled();
    });
    // `edit_block` tolerates a block that left the tree by appending the
    // text at the end of the page — right for the page you are reading,
    // and a silent move to another page from here.
    expect(editBlock).not.toHaveBeenCalled();
  });

  it("surfaces the refusal when the block is gone", async () => {
    // The backend's own sentence, forwarded verbatim: the wording is
    // `outl_actions::refusal`'s and a copy of it here would be the
    // second owner invariant 12 forbids. What is pinned is that the
    // string reaches `lastError` unchanged.
    vi.mocked(editBacklinkBlock).mockRejectedValue(
      "That block was deleted, so your text could not be saved.",
    );
    const host = mount([backlink()]);

    row(host).click();
    type(editor(host), "edited");
    escape(editor(host));

    await vi.waitFor(() => {
      // Nothing was written and retrying the same id cannot succeed, so
      // the refusal is a message — and it has to reach the user.
      expect(appState.lastError).toBe(
        "That block was deleted, so your text could not be saved.",
      );
    });
  });

  /**
   * **The one thing in this feature that cannot be reconstructed.**
   *
   * The refused text exists in no op, in no `.md` and on no peer — only
   * in this textarea. The editor used to close on a refusal, and the
   * draft signal died with the component, so the user's paragraph was
   * gone with nothing to recover it from.
   *
   * Reachable in the ordinary two-device case: a row drawn from the
   * from-disk index, a peer trashing the citing block, and an `Esc`
   * some seconds later.
   *
   * Asserted as **the text still being in the editor**, not as an error
   * having been reported. The previous test asserted `lastError` and
   * `editingBacklink === null` and said nothing about the draft, which
   * is how the loss was pinned as correct rather than caught.
   *
   * Mobile already answered a refusal this way
   * (`BacklinksSection.row-ops.ts` → `if (written) setEditing(null)`),
   * so the two clients no longer differ on it.
   */
  it("keeps the editor up on the user's draft when the commit is refused", async () => {
    vi.mocked(editBacklinkBlock).mockRejectedValue(
      "That block was deleted, so your text could not be saved.",
    );
    const host = mount([backlink()]);

    row(host).click();
    const ta = editor(host);
    type(ta, "three sentences the user typed and cannot retype");
    escape(ta);

    await vi.waitFor(() => {
      expect(appState.lastError).not.toBeNull();
    });
    // The text first, because it is the assertion that cannot be
    // satisfied any other way: the draft lives in the component, so a
    // closed editor is a destroyed paragraph.
    expect(host.querySelector("textarea")?.value).toBe(
      "three sentences the user typed and cannot retype",
    );
    // And the editor is still *this row's*. `editingBacklink` is what
    // keeps it there — and is also what `isEditing()` reads, so the
    // background refreshes that would remount the row and re-seed the
    // draft from disk stay deferred while it is set.
    expect(appState.editingBacklink).toEqual({
      pageId: SOURCE_PAGE.id,
      blockId: "src-1",
    });
  });

  it("leaves the editor and clears the draft once the commit lands", async () => {
    // The other direction of the same branch: keeping the editor open
    // on *success* would leave the row in Insert after every edit.
    vi.mocked(editBacklinkBlock).mockResolvedValue(sourceView());
    vi.mocked(pageBacklinks).mockResolvedValue(backlinksReply([backlink()]));
    const host = mount([backlink()]);

    row(host).click();
    type(editor(host), "edited");
    escape(editor(host));

    await vi.waitFor(() => {
      expect(appState.editingBacklink).toBeNull();
    });
    expect(host.querySelector("textarea")).toBeNull();
  });

  it("does not apply the reply to the page on screen", async () => {
    vi.mocked(editBacklinkBlock).mockResolvedValue(sourceView());
    vi.mocked(pageBacklinks).mockResolvedValue(backlinksReply([backlink()]));
    const host = mount([backlink()]);

    row(host).click();
    type(editor(host), "edited");
    escape(editor(host));

    await vi.waitFor(() => {
      expect(editBacklinkBlock).toHaveBeenCalled();
    });
    // The reply describes the source page. Installing it here replaces
    // the outline the user is reading with someone else's.
    expect(appState.page?.id).toBe(OPEN_PAGE.id);
    expect(appState.outline.map((b) => b.id)).toEqual(["own-1"]);
  });

  it("applies the reply when the backlink cites the open page", async () => {
    const selfView: PageView = {
      page: OPEN_PAGE,
      outline: [block("own-1", "a block of the open page, edited")],
      backlinks: [],
      backlinks_order: "newest",
    };
    vi.mocked(editBacklinkBlock).mockResolvedValue(selfView);
    vi.mocked(pageBacklinks).mockResolvedValue(backlinksReply([]));
    const host = mount([backlink({ source_page: OPEN_PAGE })]);

    row(host).click();
    type(editor(host), "edited");
    escape(editor(host));

    await vi.waitFor(() => {
      // A page can cite itself, and discarding the reply there drops a
      // genuine refresh of the outline on screen.
      expect(appState.outline[0]?.text).toBe(
        "a block of the open page, edited",
      );
    });
  });

  it("refetches backlinks once per commit, not per keystroke", async () => {
    vi.mocked(editBacklinkBlock).mockResolvedValue(sourceView());
    vi.mocked(pageBacklinks).mockResolvedValue(backlinksReply([backlink()]));
    const host = mount([backlink()]);

    row(host).click();
    const ta = editor(host);
    type(ta, "c");
    type(ta, "ci");
    type(ta, "cit");
    type(ta, "cite");

    // The scan is `O(blocks-in-workspace)`; the draft already holds what
    // the user is looking at, so nothing is owed until the commit lands.
    expect(pageBacklinks).not.toHaveBeenCalled();

    escape(ta);
    await vi.waitFor(() => {
      expect(pageBacklinks).toHaveBeenCalledTimes(1);
    });
    expect(pageBacklinks).toHaveBeenCalledWith(OPEN_PAGE.slug);
  });

  it("does not open an editor on an orphan backlink", () => {
    const host = mount([backlink({ source_page: null })]);

    row(host).click();

    // `source_page` is null when the citing block lives outside any
    // page. There is no page to address, and guessing one writes to the
    // wrong document.
    expect(host.querySelector("textarea")).toBeNull();
    expect(appState.editingBacklink).toBeNull();
    expect(editBacklinkBlock).not.toHaveBeenCalled();
  });
});
