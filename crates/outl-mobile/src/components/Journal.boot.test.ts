import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { PageView } from "@outl/shared/api/types";

const commands = vi.hoisted(() => ({
  openTodayJournal: vi.fn(),
  openJournalFor: vi.fn(),
  openPageBySlug: vi.fn(),
  reloadWorkspace: vi.fn(async () => {}),
  syncNow: vi.fn(async () => {}),
}));
vi.mock("@outl/shared/api/commands", () => commands);

const bootApi = vi.hoisted(() => ({
  bootMark: vi.fn(),
  waitWorkspaceReady: vi.fn(async () => true),
  // The cache helpers are not used here, but the module is mocked
  // wholesale so a real `invoke` never reaches a non-existent backend.
  BOOT_MARKS: ["html_parsed", "theme_applied", "first_content", "interactive"],
  BOOT_STYLE_KEY: "outl.boot.style",
  cacheBootStyle: vi.fn(),
}));
vi.mock("@outl/shared/boot", () => bootApi);

const splash = vi.hoisted(() => ({ dismissBootSplash: vi.fn() }));
vi.mock("../lib/splash", () => splash);

import {
  OPEN_TIMEOUT_MS,
  SYNC_TIMEOUT_MS,
  createJournalBoot,
  nextPaint,
  runBootSequence,
} from "./Journal.boot";
import type { BootSequence, JournalBootDeps } from "./Journal.boot";

function view(slug = "2026-10-14", outline: unknown[] = [{ id: "b1" }]): PageView {
  return {
    page: { id: "page-1", slug, title: "", kind: "journal" },
    outline,
  } as unknown as PageView;
}

/** The gates the two loads hold open, as plain observable state. */
function harness(over: Partial<JournalBootDeps> = {}) {
  const state = {
    loaded: false,
    loadFailed: false,
    syncing: false,
    error: null as string | null,
    applied: [] as PageView[],
  };
  const deps: JournalBootDeps = {
    view: () => null,
    editingId: () => null,
    applyView: (v) => state.applied.push(v),
    setError: (m) => {
      state.error = m;
    },
    setLoaded: (v) => {
      state.loaded = v;
    },
    setLoadFailed: (v) => {
      state.loadFailed = v;
    },
    setSyncing: (v) => {
      state.syncing = v;
    },
    withError: async (fn) => {
      try {
        state.error = null;
        return await fn();
      } catch (e) {
        state.error = String(e);
        return undefined;
      }
    },
    refreshPeerStatus: async () => {},
    ...over,
  };
  return { boot: createJournalBoot(deps), state };
}

/** A promise that never settles — the shape of a Tauri command whose
 *  thread is pinned, which is the failure every cap here exists for. */
const forever = () => new Promise<never>(() => {});

