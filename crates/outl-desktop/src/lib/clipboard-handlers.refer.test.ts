/**
 * `/refer` and `/refer-embed` on the desktop.
 *
 * `outl_shortcuts::support` now says the desktop performs
 * `CopyBlockRef` **and** `CopyBlockEmbed`, so `shortcuts.support.test.ts`
 * already fails when either handler is missing. What it cannot see is
 * whether the handler does the right thing, and there are exactly two
 * ways to get this wrong without it noticing:
 *
 * - **the wrong command**, since both take one block id and answer one
 *   string — an embed handler calling `copy_block_ref` copies a ref
 *   that looks correct and silently never expands;
 * - **a swallowed refusal.** The backend rejects with "no ref handle
 *   yet — save and retry" for a block created this session and not yet
 *   projected to disk. That is the one state the user has to be told
 *   about, because the fix is theirs (wait, or edit something) and the
 *   clipboard otherwise still holds whatever was in it before.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

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

vi.mock("./api", () => ({
  redoPage: vi.fn(),
  undoPage: vi.fn(),
}));

import { copyBlockEmbed, copyBlockRef } from "@outl/shared/api/commands";

import { buildHandlers } from "./action-handlers";
import { setAppState } from "./store";

describe("the block-handle handlers reach the OS clipboard", () => {
  const applyView = vi.fn();
  const setError = vi.fn();
  const writeText = vi.fn(() => Promise.resolve());

  beforeEach(() => {
    vi.clearAllMocks();
    // Absent under happy-dom, and the handler reads it optionally.
    Object.defineProperty(globalThis.navigator, "clipboard", {
      value: { writeText },
      configurable: true,
      writable: true,
    });
    setAppState({ selectedBlockId: "blk-a" });
  });

  it("CopyBlockRef copies the `((blk-x))` handle of the selected block", async () => {
    vi.mocked(copyBlockRef).mockResolvedValue("((blk-abc123))");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.CopyBlockRef?.();

    expect(copyBlockRef).toHaveBeenCalledWith("blk-a");
    expect(copyBlockEmbed).not.toHaveBeenCalled();
    expect(writeText).toHaveBeenCalledWith("((blk-abc123))");
    expect(setError).not.toHaveBeenCalled();
  });

  it("CopyBlockEmbed copies the `!((blk-x))` spelling, from its own command", async () => {
    vi.mocked(copyBlockEmbed).mockResolvedValue("!((blk-abc123))");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.CopyBlockEmbed?.();

    // The `!` comes from `outl_md::inline::block_embed_token`. A client
    // that built it by prefixing the ref would pass a looser assertion
    // and be a second owner of the syntax.
    expect(copyBlockEmbed).toHaveBeenCalledWith("blk-a");
    expect(copyBlockRef).not.toHaveBeenCalled();
    expect(writeText).toHaveBeenCalledWith("!((blk-abc123))");
  });

  it("neither fires without a selected block", async () => {
    setAppState({ selectedBlockId: null });

    const handlers = buildHandlers({ applyView, setError });
    await handlers.CopyBlockRef?.();
    await handlers.CopyBlockEmbed?.();

    expect(copyBlockRef).not.toHaveBeenCalled();
    expect(copyBlockEmbed).not.toHaveBeenCalled();
    expect(writeText).not.toHaveBeenCalled();
  });

  it("surfaces 'no ref handle yet' instead of copying", async () => {
    const refusal = "no ref handle yet — save and retry";
    vi.mocked(copyBlockRef).mockRejectedValue(new Error(refusal));

    const handlers = buildHandlers({ applyView, setError });
    await handlers.CopyBlockRef?.();

    expect(setError).toHaveBeenCalledWith(refusal);
    // The clipboard keeps whatever it held; a write here would replace
    // it with nothing and read as a copy that worked.
    expect(writeText).not.toHaveBeenCalled();
  });

  it("the same refusal on the embed path also reaches the user", async () => {
    const refusal = "no ref handle yet — save and retry";
    vi.mocked(copyBlockEmbed).mockRejectedValue(new Error(refusal));

    const handlers = buildHandlers({ applyView, setError });
    await handlers.CopyBlockEmbed?.();

    expect(setError).toHaveBeenCalledWith(refusal);
    expect(writeText).not.toHaveBeenCalled();
  });

  it("a webview that refuses the clipboard does not raise", async () => {
    vi.mocked(copyBlockRef).mockResolvedValue("((blk-abc123))");
    writeText.mockReturnValueOnce(Promise.reject(new Error("no permission")));

    const handlers = buildHandlers({ applyView, setError });
    await expect(handlers.CopyBlockRef?.()).resolves.toBeUndefined();
    await Promise.resolve();

    // Best-effort, like every other clipboard write in this client.
    expect(setError).not.toHaveBeenCalled();
  });
});
