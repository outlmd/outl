/**
 * Pins the two handlers whose bodies the `action-handlers.ts` split
 * actually changed: `YankCurrentBlock` and `YankRange`.
 *
 * Every other handler moved byte-identical, so the unchanged suite is
 * their proof. These two gained an argument — `copySelectionToClipboard`
 * is now a module-level export taking the `HandlerCtx` it used to close
 * over — and *no* test mentioned either one, so the split shipped with
 * zero behavioural evidence on the only lines it edited.
 *
 * What matters is the whole round-trip, not the call: the block ids
 * reach `copy_markdown`, and the markdown it answers with reaches the
 * OS clipboard. A `ctx` threaded to the wrong place would still
 * compile and would break exactly there, silently, because both halves
 * are deliberately best-effort and swallow failure.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { BlockNode } from "@outl/shared/api/types";

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => ({ close: vi.fn() })),
}));

vi.mock("@outl/shared/api/commands", () => ({
  copyBlockMarkdown: vi.fn(),
  copyMarkdown: vi.fn(),
  createBlock: vi.fn(),
  deleteBlock: vi.fn(),
  deletePage: vi.fn(),
  editBlock: vi.fn(),
  indentBlock: vi.fn(),
  moveBlockAfter: vi.fn(),
  moveBlockDown: vi.fn(),
  moveBlockUp: vi.fn(),
  nextDay: vi.fn(),
  openJournalFor: vi.fn(),
  openRef: vi.fn(),
  openTodayJournal: vi.fn(),
  outdentBlock: vi.fn(),
  pasteBlockAfter: vi.fn(),
  pluginSyncHooks: vi.fn(),
  previousDay: vi.fn(),
  runCodeBlock: vi.fn(),
  setBlockCollapsed: vi.fn(),
  setBlockRemind: vi.fn(),
  snoozeReminder: vi.fn(),
  todaySlug: vi.fn(),
  toggleTodo: vi.fn(),
}));

vi.mock("./api", () => ({
  redoPage: vi.fn(),
  undoPage: vi.fn(),
}));

import { copyMarkdown } from "@outl/shared/api/commands";

import { buildHandlers } from "./action-handlers";
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

describe("yank handlers reach the OS clipboard", () => {
  const applyView = vi.fn();
  const setError = vi.fn();
  const writeText = vi.fn(() => Promise.resolve());

  beforeEach(() => {
    vi.clearAllMocks();
    // `navigator.clipboard` is absent under happy-dom; the handler
    // reads it optionally (`?.`) so it has to be installed to observe.
    Object.defineProperty(globalThis.navigator, "clipboard", {
      value: { writeText },
      configurable: true,
      writable: true,
    });
    setAppState({
      page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
      outline: [
        block("blk-a", "first block"),
        block("blk-b", "second block"),
        block("blk-c", "third block"),
      ],
      backlinks: [],
      selectedBlockId: null,
      selectedBacklinkBlockId: null,
      visualAnchorId: null,
      lastVisualRange: null,
      yankRegister: [],
      mode: "vim-normal",
    });
  });

  it("Y copies the selected block's markdown and fills the yank register", async () => {
    vi.mocked(copyMarkdown).mockResolvedValue("- first block\n");
    setAppState("selectedBlockId", "blk-a");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.YankCurrentBlock?.();
    // The clipboard write is fire-and-forget inside the handler.
    await Promise.resolve();
    await Promise.resolve();

    expect(copyMarkdown).toHaveBeenCalledWith(["blk-a"]);
    expect(writeText).toHaveBeenCalledWith("- first block\n");
    // The in-app register holds the block text, not the markdown.
    expect(appState.yankRegister).toEqual(["first block"]);
  });

  it("Y does nothing when no block is selected", async () => {
    const handlers = buildHandlers({ applyView, setError });
    await handlers.YankCurrentBlock?.();

    expect(copyMarkdown).not.toHaveBeenCalled();
    expect(writeText).not.toHaveBeenCalled();
  });

  it("Visual y copies the whole range and leaves Visual mode", async () => {
    vi.mocked(copyMarkdown).mockResolvedValue("- first block\n- second block\n");
    setAppState({
      visualAnchorId: "blk-a",
      selectedBlockId: "blk-b",
      mode: "vim-visual",
    });

    const handlers = buildHandlers({ applyView, setError });
    await handlers.YankRange?.();
    await Promise.resolve();
    await Promise.resolve();

    expect(copyMarkdown).toHaveBeenCalledWith(["blk-a", "blk-b"]);
    expect(writeText).toHaveBeenCalledWith("- first block\n- second block\n");
    expect(appState.yankRegister).toEqual(["first block", "second block"]);
    // Every Visual exit captures the range for `g v`.
    expect(appState.lastVisualRange).toEqual({ lo: "blk-a", hi: "blk-b" });
    expect(appState.mode).toBe("vim-normal");
  });

  it("a rejected clipboard write still leaves the yank register filled", async () => {
    vi.mocked(copyMarkdown).mockResolvedValue("- first block\n");
    writeText.mockReturnValueOnce(Promise.reject(new Error("no permission")));
    setAppState("selectedBlockId", "blk-a");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.YankCurrentBlock?.();
    await Promise.resolve();
    await Promise.resolve();

    // Both halves are best-effort: a webview that refuses the clipboard
    // must not cost the user `p` / `P`, and must not raise.
    expect(appState.yankRegister).toEqual(["first block"]);
    expect(setError).not.toHaveBeenCalled();
  });

  it("a failed copy_markdown never writes to the clipboard", async () => {
    vi.mocked(copyMarkdown).mockRejectedValue(new Error("backend down"));
    setAppState("selectedBlockId", "blk-a");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.YankCurrentBlock?.();
    await Promise.resolve();
    await Promise.resolve();

    expect(writeText).not.toHaveBeenCalled();
    // `safeCall` is what surfaces it, which is the `ctx` wiring under test.
    expect(setError).toHaveBeenCalledWith("backend down");
  });
});
