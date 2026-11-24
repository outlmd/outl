/**
 * Visual range move: which blocks a `MoveVisualRange*` issues commands
 * for. The walk-order tests (indent, `reverse` for down) stay in
 * `action-handlers.test.ts`; these pin the range-as-roots rule the
 * move ops added.
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
import { setAppState } from "./store";

function block(id: string, children: BlockNode[] = []): BlockNode {
  return {
    id,
    text: id,
    todo: null,
    tokens: [],
    collapsed: false,
    properties: [],
    children,
  };
}

const view = { page: {}, outline: [] } as unknown as PageView;
const applyView = vi.fn();
const setError = vi.fn();
const settle: Array<() => void> = [];

function select(anchor: string, cursor: string) {
  setAppState({
    visualAnchorId: anchor,
    selectedBlockId: cursor,
    mode: "vim-visual",
  });
}

function calls(fn: typeof moveBlockUp): string[] {
  return vi.mocked(fn).mock.calls.map((c) => c[1]);
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(moveBlockUp).mockResolvedValue(view);
  vi.mocked(moveBlockDown).mockResolvedValue(view);
  setAppState({
    page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
    outline: [
      block("a"),
      block("b"),
      block("c"),
      block("d", [block("d1"), block("d2")]),
      block("e"),
    ],
    backlinks: [],
    focusBlockId: null,
    editingBlockId: null,
  });
});

afterEach(async () => {
  while (settle.length) settle.pop()?.();
  await Promise.resolve();
});

describe("MoveVisualRangeUp / Down", () => {
  it("leaves b,c: up walks top-down, down walks bottom-up", async () => {
    const h = buildHandlers({ applyView, setError });
    select("b", "c");
    await h.MoveVisualRangeUp?.();
    expect(calls(moveBlockUp)).toEqual(["b", "c"]);

    await h.MoveVisualRangeDown?.();
    expect(calls(moveBlockDown)).toEqual(["c", "b"]);
  });

  it("a parent with its children issues one command", async () => {
    const h = buildHandlers({ applyView, setError });
    select("d", "d2");
    await h.MoveVisualRangeUp?.();
    expect(calls(moveBlockUp)).toEqual(["d"]);
  });

  it("a range at the edge issues none", async () => {
    const h = buildHandlers({ applyView, setError });
    select("a", "b");
    await h.MoveVisualRangeUp?.();
    select("d", "e");
    await h.MoveVisualRangeDown?.();
    expect(moveBlockUp).not.toHaveBeenCalled();
    expect(moveBlockDown).not.toHaveBeenCalled();
  });

  it("a range starting on a child and running past its parent issues none", async () => {
    const h = buildHandlers({ applyView, setError });
    select("d2", "e");
    await h.MoveVisualRangeUp?.();
    await h.MoveVisualRangeDown?.();
    expect(moveBlockUp).not.toHaveBeenCalled();
    expect(moveBlockDown).not.toHaveBeenCalled();
  });

  it("two un-awaited Up calls on b,c issue exactly b,c", async () => {
    let release!: () => void;
    vi.mocked(moveBlockUp).mockImplementation(
      () =>
        new Promise<PageView>((r) => {
          release = () => r(view);
          settle.push(release);
        }),
    );
    const h = buildHandlers({ applyView, setError });
    select("b", "c");

    const first = h.MoveVisualRangeUp?.();
    const second = h.MoveVisualRangeUp?.();
    await second;
    expect(calls(moveBlockUp)).toEqual(["b"]);
    release();
    await vi.waitFor(() => expect(moveBlockUp).toHaveBeenCalledTimes(2));
    release();
    await first;
    expect(calls(moveBlockUp)).toEqual(["b", "c"]);
  });

  it("a range holding appState.focusBlockId issues none", async () => {
    setAppState("focusBlockId", "b");
    const h = buildHandlers({ applyView, setError });
    select("b", "c");
    await h.MoveVisualRangeUp?.();
    expect(moveBlockUp).not.toHaveBeenCalled();
  });
});
