import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

import { mergePages, renamePage } from "./page-merge";

beforeEach(() => {
  invoke.mockReset();
});

describe("page-merge wrappers", () => {
  it("renamePage invokes rename_page with camelCase args", async () => {
    invoke.mockResolvedValue({ status: "exists", existing: { id: "i", slug: "s", title: "S", kind: "page" } });
    const reply = await renamePage("old", "New name");
    expect(invoke).toHaveBeenCalledWith("rename_page", { page: "old", newName: "New name" });
    expect(reply.status).toBe("exists");
  });

  it("mergePages invokes merge_pages", async () => {
    invoke.mockResolvedValue({ view: {}, report: {} });
    await mergePages("a", "b");
    expect(invoke).toHaveBeenCalledWith("merge_pages", { source: "a", target: "b" });
  });

  it("lets a backend error reject", async () => {
    invoke.mockRejectedValue(new Error("cannot merge a page into itself"));
    await expect(mergePages("a", "a")).rejects.toThrow("cannot merge a page into itself");
  });
});
