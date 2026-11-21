import { readFileSync } from "node:fs";
import { join } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot } from "solid-js";

import {
  listenForDeepLink,
  listenForFileDrop,
  listenForRefProjectionFailed,
  listenForWorkspaceReady,
  type JournalListenerDeps,
} from "./Journal.listeners";
import type { PageView } from "@outl/shared/api/types";

/**
 * Registered listeners, keyed by event name, **with their callbacks**.
 *
 * Keeping the callback is what makes the contract testable at all: the
 * first version of this mock dropped the second argument, so every test
 * exercised the handle's lifecycle and none of them ever ran a listener
 * body. The guard that skips a deep-link mid-edit, the `workspace-ready`
 * branch, and the on-page / off-page routing of a refusal could all be
 * deleted with the suite still green.
 */
const bus = vi.hoisted(() => ({
  handlers: new Map<string, (e: { payload: unknown }) => unknown>(),
  handles: [] as Array<{ disposed: boolean }>,
}));

/** Held shut so a test can unmount *while the dynamic import is still in
 *  flight* — the window the synchronous `onCleanup` exists to cover.
 *  Releasing twice is harmless: the promise is already settled. */
const gate = vi.hoisted(() => {
  let release!: () => void;
  const opened = new Promise<void>((r) => {
    release = r;
  });
  return { opened, release: () => release() };
});

vi.mock("@tauri-apps/api/event", async () => {
  await gate.opened;
  return {
    listen: vi.fn(async (event: string, cb: (e: { payload: unknown }) => unknown) => {
      bus.handlers.set(event, cb);
      const h = { disposed: false };
      bus.handles.push(h);
      return () => {
        h.disposed = true;
      };
    }),
  };
});

const fileDrop = vi.hoisted(() => ({ install: vi.fn() }));
vi.mock("@outl/shared/drag-drop", () => ({ installFileDrop: fileDrop.install }));

const commands = vi.hoisted(() => ({
  openTodayJournal: vi.fn(),
  openJournalFor: vi.fn(),
  openPageBySlug: vi.fn(),
}));
vi.mock("@outl/shared/api/commands", () => commands);

function view(id = "page-1", slug = "2026-09-18"): PageView {
  return {
    page: { id, slug, title: "", kind: "journal" },
    outline: [],
  } as unknown as PageView;
}

/** Deps backed by plain mutable state, plus spies on what the listeners
 *  are supposed to call. */
function harness(over: Partial<JournalListenerDeps> = {}) {
  const calls = {
    applyView: vi.fn(),
    // A deep link is a navigation and takes the zoom with it; every
    // other listener here is a refresh and must not.
    openView: vi.fn(),
    setError: vi.fn(),
    setAheadOfLog: vi.fn(),
    loadTodayWithRetry: vi.fn(async () => {}),
    pullAndReload: vi.fn(async () => {}),
    onFileDrop: vi.fn(),
    openCapture: vi.fn(),
  };
  const deps: JournalListenerDeps = {
    view: () => null,
    editingId: () => null,
    ...calls,
    ...over,
  };
  return { deps, calls };
}

/** Fire the listener registered for `event`, as the backend would. */
async function emit(event: string, payload: unknown) {
  const cb = bus.handlers.get(event);
  if (!cb) throw new Error(`no listener registered for ${event}`);
  await cb({ payload });
}

beforeEach(() => {
  bus.handlers.clear();
  bus.handles = [];
  fileDrop.install.mockReset();
  for (const fn of Object.values(commands)) fn.mockReset();
});

/**
 * The bug the module's shape exists to prevent, stated as a test rather
 * than as a comment: the component unmounts *before* the dynamic
 * `import()` resolves. Cleanup must already be armed at that point, and
 * the handle that arrives afterwards must be disposed on the spot rather
 * than pushed onto a list nobody will ever walk again.
 */
