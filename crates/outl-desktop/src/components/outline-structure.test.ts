import { describe, expect, it, vi } from "vitest";

import type { PageView } from "@outl/shared/api/types";

/**
 * Regression for #477: Tab / Shift-Tab inside a block's textarea
 * indented / outdented it but left edit mode, and the selection stayed
 * on whatever block was selected before — so the very next keystroke
 * ran as a Normal-mode command against the wrong block.
 *
 * `<BlockEditor />`'s Tab handler commits the draft (which flips
 * `editingBlockId` to `null`) before calling `onIndent` / `onOutdent`.
 * The two callbacks must put the moved block straight back into edit
 * mode — on itself, not on whatever was selected beforehand — whether
 * the move landed, was a no-op (first block can't indent further), or
 * the command outright failed.
 */

vi.mock("@outl/shared/api/commands", () => ({
  indentBlock: vi.fn(),
  outdentBlock: vi.fn(),
}));

import { indentBlock, outdentBlock } from "@outl/shared/api/commands";
import { createStructureOps, type StructureOpsDeps } from "./outline-structure";

function makeView(): PageView {
  return {
    page: { id: "page-1", slug: "today", kind: "journal", title: "", icon: null },
    outline: [],
    warnings: [],
    page_properties: [],
    backlinks: [],
  } as unknown as PageView;
}

async function handleError<T>(p: Promise<T>): Promise<T | undefined> {
  try {
    return await p;
  } catch {
    return undefined;
  }
}

function makeDeps(): StructureOpsDeps {
  return {
    pageId: vi.fn(() => "page-1"),
    applyView: vi.fn(),
    handleError,
    setSelectedBlockId: vi.fn(),
    setEditingId: vi.fn(),
  };
}

describe("createStructureOps — #477", () => {
  it("onIndent re-selects and re-edits the moved block after a successful move", async () => {
    const view = makeView();
    vi.mocked(indentBlock).mockResolvedValue(view);
    const deps = makeDeps();
    const ops = createStructureOps(deps);

    await ops.onIndent("blk-1");

    expect(indentBlock).toHaveBeenCalledWith("page-1", "blk-1");
    expect(deps.applyView).toHaveBeenCalledWith(view);
    expect(deps.setSelectedBlockId).toHaveBeenCalledWith("blk-1");
    expect(deps.setEditingId).toHaveBeenCalledWith("blk-1");
  });

  it("onOutdent re-selects and re-edits the moved block after a successful move", async () => {
    const view = makeView();
    vi.mocked(outdentBlock).mockResolvedValue(view);
    const deps = makeDeps();
    const ops = createStructureOps(deps);

    await ops.onOutdent("blk-2");

    expect(outdentBlock).toHaveBeenCalledWith("page-1", "blk-2");
    expect(deps.applyView).toHaveBeenCalledWith(view);
    expect(deps.setSelectedBlockId).toHaveBeenCalledWith("blk-2");
    expect(deps.setEditingId).toHaveBeenCalledWith("blk-2");
  });

  it("stays in edit mode on the block even when the command rejects", async () => {
    // A genuine backend failure (bad page id, invalid node). The block
    // must not be left stranded outside edit mode with a stale
    // selection — that is the exact bug #477 reports.
    vi.mocked(indentBlock).mockRejectedValue(new Error("boom"));
    const deps = makeDeps();
    const ops = createStructureOps(deps);

    await ops.onIndent("blk-3");

    expect(deps.applyView).not.toHaveBeenCalled();
    expect(deps.setSelectedBlockId).toHaveBeenCalledWith("blk-3");
    expect(deps.setEditingId).toHaveBeenCalledWith("blk-3");
  });

  it("stays in edit mode on the block when the move is a backend no-op (first block, Tab)", async () => {
    // `indent_block` swallows `NoPreviousSibling` and returns the
    // unchanged view rather than rejecting — this pins that the
    // no-op path re-enters edit mode exactly like the success path.
    const view = makeView();
    vi.mocked(indentBlock).mockResolvedValue(view);
    const deps = makeDeps();
    const ops = createStructureOps(deps);

    await ops.onIndent("blk-first");

    expect(deps.setSelectedBlockId).toHaveBeenCalledWith("blk-first");
    expect(deps.setEditingId).toHaveBeenCalledWith("blk-first");
  });
});
