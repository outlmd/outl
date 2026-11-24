/**
 * `isEditing()` — the one answer five background refreshes ask before
 * they re-materialize the tree.
 *
 * Each of them (a peer streaming ops, the reconcile pass, the
 * capture-inbox drain, the midnight jump to today, and the drain that
 * waits for the user to leave) used to read `appState.editingBlockId`
 * directly. That field names the **outline's** editor only, so making a
 * backlink row editable put a textarea inside the one list nothing
 * protected: a refetch replaces `appState.backlinks` with fresh objects,
 * `<For>` keys by reference, the row remounts, and the draft re-seeds
 * from disk over whatever the user had typed.
 *
 * The second case here is the half that is easy to get wrong in the
 * other direction. The editor is rendered *by the row*, so a reply that
 * drops the row unmounts the editor while `editingBacklink` stays set —
 * and a guard that trusted the field would then defer every reload for
 * the rest of the session. Asking whether the editor is on screen is
 * what keeps the deferral from becoming a wedge.
 */
import { beforeEach, describe, expect, it } from "vitest";

import type { Backlink, BlockNode } from "@outl/shared/api/types";

import { isEditing } from "./mode";
import { setAppState } from "./store";

function block(id: string): BlockNode {
  return {
    id,
    text: "cites [[today]]",
    todo: null,
    tokens: [],
    collapsed: false,
    properties: [],
    children: [],
  };
}

function row(id: string): Backlink {
  return {
    block_id: id,
    todo: null,
    source_page: {
      id: "pg-source",
      slug: "source",
      title: "Source",
      kind: "page",
    },
    source_block: block(id),
    source_block_path: [0],
    ancestors: [],
  };
}

describe("isEditing", () => {
  beforeEach(() => {
    setAppState({
      editingBlockId: null,
      editingBacklink: null,
      backlinks: [],
    });
  });

  it("is false with no editor anywhere", () => {
    expect(isEditing()).toBe(false);
  });

  it("is true while the outline's editor is mounted", () => {
    setAppState("editingBlockId", "blk-1");

    expect(isEditing()).toBe(true);
  });

  it("is true while a backlink row's editor is mounted", () => {
    setAppState({
      backlinks: [row("blk-a")],
      editingBacklink: { pageId: "pg-source", blockId: "blk-a" },
    });

    expect(isEditing()).toBe(true);
  });

  it("is true while a descendant row's editor is mounted, and only that copy", () => {
    const parent = row("blk-a");
    parent.source_block.children = [block("kid")];
    setAppState({
      backlinks: [parent],
      editingBacklink: { pageId: "pg-source", blockId: "kid", key: "blk-a>kid" },
    });
    expect(isEditing()).toBe(true);

    // A refetch that drops the child unmounts its editor, so the guard
    // lets go: reading `blockId` alone would hold it for the session.
    setAppState("backlinks", [row("blk-a")]);
    expect(isEditing()).toBe(false);
  });

  it("is false once the edited row leaves the list", () => {
    setAppState({
      backlinks: [row("blk-b")],
      editingBacklink: { pageId: "pg-source", blockId: "blk-a" },
    });

    // The row is gone, so its editor is gone with it. Reading the field
    // alone would hold every deferred reload open forever.
    expect(isEditing()).toBe(false);
  });
});