describe("a handle that arrives after unmount", () => {
  it("is disposed, not leaked — deep link", async () => {
    let dispose!: () => void;
    createRoot((d) => {
      dispose = d;
      listenForDeepLink(harness().deps);
    });

    dispose();
    gate.release();
    await vi.waitFor(() => expect(bus.handles).toHaveLength(1));

    expect(bus.handles[0].disposed).toBe(true);
  });

  it("is disposed, not leaked — workspace ready (two handles)", async () => {
    let dispose!: () => void;
    createRoot((d) => {
      dispose = d;
      listenForWorkspaceReady(harness().deps);
    });

    dispose();
    gate.release();
    await vi.waitFor(() => expect(bus.handles.length).toBeGreaterThanOrEqual(2));

    // Both listeners this function registers must be disposed, not just
    // the first — the two-handle path is where a single-handle variable
    // would silently drop one.
    expect(bus.handles.every((h) => h.disposed)).toBe(true);
  });

  it("is disposed, not leaked — file drop", async () => {
    const h = { disposed: false };
    let resolveInstall!: (un: () => void) => void;
    fileDrop.install.mockReturnValue(
      new Promise<() => void>((r) => {
        resolveInstall = r;
      }),
    );

    let dispose!: () => void;
    createRoot((d) => {
      dispose = d;
      listenForFileDrop(harness().deps);
    });

    dispose();
    resolveInstall(() => {
      h.disposed = true;
    });
    await vi.waitFor(() => expect(h.disposed).toBe(true));
  });
});

/** The ordinary path: still mounted when the handle lands, so it is kept
 *  and only disposed when the owner goes away. */
describe("a handle that arrives while still mounted", () => {
  it("is disposed at unmount, not before", async () => {
    let dispose!: () => void;
    createRoot((d) => {
      dispose = d;
      listenForDeepLink(harness().deps);
    });

    gate.release();
    await vi.waitFor(() => expect(bus.handles).toHaveLength(1));
    expect(bus.handles[0].disposed).toBe(false);

    dispose();
    expect(bus.handles[0].disposed).toBe(true);
  });
});

describe("deep link", () => {
  /** Navigating mid-edit would yank the textarea out from under the
   *  user's keystroke. The guard is one line and nothing else enforces it. */
  it("does not navigate while a block is being edited", async () => {
    const { deps } = harness({ editingId: () => "blk-1" });
    createRoot(() => listenForDeepLink(deps));
    gate.release();
    await vi.waitFor(() => expect(bus.handlers.has("deep-link://navigate")).toBe(true));

    await emit("deep-link://navigate", { kind: "today" });

    expect(commands.openTodayJournal).not.toHaveBeenCalled();
  });

  it.each([
    ["today", { kind: "today" }, "openTodayJournal"],
    ["daily", { kind: "daily", date: "2026-09-18" }, "openJournalFor"],
    ["page", { kind: "page", slug: "notes" }, "openPageBySlug"],
  ] as const)("routes a %s link to %s", async (_label, payload, cmd) => {
    const { deps, calls } = harness();
    commands[cmd].mockResolvedValue(view());
    createRoot(() => listenForDeepLink(deps));
    gate.release();
    await vi.waitFor(() => expect(bus.handlers.has("deep-link://navigate")).toBe(true));

    await emit("deep-link://navigate", payload);

    expect(commands[cmd]).toHaveBeenCalled();
    // `openView`, not `applyView`: a deep link into the page already on
    // screen still has to leave the zoom the user was in.
    expect(calls.openView).toHaveBeenCalled();
    expect(calls.applyView).not.toHaveBeenCalled();
  });

  it("opens the capture sheet for outl://capture without navigating", async () => {
    const { deps, calls } = harness();
    createRoot(() => listenForDeepLink(deps));
    gate.release();
    await vi.waitFor(() => expect(bus.handlers.has("deep-link://navigate")).toBe(true));

    await emit("deep-link://navigate", { kind: "capture" });

    expect(calls.openCapture).toHaveBeenCalledOnce();
    expect(commands.openTodayJournal).not.toHaveBeenCalled();
    expect(calls.openView).not.toHaveBeenCalled();
  });

  it("surfaces a failed navigation instead of swallowing it", async () => {
    const { deps, calls } = harness();
    commands.openTodayJournal.mockRejectedValue(new Error("boom"));
    createRoot(() => listenForDeepLink(deps));
    gate.release();
    await vi.waitFor(() => expect(bus.handlers.has("deep-link://navigate")).toBe(true));

    await emit("deep-link://navigate", { kind: "today" });

    expect(calls.setError).toHaveBeenCalledWith("Error: boom");
    expect(calls.openView).not.toHaveBeenCalled();
  });
});

