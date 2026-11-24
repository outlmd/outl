/**
 * `buildContextActions` (in `Journal.context-actions.ts`, extracted out
 * of `Journal.tsx`) is what decides which rows the block long-press
 * menu shows. This
 * file covers the RFC 0254 phase 2 addition — "Copy block" / "Paste
 * block" — and the one thing worth pinning: "Paste block" is armed
 * ("Copy block" always is), not the other way round, and it never
 * collapses with the pre-existing "Copy text" ( = `YankCurrentBlock`)
 * row. Everything else `buildContextActions` does (move-up/down
 * guards, the fenced-code "Run" row) predates this phase and is
 * exercised in production, not re-covered here.
 */

import { describe, expect, it, vi } from "vitest";

// No Tauri mocks: the module under test imports only `@outl/shared`
// (`detectFence`, `findBlock`) and a type. It used to live in
// `Journal.tsx`, which does pull the real `@tauri-apps/plugin-*`
// packages at module scope, and the mocks below existed for that. If
// re-adding one is ever necessary here, that is the signal a Tauri
// dependency leaked into what is meant to stay a pure function.

import {
  buildBacklinkContextActions,
  buildContextActions,
} from "./Journal.context-actions";
import type { Backlink, BlockNode, PageView } from "@outl/shared/api/types";

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
    copyBlockEmbed: vi.fn(),
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

  /**
   * The embed is the ref's twin, not a variant of it: same handle,
   * different spelling, different thing on the page it lands on. The
   * two rows therefore sit together, and the pin that matters is the
   * adjacency — a third row wandering between them is how a user comes
   * to tap the wrong one by muscle memory.
   */
  it("offers Copy block embed directly after Copy block ref", () => {
    const rows = ids(buildContextActions(BLOCK_ID, pageView([block()]), handlers()));
    const ref = rows.indexOf("copyBlockRef");
    expect(ref).toBeGreaterThanOrEqual(0);
    expect(rows[ref + 1]).toBe("copyBlockEmbed");
  });

  it("Copy block embed dispatches to copyBlockEmbed alone", () => {
    const h = handlers();
    const actions = buildContextActions(BLOCK_ID, pageView([block()]), h);
    const embed = actions.find((a) => a.id === "copyBlockEmbed")!;
    embed.onSelect();
    expect(h.copyBlockEmbed).toHaveBeenCalledWith(BLOCK_ID);
    // `!((blk-…))` is not "the ref, plus a character" on this side
    // either: the row that writes it calls its own command.
    expect(h.copyBlockRef).not.toHaveBeenCalled();
    expect(h.copy).not.toHaveBeenCalled();
  });

  it("draws the embed row with its own glyph, and arms no keyboard", () => {
    const actions = buildContextActions(BLOCK_ID, pageView([block()]), handlers());
    const embed = actions.find((a) => a.id === "copyBlockEmbed")!;
    const ref = actions.find((a) => a.id === "copyBlockRef")!;
    // Two rows that do different things to the paste cannot look the
    // same — the sheet shows a glyph and a label, nothing else.
    expect(embed.iconPath).not.toBe(ref.iconPath);
    expect(embed.label).toBe("Copy block embed");
    // Nothing is typed after a clipboard write, so raising the
    // keyboard would put it over a sheet that is closing.
    expect(embed.armsKeyboard).toBeUndefined();
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

/**
 * A long-press on a **backlink row** reaches the same sheet, because
 * mobile has no chord surface at all — so where the desktop binds a
 * key, this client offers a row. The list is a different question from
 * the outline's, though, and the differences are the point:
 *
 * - **An orphan backlink offers nothing.** `backlinkEditTarget` is
 *   `null` there, so there is no page to address a mutation to, and
 *   guessing one writes to the wrong document.
 * - **Move up / down carry no `enabled` guard.** The row is one flat
 *   block with no sibling list rendered, so the position the outline's
 *   builder resolves does not exist here. Hiding them on a guess would
 *   be a worse answer than offering them.
 * - **Nothing creates a block.** See the test for why.
 */
describe("buildBacklinkContextActions", () => {
  function link(over: Partial<Backlink> = {}): Backlink {
    return {
      block_id: "b1",
      todo: null,
      ancestors: [],
      source_page: {
        id: "p1",
        slug: "journal",
        title: "Journal",
        kind: "journal",
      },
      source_block: {
        id: "b1",
        text: "cites [[project]]",
        todo: null,
        tokens: [],
        collapsed: false,
        properties: [],
        children: [],
      },
      source_block_path: [],
      ...over,
    } as unknown as Backlink;
  }

  function backlinkHandlers() {
    return {
      toggleTodo: vi.fn(),
      indent: vi.fn(),
      outdent: vi.fn(),
      moveUp: vi.fn(),
      moveDown: vi.fn(),
      delete: vi.fn(),
      createBelow: vi.fn(),
      createAbove: vi.fn(),
    };
  }

  it("a backlink row's context actions target the source page", () => {
    const h = backlinkHandlers();
    const l = link();
    const actions = buildBacklinkContextActions(l, l.source_block, h);

    // Every row hands the whole `Backlink` back, which is what carries
    // `source_page.id`. An action that took a bare block id would have
    // to be told which page separately, and the only page this client
    // holds in that scope is the one on screen — the wrong one.
    for (const action of actions) action.onSelect();

    expect(h.toggleTodo).toHaveBeenCalledWith(l, l.source_block);
    expect(h.indent).toHaveBeenCalledWith(l, l.source_block);
    expect(h.outdent).toHaveBeenCalledWith(l, l.source_block);
    expect(h.moveUp).toHaveBeenCalledWith(l, l.source_block);
    expect(h.moveDown).toHaveBeenCalledWith(l, l.source_block);
    expect(h.delete).toHaveBeenCalledWith(l, l.source_block);
  });

  it("offers every operation the row supports, text aside", () => {
    const actions = buildBacklinkContextActions(link(), link().source_block, backlinkHandlers());

    // Parity with the TUI's `Focus::Backlink` chords and the desktop's
    // `BacklinkRowOps` — the text edit is missing because the tap is
    // the gesture for it.
    expect(ids(actions)).toEqual([
      "toggleTodo",
      "newBlockBelow",
      "newBlockAbove",
      "indent",
      "outdent",
      "moveUp",
      "moveDown",
      "delete",
    ]);
  });

  it("an orphan backlink offers no structural actions", () => {
    const actions = buildBacklinkContextActions(
      link({ source_page: null }),
      link().source_block,
      backlinkHandlers(),
    );

    expect(actions).toEqual([]);
  });

  it("returns nothing with no row long-pressed (menu closed)", () => {
    expect(buildBacklinkContextActions(null, null, backlinkHandlers())).toEqual([]);
  });

  it("creates on the source page, below and above", () => {
    const h = backlinkHandlers();
    const l = link();
    const actions = buildBacklinkContextActions(l, l.source_block, h);

    actions.find((a) => a.id === "newBlockBelow")!.onSelect();
    expect(h.createBelow).toHaveBeenCalledWith(l, l.source_block);
    expect(h.createAbove).not.toHaveBeenCalled();

    actions.find((a) => a.id === "newBlockAbove")!.onSelect();
    expect(h.createAbove).toHaveBeenCalledWith(l, l.source_block);
  });

  it("no row arms the keyboard, the create rows included", () => {
    const actions = buildBacklinkContextActions(link(), link().source_block, backlinkHandlers());

    // `armsKeyboard` is the outline's contract for an action that ends
    // with a caret in a textarea. A block created here cites nothing,
    // so it is a backlink of no page, appears in no row on this screen
    // and has no editor to focus — arming the keyboard would raise it
    // over nothing. The block is still created, and the user is told
    // where it went (`backlinkCreatedElsewhere`): a declared limit
    // beats a wrong target, which is the TUI's answer to the same wall.
    expect(actions.some((a) => a.armsKeyboard)).toBe(false);
  });

  it("keeps both moves offered, since a flat row has no sibling list", () => {
    const actions = buildBacklinkContextActions(link(), link().source_block, backlinkHandlers());

    for (const id of ["moveUp", "moveDown"]) {
      const action = actions.find((a) => a.id === id)!;
      expect(action.enabled).toBeUndefined();
    }
  });

  it("marks only delete destructive, as the outline's menu does", () => {
    const actions = buildBacklinkContextActions(link(), link().source_block, backlinkHandlers());

    expect(actions.filter((a) => a.destructive).map((a) => a.id)).toEqual([
      "delete",
    ]);
  });

  it("draws each action with the glyph the outline's menu uses", () => {
    const outline = buildContextActions(BLOCK_ID, pageView([block()]), handlers());
    const rows = buildBacklinkContextActions(link(), link().source_block, backlinkHandlers());

    // One action, one glyph, whichever sheet it was reached from — two
    // icon literals for "Indent" is how the same action comes to look
    // like two different ones.
    let compared = 0;
    for (const row of rows) {
      const twin = outline.find((a) => a.id === row.id);
      // `newBlockBelow` has no twin: the outline's sheet offers only
      // "New block above" (its Enter-at-end gesture covers below).
      if (!twin) continue;
      expect(row.iconPath, row.id).toBe(twin.iconPath);
      expect(row.label, row.id).toBe(twin.label);
      compared += 1;
    }
    // Without this the loop passes by comparing nothing, which is what
    // a renamed id would silently turn it into.
    expect(compared).toBe(7);
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