beforeEach(() => {
  for (const fn of Object.values(commands)) fn.mockReset();
  commands.reloadWorkspace.mockResolvedValue(undefined);
  commands.syncNow.mockResolvedValue(undefined);
  bootApi.bootMark.mockReset();
  bootApi.waitWorkspaceReady.mockReset();
  bootApi.waitWorkspaceReady.mockResolvedValue(true);
  splash.dismissBootSplash.mockReset();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("boot load", () => {
  it("opens today and releases the skeleton", async () => {
    const v = view();
    commands.openTodayJournal.mockResolvedValue(v);
    const { boot, state } = harness();

    await boot.loadTodayWithRetry();

    expect(state.applied).toEqual([v]);
    expect(state.loaded).toBe(true);
    expect(state.loadFailed).toBe(false);
    expect(state.error).toBeNull();
  });

  /**
   * The one that matters: the open is the ONLY awaited command on the
   * first-paint path, and `loaded` is the sole gate on the skeleton. An
   * uncapped open that never settles therefore left the user looking at a
   * skeleton forever, with no retry and nothing said — the infinite-boot
   * half of the iOS watchdog kill.
   */
  it("a never-settling open ends in the failure state, not a permanent skeleton", async () => {
    vi.useFakeTimers();
    commands.openTodayJournal.mockImplementation(forever);
    const { boot, state } = harness();

    const load = boot.loadTodayWithRetry();
    await vi.advanceTimersByTimeAsync(OPEN_TIMEOUT_MS + 1);
    await load;

    expect(state.loaded).toBe(true); // skeleton gone
    expect(state.loadFailed).toBe(true); // Retry button armed
    expect(state.error).toContain("Opening the workspace");
    expect(state.applied).toEqual([]);
    // One attempt, then done: a stall is not a "still booting" retry.
    expect(commands.openTodayJournal).toHaveBeenCalledTimes(1);
  });

  it("keeps retrying while the workspace is still opening", async () => {
    vi.useFakeTimers();
    commands.openTodayJournal
      .mockRejectedValueOnce("workspace_loading")
      .mockRejectedValueOnce("workspace_loading")
      .mockResolvedValueOnce(view());
    const { boot, state } = harness();

    const load = boot.loadTodayWithRetry();
    await vi.advanceTimersByTimeAsync(1000);
    await load;

    expect(commands.openTodayJournal).toHaveBeenCalledTimes(3);
    expect(state.loadFailed).toBe(false);
    expect(state.loaded).toBe(true);
  });

  /**
   * The point of replacing the backoff: the load asks the backend to
   * *wait*, so it resolves when the workspace is published instead of on
   * the next tick of an interval it owns.
   */
  it("waits for the workspace before asking for the journal", async () => {
    commands.openTodayJournal.mockResolvedValue(view());
    const { boot } = harness();

    await boot.loadTodayWithRetry();

    expect(bootApi.waitWorkspaceReady).toHaveBeenCalled();
    expect(commands.openTodayJournal).toHaveBeenCalledTimes(1);
  });

  /**
   * A backend built before `wait_workspace_ready` existed must still
   * boot: the open itself answers `workspace_loading` while the opener
   * runs, which is what the loop handles.
   */
  it("still loads when the backend has no readiness command", async () => {
    bootApi.waitWorkspaceReady.mockRejectedValueOnce(
      new Error("command not found"),
    );
    commands.openTodayJournal.mockResolvedValue(view());
    const { boot, state } = harness();

    await boot.loadTodayWithRetry();

    expect(state.loaded).toBe(true);
    expect(state.loadFailed).toBe(false);
  });

  it("reports first_content once real content is applied", async () => {
    commands.openTodayJournal.mockResolvedValue(view());
    const { boot } = harness();

    await boot.loadTodayWithRetry();

    expect(bootApi.bootMark).toHaveBeenCalledWith("first_content");
  });

  /**
   * The failure state carries a Retry button, so leaving the splash up
   * over it is a dead end — the one case a timer-dismissed splash got
   * right by accident and an app-dismissed one has to do on purpose.
   */
  it("dismisses the splash on failure too", async () => {
    commands.openTodayJournal.mockRejectedValue("boom");
    const { boot, state } = harness();

    await boot.loadTodayWithRetry();

    expect(state.loadFailed).toBe(true);
    expect(splash.dismissBootSplash).toHaveBeenCalled();
  });

  it("gives up with a message when the workspace never finishes opening", async () => {
    vi.useFakeTimers();
    commands.openTodayJournal.mockRejectedValue("workspace_loading");
    const { boot, state } = harness();

    const load = boot.loadTodayWithRetry();
    await vi.advanceTimersByTimeAsync(60_000);
    await load;

    expect(state.error).toBe("Workspace took too long to open.");
    expect(state.loadFailed).toBe(true);
    expect(state.loaded).toBe(true);
  });
});

describe("pull and reload", () => {
  it("reopens the page and applies what landed", async () => {
    const next = view("2026-10-14", [{ id: "b1" }, { id: "b2" }]);
    commands.openJournalFor.mockResolvedValue(next);
    const { boot, state } = harness({ view: () => view() });

    await boot.pullAndReload();

    expect(state.applied).toEqual([next]);
    expect(state.syncing).toBe(false);
  });

  /**
   * `syncing` is what `handleRefresh` and `PullToRefresh` both refuse to
   * start a second pass under, so a pass that leaves it set takes the
   * user's only recovery gesture with it — permanently, since nothing
   * else clears the flag. It has to come back down on every exit.
   */
  it("releases the sync spinner when the pass throws", async () => {
    commands.openJournalFor.mockResolvedValue(view("2026-10-14", [{ id: "b2" }]));
    const { boot, state } = harness({
      view: () => view(),
      // Not wrapped by `withError`: this is the caller's own code, which
      // is exactly the gap a trailing `setSyncing(false)` could not cover.
      applyView: () => {
        throw new Error("render blew up");
      },
    });

    await expect(boot.pullAndReload()).rejects.toThrow("render blew up");

    expect(state.syncing).toBe(false);
  });

  it("releases the sync spinner when a command never settles", async () => {
    vi.useFakeTimers();
    commands.syncNow.mockImplementation(forever);
    commands.reloadWorkspace.mockImplementation(forever);
    commands.openJournalFor.mockImplementation(forever);
    const { boot, state } = harness({ view: () => view() });

    const pass = boot.pullAndReload();
    expect(state.syncing).toBe(true);
    await vi.advanceTimersByTimeAsync(SYNC_TIMEOUT_MS + OPEN_TIMEOUT_MS * 2 + 3);
    await pass;

    expect(state.syncing).toBe(false);
    expect(state.applied).toEqual([]);
  });

  it("a background pass never touches the spinner", async () => {
    commands.openJournalFor.mockResolvedValue(view());
    const { boot, state } = harness({ view: () => view() });
    let sawSpinner = false;
    const pass = boot.pullAndReload({ background: true });
    sawSpinner = state.syncing;
    await pass;

    expect(sawSpinner).toBe(false);
    expect(state.syncing).toBe(false);
  });

  it("defers the re-render while a block is being edited, and drains once", async () => {
    const { boot, state } = harness({ view: () => view(), editingId: () => "b1" });

    expect(boot.takePendingReload()).toBe(false);
    await boot.pullAndReload();

    expect(commands.syncNow).toHaveBeenCalled(); // ops still pulled to disk
    expect(commands.openJournalFor).not.toHaveBeenCalled(); // but not re-rendered
    expect(state.applied).toEqual([]);
    expect(boot.takePendingReload()).toBe(true);
    expect(boot.takePendingReload()).toBe(false); // clears on read
  });

  it("drops a reply that comes back empty over real content", async () => {
    commands.openJournalFor.mockResolvedValue(view("2026-10-14", []));
    const { boot, state } = harness({ view: () => view() });

    await boot.pullAndReload();

    expect(state.applied).toEqual([]);
  });
});

/**
 * The order a cold start runs in.
 *
 * The complaint these pin is "opening the app is slow to load today's
 * journal", and the cause was ordering, not any one command: the boot
 * fired a force-sync in the first tick, ahead of the load, which dials
 * every known peer on the same thread that has to draw the journal.
 */
describe("boot sequence", () => {
  /** Let every pending microtask (and a macrotask) run. */
  const flush = () => new Promise((r) => setTimeout(r, 0));

  /** A sequence whose every step is recorded, overridable per test. */
  function steps(over: Partial<BootSequence> = {}) {
    const log: string[] = [];
    const seq: BootSequence = {
      load: async () => {
        log.push("load");
      },
      painted: async () => {
        log.push("painted");
      },
      deepLink: async () => {
        log.push("deepLink");
      },
      firstSync: () => log.push("firstSync"),
      catchUp: () => log.push("catchUp"),
      ...over,
    };
    return { seq, log };
  }

  it("does not sync before the journal is on screen", async () => {
    commands.openTodayJournal.mockResolvedValue(view());
    commands.openJournalFor.mockResolvedValue(view());
    const { boot, state } = harness({ view: () => view() });
    let paint!: () => void;
    const painted = new Promise<void>((r) => {
      paint = r;
    });
    const { seq, log } = steps({
      load: async () => {
        log.push("load");
        await boot.loadTodayWithRetry();
      },
      painted: () => {
        log.push("painted");
        return painted;
      },
      // The real wiring, so this is a claim about `syncNow` and not
      // about a stub named after it.
      firstSync: () => {
        log.push("firstSync");
        void boot.pullAndReload();
      },
    });

    const run = runBootSequence(seq);
    await flush();

    // Today's journal is applied and the skeleton is gone …
    expect(state.applied).toEqual([view()]);
    expect(state.loaded).toBe(true);
    // … and not one peer has been dialled yet.
    expect(commands.syncNow).not.toHaveBeenCalled();
    expect(log).toEqual(["load", "painted"]);

    paint();
    await run;

    expect(commands.syncNow).toHaveBeenCalledTimes(1);
    expect(log).toEqual(["load", "painted", "deepLink", "firstSync", "catchUp"]);
  });

  it("holds the tickers until the journal has painted", async () => {
    let paint!: () => void;
    const painted = new Promise<void>((r) => {
      paint = r;
    });
    const { seq, log } = steps({ painted: () => painted });

    const run = runBootSequence(seq);
    await flush();

    expect(log).toEqual(["load"]);

    paint();
    await run;

    expect(log).toEqual(["load", "deepLink", "firstSync", "catchUp"]);
  });

  it("lets a cold-start deep link land before the first sync", async () => {
    let landed!: () => void;
    const drain = new Promise<void>((r) => {
      landed = r;
    });
    const { seq, log } = steps({
      deepLink: () => {
        log.push("deepLink");
        return drain;
      },
    });

    const run = runBootSequence(seq);
    await flush();

    // `firstSync` reopens whatever page is current when it starts, so
    // racing it against the drain flips the link's target back to today.
    expect(log).toEqual(["load", "painted", "deepLink"]);

    landed();
    await run;

    expect(log).toEqual(["load", "painted", "deepLink", "firstSync", "catchUp"]);
  });

  it("still starts the tickers when the deep-link drain fails", async () => {
    const { seq, log } = steps({
      deepLink: async () => {
        throw new Error("take_pending_deep_link blew up");
      },
    });

    await expect(runBootSequence(seq)).rejects.toThrow("blew up");

    // A running app with no peer poll and no reminder delivery is a
    // worse outcome than a lost deep link.
    expect(log).toEqual(["load", "painted", "firstSync", "catchUp"]);
  });

  it("waits for a second frame, not the first", async () => {
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) =>
      frames.push(cb),
    );
    let resolved = false;
    void nextPaint().then(() => {
      resolved = true;
    });

    expect(frames).toHaveLength(1);
    frames[0]!(0);
    await new Promise((r) => setTimeout(r, 0));

    // A `requestAnimationFrame` callback runs BEFORE the paint it
    // belongs to, so work started here is still ahead of the pixels.
    expect(resolved).toBe(false);
    expect(frames).toHaveLength(2);

    frames[1]!(0);
    await new Promise((r) => setTimeout(r, 0));

    expect(resolved).toBe(true);
    vi.unstubAllGlobals();
  });
});
