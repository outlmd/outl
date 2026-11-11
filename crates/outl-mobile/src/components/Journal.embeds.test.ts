/**
 * The skip rule is the whole reason this module exists separately.
 *
 * `resolveEmbeds` is O(workspace) and `applyView` runs it on every
 * mutation, so a resolver with no skip puts a full index rebuild on the
 * commit path. A skip keyed on the wrong thing is worse than none: it
 * leaves a newly typed ref rendering as its raw handle until the user
 * navigates away and back, which reads exactly like
 * [#16](https://github.com/outlmd/outl/issues/16) not being fixed.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

import { createEmbedResolver } from "./Journal.embeds";
import type { BlockNode } from "@outl/shared/api/types";
import type { EmbedMap } from "@outl/shared/markdown";

/** A one-entry resolved map, the shape `resolveEmbeds` answers with. */
function resolved(handle: string, text: string): EmbedMap {
  return {
    [handle]: {
      handle,
      text,
      page_slug: "notes",
      status: null,
      children: [],
    },
  };
}

/** Let the microtask queue drain so a settled promise's `.then` runs. */
async function flush(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
}

const commands = vi.hoisted(() => ({ resolveEmbeds: vi.fn() }));
vi.mock("@outl/shared/api/commands", () => commands);

/**
 * `collectBlockRefHandles` walks `tokens`, not `text` — the backend has
 * already tokenized the block — so a fixture has to carry the token.
 */
function node(id: string, tokens: unknown[]): BlockNode {
  return {
    id,
    text: id,
    tokens,
    children: [],
    collapsed: false,
    todo: null,
    properties: [],
  } as unknown as BlockNode;
}

/** A block with no ref in it. */
function plain(id: string): BlockNode {
  return node(id, [{ kind: "text", value: "plain text" }]);
}

/** A block whose text carries a `((blk-…))` handle. */
function withRef(id: string, handle: string): BlockNode {
  return node(id, [
    { kind: "text", value: "see " },
    { kind: "blockref", value: handle },
  ]);
}

describe("createEmbedResolver", () => {
  beforeEach(() => {
    commands.resolveEmbeds.mockReset();
    commands.resolveEmbeds.mockResolvedValue({});
  });

  function resolver(slug = "notes") {
    let current = slug;
    const r = createEmbedResolver(() => current);
    return {
      r,
      /** What the resolver is currently publishing to `<BlockRow />`. */
      embeds: () => r.embeds(),
      navigate(next: string) {
        current = next;
      },
    };
  }

  it("resolves the handles the outline carries", () => {
    const { r } = resolver();
    r.resolve([withRef("b1", "blk-aaaaaa")]);
    expect(commands.resolveEmbeds).toHaveBeenCalledWith(["blk-aaaaaa"]);
  });

  it("skips a repeat resolve for the same page and the same handles", () => {
    const { r } = resolver();
    const outline = [withRef("b1", "blk-aaaaaa")];
    r.resolve(outline);
    r.resolve(outline);
    r.resolve(outline);
    expect(commands.resolveEmbeds).toHaveBeenCalledTimes(1);
  });

  it("resolves again when a ref is typed", () => {
    // The failure this pins: a skip keyed only on the slug would leave
    // the new ref printing its raw handle until the user navigated away.
    const { r } = resolver();
    r.resolve([withRef("b1", "blk-aaaaaa")]);
    r.resolve([withRef("b1", "blk-aaaaaa"), withRef("b2", "blk-bbbbbb")]);
    expect(commands.resolveEmbeds).toHaveBeenCalledTimes(2);
    expect(commands.resolveEmbeds).toHaveBeenLastCalledWith([
      "blk-aaaaaa",
      "blk-bbbbbb",
    ]);
  });

  it("resolves again on a navigation, even with identical handles", () => {
    // Same handle set, different page: the source block may have been
    // edited while the user was over there.
    const { r, navigate } = resolver();
    const outline = [withRef("b1", "blk-aaaaaa")];
    r.resolve(outline);
    navigate("other");
    r.resolve(outline);
    expect(commands.resolveEmbeds).toHaveBeenCalledTimes(2);
  });

  it("publishes an empty map for a page with no refs, and asks nothing", () => {
    const { r, embeds } = resolver();
    r.resolve([plain("b1")]);
    expect(commands.resolveEmbeds).not.toHaveBeenCalled();
    expect(embeds()).toEqual({});
  });

  it("does not re-ask forever after the last ref is deleted", () => {
    // The skip key is recorded *before* the empty-handle early return.
    // Asserting only that a ref-less page asks nothing cannot fail —
    // zero handles never call `resolveEmbeds` either way. What the
    // ordering actually decides is whether the key advances to "",
    // which is observable one step later: delete the last ref, then
    // type it back. Record after the early return and the key is still
    // the old handle set, so the retype is skipped and the ref renders
    // as a raw handle.
    const { r } = resolver();
    r.resolve([withRef("b1", "blk-aaaaaa")]);
    r.resolve([plain("b1")]);
    r.resolve([plain("b1")]);
    r.resolve([withRef("b1", "blk-aaaaaa")]);
    expect(commands.resolveEmbeds).toHaveBeenCalledTimes(2);
  });

  it("ignores a stale resolve that lands after a newer one", async () => {
    // `resolve_embeds` rebuilds the index on a blocking pool and takes
    // no workspace lock, so two in-flight calls can finish in either
    // order. Without a generation guard the older page's map wins, and
    // the skip key already names the newer page — so nothing ever
    // re-resolves and the new refs stay raw handles.
    const { r, embeds, navigate } = resolver();
    let landA: (m: EmbedMap) => void = () => {};
    let landB: (m: EmbedMap) => void = () => {};
    commands.resolveEmbeds
      .mockImplementationOnce(
        () =>
          new Promise<EmbedMap>((res) => {
            landA = res;
          }),
      )
      .mockImplementationOnce(
        () =>
          new Promise<EmbedMap>((res) => {
            landB = res;
          }),
      );

    r.resolve([withRef("b1", "blk-aaaaaa")]);
    navigate("other");
    r.resolve([withRef("b2", "blk-bbbbbb")]);

    landB(resolved("blk-bbbbbb", "the new page's source"));
    await flush();
    landA(resolved("blk-aaaaaa", "the old page's source"));
    await flush();

    expect(embeds()).toEqual(resolved("blk-bbbbbb", "the new page's source"));
  });

  it("swallows a failed resolve and publishes nothing", async () => {
    // The fallback is the raw handle, which is what the user saw before
    // this existed. A toast here would fire on every commit mid-sync.
    const { r, embeds } = resolver();
    commands.resolveEmbeds.mockRejectedValue(new Error("workspace busy"));
    r.resolve([withRef("b1", "blk-aaaaaa")]);
    await Promise.resolve();
    await Promise.resolve();
    expect(embeds()).toEqual({});
  });
});