describe("workspace-ready", () => {
  /**
   * The event fires when peer ops **land**, so there may not be another
   * one. An early return here threw the signal away and left the view
   * waiting for the 5s poll — the branch this asserts is the fix.
   */
  it("boots the journal when no view has landed yet", async () => {
    const { deps, calls } = harness({ view: () => null });
    createRoot(() => listenForWorkspaceReady(deps));
    gate.release();
    await vi.waitFor(() => expect(bus.handlers.has("workspace-ready")).toBe(true));

    await emit("workspace-ready", null);

    expect(calls.loadTodayWithRetry).toHaveBeenCalled();
    expect(calls.pullAndReload).not.toHaveBeenCalled();
  });

  /**
   * Peer-op writes emit this event repeatedly, so a slow boot sees several
   * of them while the first load is still awaiting. Without a guard each
   * starts its own 50-attempt retry loop, and on iOS they all queue on the
   * one thread that serves Tauri IPC and paints the UI — the stack the
   * scene-update watchdog kills the app for.
   */
  it("never stacks a second boot load on a slow one", async () => {
    let settle!: () => void;
    const first = new Promise<void>((r) => {
      settle = r;
    });
    const load = vi.fn(() => first);
    const { deps } = harness({ view: () => null, loadTodayWithRetry: load });
    createRoot(() => listenForWorkspaceReady(deps));
    gate.release();
    await vi.waitFor(() => expect(bus.handlers.has("workspace-ready")).toBe(true));

    const inFlight = emit("workspace-ready", null);
    await emit("workspace-ready", null);
    await emit("workspace-ready", null);
    expect(load).toHaveBeenCalledTimes(1);

    settle();
    await inFlight;
    // The guard releases with the load, so a later event still boots.
    await emit("workspace-ready", null);
    expect(load).toHaveBeenCalledTimes(2);
  });

  it("goes through the guarded reload once a view exists", async () => {
    const { deps, calls } = harness({ view: () => view() });
    createRoot(() => listenForWorkspaceReady(deps));
    gate.release();
    await vi.waitFor(() => expect(bus.handlers.has("workspace-ready")).toBe(true));

    await emit("workspace-ready", null);

    expect(calls.pullAndReload).toHaveBeenCalledWith({ background: true });
    expect(calls.loadTodayWithRetry).not.toHaveBeenCalled();
  });
});

describe("projection-write-failed", () => {
  const notice = { lines: ["a line the log never saw"] };

  it("becomes the open page's sticky banner", async () => {
    const { deps, calls } = harness({ view: () => view("page-1", "today") });
    createRoot(() => listenForWorkspaceReady(deps));
    gate.release();
    await vi.waitFor(() => expect(bus.handlers.has("projection-write-failed")).toBe(true));

    await emit("projection-write-failed", {
      page_id: "page-1",
      error: "refused",
      md_ahead_of_log: notice,
    });

    expect(calls.setAheadOfLog).toHaveBeenCalledWith({ slug: "today", info: notice });
    expect(calls.setError).not.toHaveBeenCalled();
  });

  /**
   * An off-screen refusal froze a page just the same. Root `CLAUDE.md`
   * invariant 8: a refusal swallowed into a log line ships a page that
   * silently stopped syncing.
   */
  it("still reaches the user when it names another page", async () => {
    const { deps, calls } = harness({ view: () => view("page-1", "today") });
    createRoot(() => listenForWorkspaceReady(deps));
    gate.release();
    await vi.waitFor(() => expect(bus.handlers.has("projection-write-failed")).toBe(true));

    await emit("projection-write-failed", {
      page_id: "page-999",
      error: "refused elsewhere",
      md_ahead_of_log: notice,
    });

    expect(calls.setError).toHaveBeenCalledWith("refused elsewhere");
    expect(calls.setAheadOfLog).not.toHaveBeenCalled();
  });

  it("reports a plain projection failure as an error", async () => {
    const { deps, calls } = harness({ view: () => view() });
    createRoot(() => listenForWorkspaceReady(deps));
    gate.release();
    await vi.waitFor(() => expect(bus.handlers.has("projection-write-failed")).toBe(true));

    await emit("projection-write-failed", {
      page_id: "page-1",
      error: "disk full",
      md_ahead_of_log: null,
    });

    expect(calls.setError).toHaveBeenCalledWith("disk full");
    expect(calls.setAheadOfLog).not.toHaveBeenCalled();
  });
});

/**
 * `open_ref` writes the page into the op log and then projects
 * `pages/<slug>.md`; when that write fails the backend emits this event.
 * Mobile registered no listener for it at all, so the ref the user had
 * just inserted pointed at a page no peer could see and nothing on
 * screen said so — the shape root `CLAUDE.md` invariant 8 is about.
 */
