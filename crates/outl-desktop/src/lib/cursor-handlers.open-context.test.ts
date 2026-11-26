/**
 * `g o` (`Action::OpenBacklinkContext`) acts on the backlink row under
 * the cursor, and says what it needs when the cursor is elsewhere.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

import type { Backlink, BlockNode } from "@outl/shared/api/types";

const jump = vi.hoisted(() => ({ openBacklinkContext: vi.fn() }));
vi.mock("./backlink-jump", () => jump);

import { buildHandlers } from "./action-handlers";
import { setAppState } from "./store";

const node = (id: string): BlockNode => ({
  id,
  text: id,
  todo: null,
  tokens: [],
  collapsed: false,
  properties: [],
  children: [],
});

const link: Backlink = {
  block_id: "cite",
  todo: null,
  ancestors: [],
  source_page: { id: "src", slug: "src", title: "src", kind: "page" },
  source_block: node("cite"),
  source_block_path: [],
};

afterEach(() => {
  vi.clearAllMocks();
  setAppState({
    page: null,
    backlinks: [],
    selectedBacklinkBlockId: null,
    selectedBlockId: null,
  } as never);
});

describe("OpenBacklinkContext", () => {
  it("opens the context of the row under the cursor", async () => {
    setAppState({
      page: { id: "open", slug: "open", title: "open", kind: "page" },
      backlinksOpen: true,
      backlinks: [link],
      selectedBlockId: null,
      selectedBacklinkBlockId: "cite",
    } as never);
    const setError = vi.fn();
    const h = buildHandlers({ applyView: () => {}, setError });
    await h.OpenBacklinkContext?.();
    expect(jump.openBacklinkContext).toHaveBeenCalledWith(link);
    expect(setError).not.toHaveBeenCalled();
  });

  it("with the cursor in the outline, explains instead of jumping", async () => {
    setAppState({ selectedBlockId: "a", selectedBacklinkBlockId: null } as never);
    const setError = vi.fn();
    const h = buildHandlers({ applyView: () => {}, setError });
    await h.OpenBacklinkContext?.();
    expect(jump.openBacklinkContext).not.toHaveBeenCalled();
    expect(setError).toHaveBeenCalledTimes(1);
  });
});
