import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { MFU_STORAGE_KEY, readCountsFromStore } from "@outl/shared/toolbar";

import {
  dispatchToolbarAction,
  type ToolbarHandlers,
} from "./Journal.toolbar-dispatch";

/** happy-dom ships no `localStorage`; the Tauri webview always has one. */
class MemStorage {
  private m = new Map<string, string>();
  get length() {
    return this.m.size;
  }
  clear() {
    this.m.clear();
  }
  getItem(k: string) {
    return this.m.has(k) ? (this.m.get(k) as string) : null;
  }
  setItem(k: string, v: string) {
    this.m.set(k, String(v));
  }
  removeItem(k: string) {
    this.m.delete(k);
  }
  key(i: number) {
    return [...this.m.keys()][i] ?? null;
  }
}

function handlers(editingId: string | null = "blk-1") {
  const calls: string[] = [];
  const note =
    (name: string) =>
    (...args: unknown[]) => {
      calls.push(args.length ? `${name}:${args.join(",")}` : name);
    };
  const h: ToolbarHandlers = {
    editingId: () => editingId,
    indent: note("indent"),
    outdent: note("outdent"),
    moveUp: note("moveUp"),
    moveDown: note("moveDown"),
    undo: note("undo"),
    redo: note("redo"),
    toggleTodo: note("toggleTodo"),
    delete: note("delete"),
    createAfter: note("createAfter"),
    appendBlock: note("appendBlock"),
    wrapSelection: note("wrap"),
    insertPair: note("pair"),
    insertText: note("text"),
    commitEdit: note("commit"),
  };
  return { h, calls };
}

describe("toolbar dispatch — routing", () => {
  beforeAll(() => vi.stubGlobal("localStorage", new MemStorage()));
  afterAll(() => vi.unstubAllGlobals());
  beforeEach(() => localStorage.clear());

  it("routes block ops to the block being edited", () => {
    const { h, calls } = handlers("blk-7");
    for (const a of ["indent", "outdent", "moveUp", "moveDown", "todo", "delete"]) {
      dispatchToolbarAction(a, h);
    }
    expect(calls).toEqual([
      "indent:blk-7",
      "outdent:blk-7",
      "moveUp:blk-7",
      "moveDown:blk-7",
      "toggleTodo:blk-7",
      "delete:blk-7",
    ]);
  });

  /** The bar can be up with nothing focused (the `+` path). A block op
   *  then has no subject and must be a no-op, not a crash. */
  it("drops block ops when nothing is being edited", () => {
    const { h, calls } = handlers(null);
    for (const a of ["indent", "outdent", "moveUp", "moveDown", "todo", "delete"]) {
      dispatchToolbarAction(a, h);
    }
    expect(calls).toEqual([]);
  });

  it("newLine appends at the page end when nothing is focused", () => {
    const focused = handlers("blk-7");
    dispatchToolbarAction("newLine", focused.h);
    expect(focused.calls).toEqual(["createAfter:blk-7"]);

    const idle = handlers(null);
    dispatchToolbarAction("newLine", idle.h);
    expect(idle.calls).toEqual(["appendBlock"]);
  });

  it("passes the formatting kind through verbatim", () => {
    const { h, calls } = handlers();
    for (const a of ["bold", "italic", "code"]) dispatchToolbarAction(a, h);
    expect(calls).toEqual(["wrap:bold", "wrap:italic", "wrap:code"]);
  });

  it("inserts the ref, block-ref and tag tokens", () => {
    const { h, calls } = handlers();
    dispatchToolbarAction("insertRef", h);
    dispatchToolbarAction("insertBlock", h);
    dispatchToolbarAction("insertHash", h);
    expect(calls).toEqual(["pair:[[,]]", "pair:((,))", "text:#"]);
  });

  it("undo and redo need no block", () => {
    const { h, calls } = handlers(null);
    dispatchToolbarAction("undo", h);
    dispatchToolbarAction("redo", h);
    expect(calls).toEqual(["undo", "redo"]);
  });

  it("ignores an id the catalog doesn't have", () => {
    const { h, calls } = handlers();
    dispatchToolbarAction("sendCarrierPigeon", h);
    expect(calls).toEqual([]);
  });
});

describe("toolbar dispatch — tap counting", () => {
  beforeAll(() => vi.stubGlobal("localStorage", new MemStorage()));
  afterAll(() => vi.unstubAllGlobals());
  beforeEach(() => localStorage.clear());

  /** This is the whole reason counting moved here: the iOS bar is
   *  native and routes through this function, so the web settings sheet
   *  can read the counts it produced. */
  it("counts a tap that arrived as a bare string from the iOS bridge", () => {
    const { h } = handlers();
    dispatchToolbarAction("bold", h);
    dispatchToolbarAction("bold", h);
    expect(readCountsFromStore().bold).toBe(2);
  });

  /** An unknown id would otherwise sit in the store forever, since
   *  nothing ever prunes it. */
  it("does not count an id outside the catalog", () => {
    const { h } = handlers();
    dispatchToolbarAction("sendCarrierPigeon", h);
    expect(localStorage.getItem(MFU_STORAGE_KEY)).toBeNull();
  });

  /** Pinned slots are positional, so counting them is dead weight. */
  it("does not count the pinned slots", () => {
    const { h } = handlers();
    dispatchToolbarAction("newLine", h);
    dispatchToolbarAction("done", h);
    const counts = readCountsFromStore();
    expect(counts.newLine).toBeUndefined();
    expect(counts.done).toBeUndefined();
  });

  /** A block op with no subject still counts: the user pressed the
   *  button, which is what MFU is measuring. */
  it("counts the tap even when the action itself is a no-op", () => {
    const { h } = handlers(null);
    dispatchToolbarAction("indent", h);
    expect(readCountsFromStore().indent).toBe(1);
  });
});