describe("ref-projection-failed", () => {
  const refusal = {
    target: "[[weekly review]]",
    error: "permission denied writing pages/weekly-review.md",
  };

  it("puts the backend's sentence on the error surface", async () => {
    const { deps, calls } = harness({ view: () => view() });
    // Asserted before the install too: `has(...) === false` is also what
    // a listener nobody registered looks like, which is the bug.
    expect(bus.handlers.has("ref-projection-failed")).toBe(false);

    createRoot(() => listenForRefProjectionFailed(deps));
    gate.release();
    await vi.waitFor(() =>
      expect(bus.handlers.has("ref-projection-failed")).toBe(true),
    );

    await emit("ref-projection-failed", refusal);

    expect(calls.setError).toHaveBeenCalledWith(refusal.error);
    // The op log already holds the page and the next save retries the
    // write, so a transient refusal must not claim the page stopped
    // syncing — the backend keeps `ahead_of_log` off this event for the
    // same reason.
    expect(calls.setAheadOfLog).not.toHaveBeenCalled();
  });

  it("is disposed, not leaked", async () => {
    let dispose!: () => void;
    createRoot((d) => {
      dispose = d;
      listenForRefProjectionFailed(harness().deps);
    });

    dispose();
    gate.release();
    await vi.waitFor(() => expect(bus.handles).toHaveLength(1));

    expect(bus.handles[0].disposed).toBe(true);
  });
});

/**
 * A listener function nothing calls is exactly the defect above, and the
 * whole reason there is one entry point rather than four call sites in
 * the component.
 *
 * So this covers both halves: `installJournalListeners` is driven for
 * real (every event it is supposed to subscribe), and the component's
 * single call to it is read as source — mounting `Journal` needs
 * haptics, the Tauri bridge and a WKWebView-shaped textarea, so a test
 * that mocked all three would pin the mocks, the reasoning
 * `Journal.load-gate.test.ts` gives.
 */
describe("installJournalListeners", () => {
  const source = (file: string) =>
    readFileSync(join(import.meta.dirname, file), "utf8");

  /**
   * Read as source, and **not** by choice: driving the installer would
   * fire four dynamic `import("@tauri-apps/api/event")`s in one tick,
   * and this `vi.mock` serves the mocked module to the first of a
   * concurrent set only — the other three get the real module and throw
   * on `window.__TAURI_INTERNALS__`. Serialised imports are all mocked,
   * which is why every test above (one listener each, and
   * `listenForWorkspaceReady`'s two events off one import) works.
   * So each listener's *behaviour* is driven individually above, and
   * what is pinned here is the composition.
   */
  it("installs every listener in this module", () => {
    const listeners = source("Journal.listeners.ts");
    const installer = listeners.slice(
      listeners.indexOf("export function installJournalListeners"),
    );
    const body = installer.slice(0, installer.indexOf("\n}"));

    const exported = [
      ...listeners.matchAll(/export function (listenFor\w+)\(/g),
    ].map((m) => m[1]!);
    expect(exported.length, "every listener this module exports").toBe(4);
    for (const fn of exported) {
      expect(
        body,
        `${fn} is exported and nothing installs it — a listener nobody ` +
          `calls is the defect this entry point exists to prevent`,
      ).toContain(`${fn}(deps)`);
    }
  });

  it("is what the journal's mount calls", () => {
    const journal = source("Journal.tsx");

    expect(
      [...journal.matchAll(/\binstallJournalListeners\(listenerDeps\)/g)],
      "the component installs the listeners exactly once",
    ).toHaveLength(1);
    // Going back to per-listener calls puts the ordering rule, and the
    // "did anyone call this one" question, back in the component.
    expect(journal).not.toMatch(/\blistenFor\w+\(/);
  });
});

describe("file drop", () => {
  it("routes dropped paths with the block under the pointer", async () => {
    const { deps, calls } = harness();
    fileDrop.install.mockResolvedValue(() => {});
    createRoot(() => listenForFileDrop(deps));
    await vi.waitFor(() => expect(fileDrop.install).toHaveBeenCalled());

    const { onDrop } = fileDrop.install.mock.calls[0][0];
    onDrop(["/tmp/a.png"], "blk-7");

    expect(calls.onFileDrop).toHaveBeenCalledWith(["/tmp/a.png"], "blk-7");
  });
});
