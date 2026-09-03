/**
 * `buildContextActions` (exported from `Journal.tsx` for exactly this)
 * is what decides which rows the block long-press menu shows. This
 * file covers the RFC 0254 phase 2 addition — "Copy block" / "Paste
 * block" — and the one thing worth pinning: "Paste block" is armed
 * ("Copy block" always is), not the other way round, and it never
 * collapses with the pre-existing "Copy text" ( = `YankCurrentBlock`)
 * row. Everything else `buildContextActions` does (move-up/down
 * guards, the fenced-code "Run" row) predates this phase and is
 * exercised in production, not re-covered here.
 */

import { describe, expect, it, vi } from "vitest";

// `Journal.tsx` imports real `@tauri-apps/plugin-*` packages and the
// full `@outl/shared/api/commands` surface at module scope — none of
// that runs at import time (every export is a lazy `invoke()` call),
// but mocking the Tauri dialog module keeps this test from depending
// on that staying true if a future refactor moves work to module load.
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-os", () => ({ platform: () => "ios" }));

import { buildContextActions } from "./Journal";
import type { BlockNode, PageView } from "@outl/shared/api/types";

const BLOCK_ID = "blk-1";

function block(overrides: Partial<BlockNode> = {}): BlockNode {
  return {
    id: BLOCK_ID,
    text: "a block",
    todo: null,
    tokens: [],
    collapsed: false,
    properties: [],
    children: [],
    ...overrides,
  };
}

function pageView(blocks: BlockNode[]): PageView {
  return {
    page: { id: "pg-1", slug: "inbox", title: "Inbox", kind: "page" },
    outline: blocks,
    backlinks: [],
    backlinks_order: "newest",
    page_properties: [],
  } as unknown as PageView;
}

function handlers() {
  return {
    indent: vi.fn(),
    outdent: vi.fn(),
    moveUp: vi.fn(),
    moveDown: vi.fn(),
    toggleTodo: vi.fn(),
    delete: vi.fn(),
    runCode: vi.fn(),
    insertTemplate: vi.fn(),
    properties: vi.fn(),
    remindMe: vi.fn(),
    copy: vi.fn(),
    copyBlock: vi.fn(),
    pasteBlock: vi.fn(),
    cutBlock: vi.fn(),
    copyBlockRef: vi.fn(),
    newBlockAbove: vi.fn(),
    attachFile: vi.fn(),
    selectBlocks: vi.fn(),
    reselectSelection: vi.fn(),
  };
}

function ids(actions: { id: string }[]): string[] {
  return actions.map((a) => a.id);
}

describe("buildContextActions — block clipboard (RFC 0254 phase 2)", () => {
  it("always offers Copy block, alongside the pre-existing Copy text", () => {
    const actions = buildContextActions(
      BLOCK_ID,
      pageView([block()]),
      handlers(),
      false,
    );
    expect(ids(actions)).toContain("copy");
    expect(ids(actions)).toContain("copyBlock");
  });

  it("hides Paste block until the clipboard is armed", () => {
    const actions = buildContextActions(
      BLOCK_ID,
      pageView([block()]),
      handlers(),
      false,
    );
    expect(ids(actions)).not.toContain("pasteBlock");
  });

  it("shows Paste block once armed, and it dispatches to the right handler", () => {
    const h = handlers();
    const actions = buildContextActions(
      BLOCK_ID,
      pageView([block()]),
      h,
      true,
    );
    const paste = actions.find((a) => a.id === "pasteBlock");
    expect(paste).toBeTruthy();
    paste!.onSelect();
    expect(h.pasteBlock).toHaveBeenCalledWith(BLOCK_ID);
    expect(h.copyBlock).not.toHaveBeenCalled();
  });

  it("Copy block dispatches to copyBlock, never to the OS-clipboard copy handler", () => {
    const h = handlers();
    const actions = buildContextActions(
      BLOCK_ID,
      pageView([block()]),
      h,
      false,
    );
    const copyBlock = actions.find((a) => a.id === "copyBlock");
    copyBlock!.onSelect();
    expect(h.copyBlock).toHaveBeenCalledWith(BLOCK_ID);
    expect(h.copy).not.toHaveBeenCalled();
  });

  it("returns nothing for a null blockId (menu closed)", () => {
    expect(buildContextActions(null, pageView([block()]), handlers(), true))
      .toEqual([]);
  });
});

describe("buildContextActions — cut / ref / new-above (RFC 0254 phase 4b)", () => {
  it("always offers Cut block, and it dispatches to cutBlock alone", () => {
    const h = handlers();
    const actions = buildContextActions(BLOCK_ID, pageView([block()]), h);
    const cut = actions.find((a) => a.id === "cutBlock");
    expect(cut).toBeTruthy();
    cut!.onSelect();
    expect(h.cutBlock).toHaveBeenCalledWith(BLOCK_ID);
    expect(h.copyBlock).not.toHaveBeenCalled();
    expect(h.delete).not.toHaveBeenCalled();
  });

  it("always offers Copy block ref, dispatching to copyBlockRef alone", () => {
    const h = handlers();
    const actions = buildContextActions(BLOCK_ID, pageView([block()]), h);
    const ref = actions.find((a) => a.id === "copyBlockRef");
    expect(ref).toBeTruthy();
    ref!.onSelect();
    expect(h.copyBlockRef).toHaveBeenCalledWith(BLOCK_ID);
    expect(h.copy).not.toHaveBeenCalled();
  });

  it("always offers New block above, dispatching to newBlockAbove", () => {
    const h = handlers();
    const actions = buildContextActions(BLOCK_ID, pageView([block()]), h);
    const above = actions.find((a) => a.id === "newBlockAbove");
    expect(above).toBeTruthy();
    above!.onSelect();
    expect(h.newBlockAbove).toHaveBeenCalledWith(BLOCK_ID);
  });
});

describe("buildContextActions — range selection (RFC 0254 phase 3)", () => {
  it("always offers Select blocks", () => {
    const actions = buildContextActions(BLOCK_ID, pageView([block()]), handlers());
    expect(ids(actions)).toContain("selectBlocks");
  });

  it("Select blocks dispatches to selectBlocks with this block's id", () => {
    const h = handlers();
    const actions = buildContextActions(BLOCK_ID, pageView([block()]), h);
    actions.find((a) => a.id === "selectBlocks")!.onSelect();
    expect(h.selectBlocks).toHaveBeenCalledWith(BLOCK_ID);
  });

  it("hides Reselect last selection until a reselectable range exists", () => {
    const actions = buildContextActions(
      BLOCK_ID,
      pageView([block()]),
      handlers(),
      false,
      false,
    );
    expect(ids(actions)).not.toContain("reselectSelection");
  });

  it("shows Reselect last selection once one is live, and dispatches with no id", () => {
    const h = handlers();
    const actions = buildContextActions(
      BLOCK_ID,
      pageView([block()]),
      h,
      false,
      true,
    );
    const reselect = actions.find((a) => a.id === "reselectSelection");
    expect(reselect).toBeTruthy();
    reselect!.onSelect();
    expect(h.reselectSelection).toHaveBeenCalledWith();
    expect(h.selectBlocks).not.toHaveBeenCalled();
  });
});
