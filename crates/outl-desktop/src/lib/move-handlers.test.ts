/**
 * Normal-mode move (`MoveBlockUp` / `MoveBlockDown`): the outline half
 * of the handler, which goes through the shared `moveOnce` gate,
 * refuses the zoom root and scrolls the moved row into view.
 *
 * The chord-to-handler half lives in
 * `components/BlockRow.move-dispatch.test.tsx`.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { BlockNode, PageView } from "@outl/shared/api/types";

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => ({ close: vi.fn() })),
}));

vi.mock("@outl/shared/api/commands", () => ({
  indentBlock: vi.fn(),
  moveBlockDown: vi.fn(),
  moveBlockUp: vi.fn(),
  outdentBlock: vi.fn(),
  deleteBlock: vi.fn(),
}));

vi.mock("./api", () => ({
  redoPage: vi.fn(),
  runCodeBlock: vi.fn(),
  undoPage: vi.fn(),
}));

import { moveBlockDown, moveBlockUp } from "@outl/shared/api/commands";

import { buildHandlers } from "./action-handlers";
import { appState, setAppState } from "./store";

function block(id: string): BlockNode {
  return {
    id,
    text: id,
    todo: null,
    tokens: [],
    collapsed: false,
    properties: [],
    children: [],
  };
}

const view = { page: {}, outline: [] } as unknown as PageView;
const applyView = vi.fn();
const setError = vi.fn();
const settle: Array<() => void> = [];

beforeEach(() => {
  vi.clearAllMocks();
  setAppState({
    page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
    outline: [block("a"), block("b"), block("c")],
    backlinks: [],
    selectedBlockId: "b",
    focusBlockId: null,
    editingBlockId: null,
  });
});

afterEach(async () => {
  while (settle.length) settle.pop()?.();
  await Promise.resolve();
  document.body.innerHTML = "";
});

describe("Normal: MoveBlockUp / MoveBlockDown", () => {
  it("moves the selected block and applies the view", async () => {
    vi.mocked(moveBlockUp).mockResolvedValue(view);
    vi.mocked(moveBlockDown).mockResolvedValue(view);
    const handlers = buildHandlers({ applyView, setError });

    await handlers.MoveBlockUp?.();
    await handlers.MoveBlockDown?.();

    expect(moveBlockUp).toHaveBeenCalledWith("pg-1", "b");
    expect(moveBlockDown).toHaveBeenCalledWith("pg-1", "b");
    expect(applyView).toHaveBeenCalledTimes(2);
  });

  it("does not move the zoomed block (appState.focusBlockId)", async () => {
    setAppState("focusBlockId", "b");
    const handlers = buildHandlers({ applyView, setError });

    await handlers.MoveBlockUp?.();
    await handlers.MoveBlockDown?.();

    expect(moveBlockUp).not.toHaveBeenCalled();
    expect(moveBlockDown).not.toHaveBeenCalled();
  });

  it("scrolls the moved row into view", async () => {
    vi.mocked(moveBlockUp).mockResolvedValue(view);
    const row = document.createElement("div");
    row.dataset.blockId = "b";
    const scroll = vi.fn();
    row.scrollIntoView = scroll;
    document.body.appendChild(row);

    await buildHandlers({ applyView, setError }).MoveBlockUp?.();
    await Promise.resolve();

    expect(scroll).toHaveBeenCalledWith({ block: "nearest" });
  });

  it("drops a second move while the first is in flight", async () => {
    let release!: (v: PageView) => void;
    vi.mocked(moveBlockUp).mockReturnValue(
      new Promise<PageView>((r) => {
        release = r;
        settle.push(() => r(view));
      }),
    );
    const handlers = buildHandlers({ applyView, setError });

    const first = handlers.MoveBlockUp?.();
    await handlers.MoveBlockUp?.();
    expect(moveBlockUp).toHaveBeenCalledTimes(1);

    release(view);
    await first;
    expect(appState.selectedBlockId).toBe("b");
  });
});
