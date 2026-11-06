import { beforeEach, describe, expect, it, vi } from "vitest";

import type { PageView, PasteReply } from "@outl/shared/api/types";
import { createPasteOps, type PasteOpsDeps } from "./outline-paste";

/**
 * Regression for #488, at the layer the component test cannot reach.
 *
 * `<BlockRow />`'s paste test mocks the callback with an already-folded
 * `{ text, caret }`, so it pins the editor half and never runs this
 * module: `run`'s whole sequence — flush the draft, run the command,
 * fold the reply — was deletable with a green suite. The three steps
 * are the fix; the last one is the one that was missing when a pasted
 * link disappeared from the block and from the `.md`.
 */

const commands = vi.hoisted(() => ({ editBlock: vi.fn() }));
vi.mock("@outl/shared/api/commands", () => commands);

function pageView(): PageView {
  return {
    page: { id: "page-1", slug: "2026-11-06", title: "", kind: "journal" },
    outline: [],
  } as unknown as PageView;
}

/** Deps backed by plain mutable state, so the ops can be driven without
 *  a Solid runtime. `calls` records the order the two round-trips ran
 *  in, which is the thing the flush exists for. */
function harness() {
  const state = {
    pageId: "page-1" as string | undefined,
    editingId: "blk-1" as string | null,
    applied: [] as PageView[],
    calls: [] as string[],
  };
  const deps: PasteOpsDeps = {
    pageId: () => state.pageId,
    editingId: () => state.editingId,
    applyView: (v) => state.applied.push(v),
    handleError: async (promise) => {
      try {
        return await promise;
      } catch {
        return undefined;
      }
    },
  };
  return { state, ops: createPasteOps(deps) };
}

/** A paste that merged into the host block. */
function reply(host_text: string, host_caret: number): PasteReply {
  return { view: pageView(), host_text, host_caret } as PasteReply;
}

describe("createPasteOps.run — #488", () => {
  const HOST_AFTER = "Some text[See the invoice](https://example.test/i)";

  beforeEach(() => {
    commands.editBlock.mockReset();
    commands.editBlock.mockResolvedValue(pageView());
  });

  it("hands the editor the host block's new text and caret", async () => {
    // Neuter the fold (`return null` instead of `pasteEditorUpdate`)
    // and only this assertion fails — the whole point of the module.
    const { state, ops } = harness();

    const update = await ops.run("blk-1", "Some text", async () =>
      reply(HOST_AFTER, HOST_AFTER.length),
    );

    expect(update).toEqual({ text: HOST_AFTER, caret: HOST_AFTER.length });
    expect(state.applied).toHaveLength(2); // the flush's view, then the paste's
  });

  it("keeps the caret where the backend put it, not at the end", async () => {
    // A paste without formatting leaves the user's tail to the right of
    // the caret. "end of text" would drop them past it.
    const { ops } = harness();

    const update = await ops.run("blk-1", "hello world", async () =>
      reply("hello BRAVE world", 12),
    );

    expect(update).toEqual({ text: "hello BRAVE world", caret: 12 });
  });

  it("converts the caret from codepoints to UTF-16 units", async () => {
    // `host_caret` counts codepoints (Rust). A textarea counts UTF-16
    // code units, so the emoji's surrogate pair has to be paid for or
    // the caret lands inside it.
    const { ops } = harness();

    const update = await ops.run("blk-1", "🎉", async () => reply("🎉ok", 3));

    expect(update).toEqual({ text: "🎉ok", caret: 4 });
  });

  it("flushes the in-flight draft before the splice runs", async () => {
    // The caret was measured against the textarea, so the block has to
    // be holding that text when the backend splices into it.
    const { state, ops } = harness();
    commands.editBlock.mockImplementation(async () => {
      state.calls.push("editBlock");
      return pageView();
    });

    await ops.run("blk-1", "Some text", async () => {
      state.calls.push("paste");
      return reply(HOST_AFTER, HOST_AFTER.length);
    });

    expect(commands.editBlock).toHaveBeenCalledWith(
      "page-1",
      "blk-1",
      "Some text",
    );
    expect(state.calls).toEqual(["editBlock", "paste"]);
  });

  it("skips the flush when the block is not the one being edited", async () => {
    // A paste from a click without an open editor: there is no draft to
    // lose, and committing would be a write the user did not make.
    const { state, ops } = harness();
    state.editingId = null;

    await ops.run("blk-1", "Some text", async () =>
      reply(HOST_AFTER, HOST_AFTER.length),
    );

    expect(commands.editBlock).not.toHaveBeenCalled();
  });

  it("returns null and runs nothing when no page is open", async () => {
    const { state, ops } = harness();
    state.pageId = undefined;
    const command = vi.fn();

    expect(await ops.run("blk-1", "Some text", command)).toBeNull();
    expect(command).not.toHaveBeenCalled();
    expect(commands.editBlock).not.toHaveBeenCalled();
  });

  it("returns null without touching the editor when the paste failed", async () => {
    const { state, ops } = harness();

    const update = await ops.run("blk-1", "Some text", async () => {
      throw new Error("backend said no");
    });

    expect(update).toBeNull();
    // Only the flush's view landed; nothing claims the paste happened.
    expect(state.applied).toHaveLength(1);
  });

  it("refreshes the page but leaves the editor alone when the host block was untouched", async () => {
    // A paste that only created siblings reports no `host_text`.
    const { state, ops } = harness();

    const update = await ops.run(
      "blk-1",
      "Some text",
      async () => ({ view: pageView() }) as PasteReply,
    );

    expect(update).toBeNull();
    expect(state.applied).toHaveLength(2);
  });
});
