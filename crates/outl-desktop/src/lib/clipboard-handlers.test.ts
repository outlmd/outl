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
 * OS clipboard plugin. A `ctx` threaded to the wrong place would still
 * compile and would break exactly there — and a failure on either half
 * has to reach the status line, since a clipboard that silently kept
 * its old contents reads as a copy that worked.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { BlockNode } from "@outl/shared/api/types";

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => ({ close: vi.fn() })),
}));

vi.mock("@outl/shared/api/commands", () => ({
  copyBlockEmbed: vi.fn(),
  copyBlockMarkdown: vi.fn(),
  copyBlockRef: vi.fn(),
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

const plugin = vi.hoisted(() => ({
  readText: vi.fn(),
  writeText: vi.fn<(text: string) => Promise<void>>(),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => plugin);

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
  const writeText = plugin.writeText;

  beforeEach(() => {
    vi.clearAllMocks();
    writeText.mockResolvedValue(undefined);
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
      blockClipboard: null,
      mode: "vim-normal",
    });
  });

  it("Y copies the selected block's markdown and disarms a pending cut", async () => {
    vi.mocked(copyMarkdown).mockResolvedValue("- first block\n");
    setAppState({
      selectedBlockId: "blk-a",
      blockClipboard: { kind: "cut", nodeId: "blk-c" },
    });

    const handlers = buildHandlers({ applyView, setError });
    await handlers.YankCurrentBlock?.();

    expect(copyMarkdown).toHaveBeenCalledWith(["blk-a"]);
    expect(writeText).toHaveBeenCalledWith("- first block\n");
    // The latest gesture is what `p` pastes; a stale cut would win.
    expect(appState.blockClipboard).toBeNull();
    expect(setError).not.toHaveBeenCalled();
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

    expect(copyMarkdown).toHaveBeenCalledWith(["blk-a", "blk-b"]);
    expect(writeText).toHaveBeenCalledWith("- first block\n- second block\n");
    // Every Visual exit captures the range for `g v`.
    expect(appState.lastVisualRange).toEqual({ lo: "blk-a", hi: "blk-b" });
    expect(appState.mode).toBe("vim-normal");
  });

  it("a rejected clipboard write reaches the status line", async () => {
    vi.mocked(copyMarkdown).mockResolvedValue("- first block\n");
    writeText.mockRejectedValueOnce(new Error("no permission"));
    setAppState("selectedBlockId", "blk-a");

    const handlers = buildHandlers({ applyView, setError });
    await expect(handlers.YankCurrentBlock?.()).resolves.toBeUndefined();

    // Swallowed, this read as a copy that worked while the clipboard
    // kept its old contents — the bug `yy` shipped with.
    expect(setError).toHaveBeenCalledWith(
      expect.stringContaining("no permission"),
    );
  });

  it("a failed copy_markdown never writes to the clipboard", async () => {
    vi.mocked(copyMarkdown).mockRejectedValue(new Error("backend down"));
    setAppState("selectedBlockId", "blk-a");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.YankCurrentBlock?.();

    expect(writeText).not.toHaveBeenCalled();
    // `safeCall` is what surfaces it, which is the `ctx` wiring under test.
    expect(setError).toHaveBeenCalledWith("backend down");
  });
});
