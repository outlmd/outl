/**
 * `yy` / `Cmd+C` then `p` / `Cmd+V`, through the OS clipboard.
 *
 * The clipboard plugin is replaced by one string, so a copy and the
 * paste after it meet where they meet in the app. What these pin is
 * everything that can go wrong *between* the two halves:
 *
 * - a `p` pressed while the yank's `copy_markdown` is still in flight
 *   must paste the new markdown, not the clipboard from before;
 * - the paste is anchored on where the cursor was when `p` was pressed,
 *   and a navigation during the read or the IPC does not land the paste
 *   (or its reply) on the wrong page;
 * - a held `p` is one paste, not a stack of them;
 * - every failure reaches the status line.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Backlink, BlockNode, PageView } from "@outl/shared/api/types";

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => ({ close: vi.fn() })),
}));

vi.mock("@outl/shared/api/commands", () => ({
  copyBlockEmbed: vi.fn(),
  copyBlockRef: vi.fn(),
  copyMarkdown: vi.fn(),
  moveBlockAfter: vi.fn(),
  pasteBlockAfter: vi.fn(),
  pluginSyncHooks: vi.fn(),
}));

/** The OS clipboard: one string, written and read by the plugin. */
const os = vi.hoisted(() => ({ text: "" }));
const plugin = vi.hoisted(() => ({
  readText: vi.fn<() => Promise<string>>(),
  writeText: vi.fn<(text: string) => Promise<void>>(),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => plugin);

vi.mock("./api", () => ({
  redoPage: vi.fn(),
  undoPage: vi.fn(),
}));

import {
  copyMarkdown,
  moveBlockAfter,
  pasteBlockAfter,
} from "@outl/shared/api/commands";

import { buildHandlers } from "./action-handlers";
import { PASTE_IN_OUTLINE } from "./clipboard-handlers";
import { appState, setAppState } from "./store";

function block(id: string, text: string): BlockNode {
  return {
    id,
    text,
    todo: null,
    tokens: [],
    collapsed: false,
    properties: [],
    children: [],
  };
}

function view(pageId = "pg-1"): PageView {
  return {
    page: { id: pageId, slug: "today", title: "Today", kind: "journal" },
    outline: [],
    backlinks: [],
    backlinks_order: "newest",
  };
}

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

function row(id: string): Backlink {
  return {
    block_id: id,
    todo: null,
    source_page: { id: "pg-src", slug: "src", title: "Src", kind: "page" },
    source_block: block(id, "cites [[today]]"),
    source_block_path: [0],
    ancestors: [],
  };
}

const applyView = vi.fn();
const setError = vi.fn();
const handlers = () => buildHandlers({ applyView, setError });

beforeEach(() => {
  vi.clearAllMocks();
  os.text = "old clipboard";
  plugin.writeText.mockImplementation(async (t) => {
    os.text = t;
  });
  plugin.readText.mockImplementation(async () => os.text);
  vi.mocked(pasteBlockAfter).mockResolvedValue({
    view: view(),
    new_id: "blk-new",
  });
  window.getSelection()?.removeAllRanges();
  setAppState({
    page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
    outline: [block("blk-a", "a"), block("blk-b", "b")],
    backlinks: [],
    backlinksOpen: true,
    focusBlockId: null,
    selectedBlockId: "blk-a",
    selectedBacklinkBlockId: null,
    editingBlockId: null,
    editingBacklink: null,
    blockClipboard: null,
    visualAnchorId: null,
    mode: "vim-normal",
    lastError: null,
  });
});

describe("yy then p", () => {
  it("pastes the new markdown even while copy_markdown is still in flight", async () => {
    const md = deferred<string>();
    vi.mocked(copyMarkdown).mockReturnValue(md.promise);
    const h = handlers();

    const yank = h.YankCurrentBlock?.();
    const paste = h.PasteClipboard?.();
    md.resolve("- a\n");
    await Promise.all([yank, paste]);

    expect(pasteBlockAfter).toHaveBeenCalledWith(
      "pg-1",
      { afterId: "blk-a" },
      "- a\n",
    );
    expect(applyView).toHaveBeenCalledWith(view());
    expect(appState.selectedBlockId).toBe("blk-new");
  });

  it("Visual y then p pastes the whole range", async () => {
    vi.mocked(copyMarkdown).mockResolvedValue("- a\n- b\n");
    setAppState({ visualAnchorId: "blk-a", selectedBlockId: "blk-b", mode: "vim-visual" });
    const h = handlers();

    void h.YankRange?.();
    await h.PasteClipboard?.();

    expect(copyMarkdown).toHaveBeenCalledWith(["blk-a", "blk-b"]);
    expect(pasteBlockAfter).toHaveBeenCalledWith(
      "pg-1",
      { afterId: "blk-b" },
      "- a\n- b\n",
    );
  });

  it("Cmd+V with no cut armed is the same paste as p", async () => {
    os.text = "from another app";
    await handlers().PasteBlock?.();

    expect(moveBlockAfter).not.toHaveBeenCalled();
    expect(pasteBlockAfter).toHaveBeenCalledWith(
      "pg-1",
      { afterId: "blk-a" },
      "from another app",
    );
  });

  it("Visual y disarms a pending cut, so Cmd+V pastes the range", async () => {
    vi.mocked(copyMarkdown).mockResolvedValue("- a\n- b\n");
    setAppState({
      blockClipboard: { kind: "cut", nodeId: "blk-b" },
      visualAnchorId: "blk-a",
      selectedBlockId: "blk-b",
      mode: "vim-visual",
    });
    const h = handlers();

    void h.YankRange?.();
    expect(appState.blockClipboard).toBeNull();
    await h.PasteBlock?.();

    expect(moveBlockAfter).not.toHaveBeenCalled();
    expect(pasteBlockAfter).toHaveBeenCalledWith(
      "pg-1",
      { afterId: "blk-b" },
      "- a\n- b\n",
    );
  });

  it("Cmd+C over a mouse-dragged selection copies that text, not the block", async () => {
    const span = document.createElement("span");
    span.textContent = "dragged words";
    document.body.appendChild(span);
    const range = document.createRange();
    range.selectNodeContents(span);
    window.getSelection()?.addRange(range);
    setAppState("blockClipboard", { kind: "cut", nodeId: "blk-b" });

    await handlers().CopyBlock?.();

    expect(copyMarkdown).not.toHaveBeenCalled();
    expect(os.text).toBe("dragged words");
    expect(appState.blockClipboard).toBeNull();
    span.remove();
  });
});

describe("where p lands", () => {
  it("on the zoom root, inside the zoomed subtree", async () => {
    setAppState({ focusBlockId: "blk-a", selectedBlockId: "blk-a" });
    await handlers().PasteClipboard?.();
    expect(pasteBlockAfter).toHaveBeenCalledWith(
      "pg-1",
      { parentId: "blk-a" },
      "old clipboard",
    );
  });

  it("zoomed with nothing selected, inside the zoomed subtree", async () => {
    setAppState({ focusBlockId: "blk-a", selectedBlockId: null });
    await handlers().PasteClipboard?.();
    expect(pasteBlockAfter).toHaveBeenCalledWith(
      "pg-1",
      { parentId: "blk-a" },
      "old clipboard",
    );
  });

  it("on an empty page, with no anchor, so the backend appends to the page", async () => {
    setAppState({ outline: [], selectedBlockId: null });
    await handlers().PasteClipboard?.();
    expect(pasteBlockAfter).toHaveBeenCalledWith(
      "pg-1",
      { afterId: null },
      "old clipboard",
    );
  });

  it("on a backlink row, nowhere, and the user is told why", async () => {
    setAppState({
      backlinks: [row("blk-x")],
      selectedBlockId: null,
      selectedBacklinkBlockId: "blk-x",
    });
    await handlers().PasteClipboard?.();

    expect(plugin.readText).not.toHaveBeenCalled();
    expect(pasteBlockAfter).not.toHaveBeenCalled();
    expect(setError).toHaveBeenCalledWith(PASTE_IN_OUTLINE);
  });

  it("yy on a backlink row copies the block that row shows", async () => {
    vi.mocked(copyMarkdown).mockResolvedValue("- cites [[today]]\n");
    setAppState({
      backlinks: [row("blk-x")],
      selectedBlockId: null,
      selectedBacklinkBlockId: "blk-x",
    });
    await handlers().YankCurrentBlock?.();

    expect(copyMarkdown).toHaveBeenCalledWith(["blk-x"]);
    expect(os.text).toBe("- cites [[today]]\n");
  });
});

describe("p while the user keeps moving", () => {
  it("is abandoned when the page changes during the clipboard read", async () => {
    const read = deferred<string>();
    plugin.readText.mockReturnValueOnce(read.promise);

    const paste = handlers().PasteClipboard?.();
    await Promise.resolve();
    setAppState("page", { id: "pg-2", slug: "other", title: "Other", kind: "page" });
    read.resolve("text");
    await paste;

    expect(pasteBlockAfter).not.toHaveBeenCalled();
  });

  it("does not paint a reply that comes back to a different page", async () => {
    const reply = deferred<{ view: PageView; new_id?: string }>();
    vi.mocked(pasteBlockAfter).mockReturnValueOnce(reply.promise);

    const paste = handlers().PasteClipboard?.();
    await vi.waitFor(() => expect(pasteBlockAfter).toHaveBeenCalled());
    setAppState("page", { id: "pg-2", slug: "other", title: "Other", kind: "page" });
    reply.resolve({ view: view(), new_id: "blk-new" });
    await paste;

    expect(applyView).not.toHaveBeenCalled();
    expect(appState.selectedBlockId).toBe("blk-a");
  });

  it("leaves the cursor where the user moved it during the IPC", async () => {
    const reply = deferred<{ view: PageView; new_id?: string }>();
    vi.mocked(pasteBlockAfter).mockReturnValueOnce(reply.promise);

    const paste = handlers().PasteClipboard?.();
    await vi.waitFor(() => expect(pasteBlockAfter).toHaveBeenCalled());
    setAppState("selectedBlockId", "blk-b");
    reply.resolve({ view: view(), new_id: "blk-new" });
    await paste;

    expect(applyView).toHaveBeenCalled();
    expect(appState.selectedBlockId).toBe("blk-b");
  });

  it("a cut moved during a navigation is still consumed", async () => {
    const moved = deferred<PageView>();
    vi.mocked(moveBlockAfter).mockReturnValueOnce(moved.promise);
    setAppState({
      blockClipboard: { kind: "cut", nodeId: "blk-b" },
      selectedBlockId: "blk-a",
    });

    const paste = handlers().PasteBlock?.();
    await vi.waitFor(() => expect(moveBlockAfter).toHaveBeenCalled());
    setAppState("page", { id: "pg-2", slug: "other", title: "Other", kind: "page" });
    moved.resolve(view());
    await paste;

    // The move is in the log; an armed cut would move it again.
    expect(appState.blockClipboard).toBeNull();
    expect(applyView).not.toHaveBeenCalled();
  });

  it("a repeat while a paste is running is dropped", async () => {
    const reply = deferred<{ view: PageView; new_id?: string }>();
    vi.mocked(pasteBlockAfter).mockReturnValueOnce(reply.promise);
    const h = handlers();

    const first = h.PasteClipboard?.();
    await vi.waitFor(() => expect(pasteBlockAfter).toHaveBeenCalled());
    await h.PasteClipboard?.();
    await h.PasteClipboard?.();
    reply.resolve({ view: view(), new_id: "blk-new" });
    await first;

    expect(pasteBlockAfter).toHaveBeenCalledTimes(1);
    // The gate opens again once it settles.
    await h.PasteClipboard?.();
    expect(pasteBlockAfter).toHaveBeenCalledTimes(2);
  });
});

describe("failures reach the status line", () => {
  it("an empty clipboard says so and sends nothing", async () => {
    os.text = "";
    await handlers().PasteClipboard?.();
    expect(setError).toHaveBeenCalledWith("Clipboard has no text");
    expect(pasteBlockAfter).not.toHaveBeenCalled();
  });

  it("a refused write on yy", async () => {
    vi.mocked(copyMarkdown).mockResolvedValue("- a\n");
    plugin.writeText.mockRejectedValueOnce("clipboard-manager.write_text not allowed");
    await handlers().YankCurrentBlock?.();
    expect(setError).toHaveBeenCalledWith(
      expect.stringContaining("write_text not allowed"),
    );
  });
});
