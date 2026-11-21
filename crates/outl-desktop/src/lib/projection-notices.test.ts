/**
 * Property: a projection refusal the backend emits reaches a surface
 * the user can see.
 *
 * `ref-projection-failed` had a payload type, an `onRefProjectionFailed`
 * subscriber and a `CHANGELOG` entry promising a toast — and no caller,
 * so `open_ref` writing a page the op log already held but the disk did
 * not was a failure nothing on screen reported. These tests drive the
 * installer against a fake event bus, so dropping either subscription
 * fails here rather than shipping.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

const bus = vi.hoisted(() => {
  const handlers = new Map<string, (e: { payload: unknown }) => void>();
  return {
    handlers,
    listen: vi.fn(async (name: string, handler: (e: { payload: unknown }) => void) => {
      handlers.set(name, handler);
      return () => handlers.delete(name);
    }),
    emit(name: string, payload: unknown) {
      const handler = handlers.get(name);
      if (!handler) throw new Error(`nothing listens to ${name}`);
      handler({ payload });
    },
  };
});
vi.mock("@tauri-apps/api/event", () => ({ listen: bus.listen }));

import { installProjectionNotices } from "./projection-notices";
import { appState, setAppState } from "./store";

const PAGE_ID = "01JQ0000000000000000000000";

beforeEach(() => {
  bus.handlers.clear();
  bus.listen.mockClear();
  setAppState({ lastError: null, mdAheadOfLog: undefined });
});

describe("ref-projection-failed", () => {
  it("puts the backend's sentence on the error toast", async () => {
    await installProjectionNotices();

    bus.emit("ref-projection-failed", {
      target: "[[weekly review]]",
      error: "permission denied writing pages/weekly-review.md",
    });

    expect(appState.lastError).toBe(
      "permission denied writing pages/weekly-review.md",
    );
  });

  it("never becomes the page's sticky banner", async () => {
    await installProjectionNotices();

    bus.emit("ref-projection-failed", { target: "[[a]]", error: "disk full" });

    // The op log holds the page and the write is retried, so the
    // transient failure must not claim the page stopped syncing.
    expect(appState.mdAheadOfLog).toBeUndefined();
  });

  it("stops reporting once the shell cleans up", async () => {
    const off = await installProjectionNotices();
    // Asserted before the cleanup too: `has(...) === false` is also what
    // a listener that was never installed looks like.
    expect(bus.handlers.has("ref-projection-failed")).toBe(true);

    off();

    expect(bus.handlers.has("ref-projection-failed")).toBe(false);
  });
});

describe("projection-write-failed", () => {
  it("becomes the sticky banner when it names the open page", async () => {
    setAppState("page", { id: PAGE_ID } as never);
    await installProjectionNotices();

    bus.emit("projection-write-failed", {
      page_id: PAGE_ID,
      md_ahead_of_log: { path: "pages/a.md", lines: 3, sample: "> kept" },
      error: "page markdown is ahead of the op log",
    });

    expect(appState.mdAheadOfLog?.lines).toBe(3);
    expect(appState.lastError).toBeNull();
  });

  it("falls back to the toast for a refusal on another page", async () => {
    setAppState("page", { id: PAGE_ID } as never);
    await installProjectionNotices();

    bus.emit("projection-write-failed", {
      page_id: "01JQ1111111111111111111111",
      md_ahead_of_log: { path: "pages/b.md", lines: 1, sample: "> kept" },
      error: "pages/b.md is ahead of the op log",
    });

    expect(appState.mdAheadOfLog).toBeUndefined();
    expect(appState.lastError).toBe("pages/b.md is ahead of the op log");
  });
});
