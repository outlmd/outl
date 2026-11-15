import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  BOOT_MARKS,
  BOOT_STYLE_KEY,
  bootMark,
  cacheBootStyle,
  waitWorkspaceReady,
} from "./index";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

/**
 * `localStorage` is not always present (the warning this suite prints on
 * Node), so the cache helpers are exercised against a stub rather than
 * whatever the runner happens to provide.
 */
function stubStorage(impl: Partial<Storage>) {
  Object.defineProperty(globalThis, "localStorage", {
    value: impl,
    configurable: true,
    writable: true,
  });
}

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(undefined);
});

describe("boot marks", () => {
  it("names the four milestones in launch order", () => {
    expect([...BOOT_MARKS]).toEqual([
      "html_parsed",
      "theme_applied",
      "first_content",
      "interactive",
    ]);
  });

  it("reports a mark by name", () => {
    bootMark("first_content");
    expect(invoke).toHaveBeenCalledWith("boot_mark", { name: "first_content" });
  });

  /**
   * The point of the mark is that it cannot delay the frame it measures.
   * A client built against a backend without the command must not print
   * an unhandled rejection on every launch either.
   */
  it("swallows a rejection instead of surfacing it", async () => {
    invoke.mockRejectedValue(new Error("command not found"));
    expect(() => bootMark("html_parsed")).not.toThrow();
    await Promise.resolve();
  });
});

describe("waitWorkspaceReady", () => {
  it("passes the timeout through and resolves the backend's answer", async () => {
    invoke.mockResolvedValue(true);
    await expect(waitWorkspaceReady(2_000)).resolves.toBe(true);
    expect(invoke).toHaveBeenCalledWith("wait_workspace_ready", {
      timeoutMs: 2_000,
    });
  });

  /** "Not yet" is an answer, not a failure. */
  it("resolves false rather than rejecting when the workspace is still opening", async () => {
    invoke.mockResolvedValue(false);
    await expect(waitWorkspaceReady()).resolves.toBe(false);
  });

  it("lets a real transport failure reject", async () => {
    invoke.mockRejectedValue(new Error("pool refused"));
    await expect(waitWorkspaceReady()).rejects.toThrow("pool refused");
  });
});

describe("the boot style cache", () => {
  it("writes under the documented key", () => {
    const store = new Map<string, string>();
    stubStorage({
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => void store.set(k, v),
    } as Partial<Storage>);

    cacheBootStyle("--color-outl-bg:#fff;");
    // Asserted against the key the inline `<head>` script reads, which
    // is the only reader in production.
    expect(store.get(BOOT_STYLE_KEY)).toBe("--color-outl-bg:#fff;");
  });

  /**
   * A private window, blocked site data and a thumbnail capture all
   * throw here, and none of them is a reason to fail installing a
   * theme — the next frame paints the real palette either way.
   */
  it("survives a storage that throws", () => {
    stubStorage({
      setItem: () => {
        throw new Error("blocked");
      },
    } as Partial<Storage>);

    expect(() => cacheBootStyle("--color-outl-bg:#fff;")).not.toThrow();
  });
});
