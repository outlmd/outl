/**
 * The two paths that open the workspace: the boot load behind the
 * skeleton, and the P2P pull-then-reload every refresh affordance and
 * every poll shares.
 *
 * **Why they are one module.** Both are the same four moves — force a
 * sync, open a page, decide whether the reply may replace what is on
 * screen — and both are the only places that hold a user-visible gate
 * open (`loaded` for the skeleton, `syncing` for the two refresh
 * affordances). A gate that is set on one path and cleared on another is
 * how a stall becomes permanent.
 *
 * **Why every call is capped.** On iOS `wry` serves Tauri IPC from the
 * main thread and a non-`async` command runs there, so one slow
 * workspace load pins the UI thread; the app was being SIGKILLed by the
 * scene-update watchdog (0x8BADF00D) with the main thread inside a sync
 * command. A `withTimeout` here does **not** free that thread — only the
 * Rust side can — but it frees the *gate*, which is the difference
 * between a skeleton the user can retry out of and one that never ends.
 * So: no `await` in here is uncapped, and every gate is cleared in a
 * `finally`.
 *
 * `Journal.boot.test.ts` drives both, including the two states that used
 * to be unreachable: a never-settling open, and a sync pass that throws.
 *
 * **And the order they run in** ({@link runBootSequence}), because that
 * order was the whole complaint: the journal was behind a force-sync that
 * dialled every peer first.
 */

import {
  openJournalFor,
  openPageBySlug,
  openTodayJournal,
  reloadWorkspace,
  syncNow,
} from "@outl/shared/api/commands";
import type { PageView } from "@outl/shared/api/types";

import { bootMark, waitWorkspaceReady } from "@outl/shared/boot";

import { withTimeout } from "../lib/async";
import { dismissBootSplash } from "../lib/splash";

/** Cap on a `syncNow` force-sync pass. With an unreachable peer the connect
 *  waits out a 10–30s timeout; awaiting that in the reload path froze the UI.
 *  6s lets a healthy pass through and bounds a dead one so the local reload
 *  always proceeds. */
export const SYNC_TIMEOUT_MS = 6000;
/** Cap on a page or workspace open (`open_today_journal`, `open_journal_for`,
 *  `open_page_by_slug`, `reload_workspace`). Deliberately the longest cap in
 *  the client — longer than the 8s edit cap — because an open replays the
 *  whole op log, so a cold one on a 64k-block graph legitimately takes
 *  seconds and a tighter bound would report a healthy boot as a failure.
 *  It buys the *gate*, not the thread: see the module doc. */
export const OPEN_TIMEOUT_MS = 15000;
/** How long one `waitWorkspaceReady` call blocks before reporting "not
 *  yet". The backend clamps it; this value only decides how often the
 *  loop gets a chance to give up.
 *
 *  **It replaced a 200ms backoff loop.** 50 attempts × 200ms could not
 *  resolve faster than its own interval, so a workspace that became
 *  ready 10ms into a tick waited out the remaining 190ms, every launch,
 *  and every attempt was an IPC round-trip that had to be served on the
 *  main thread. The wait is latched in Rust now
 *  (`outl_tauri_shared::WorkspaceReady`), so the reply arrives when it
 *  becomes true. */
const READY_WAIT_MS = 2_000;

/** How many times the load asks again after a "not yet".
 *  `READY_WAIT_MS × BOOT_ATTEMPTS` is the same ~10s budget the old
 *  backoff had, spent waiting instead of polling. */
const BOOT_ATTEMPTS = 5;

/** What the two loads need from `Journal`. Accessors, never values. */
export interface JournalBootDeps {
  view: () => PageView | null;
  editingId: () => string | null;
  applyView: (v: PageView) => void;
  setError: (message: string | null) => void;
  /** Releases the skeleton. Set on every exit of the boot load, including failure. */
  setLoaded: (value: boolean) => void;
  /** Arms the retry button inside the released skeleton's place. */
  setLoadFailed: (value: boolean) => void;
  /** The spinner the sync dot and `PullToRefresh` both refuse to start under. */
  setSyncing: (value: boolean) => void;
  /** Toast-on-failure wrapper; resolves `undefined` instead of throwing. */
  withError: <T>(fn: () => Promise<T>) => Promise<T | undefined>;
  /** Re-read the sync dot off the dial outcomes a pass produced. */
  refreshPeerStatus: () => Promise<void>;
}

export interface JournalBoot {
  /** Open today's journal, retrying while the workspace is still booting. */
  loadTodayWithRetry: () => Promise<void>;
  /** Force a sync pass, reload the op log, re-render the open page. */
  pullAndReload: (opts?: { background?: boolean }) => Promise<void>;
  /** Was a peer reload deferred because the user was editing? Clears on read. */
  takePendingReload: () => boolean;
}

export function createJournalBoot(deps: JournalBootDeps): JournalBoot {
  // Monotonic reload generation. Every async reload path captures this at
  // start; a reload whose generation is no longer the latest is a stale read
  // that must NOT clobber a newer one (the mobile "flicker" was an unguarded
  // slow reload applying an older op-log state after a fresh one landed).
  let reloadGen = 0;
  // Set when a peer-driven reload was suppressed because the user was editing.
  // `Journal`'s `createEffect` on `editingId` drains it the moment they leave
  // edit mode, so a sync never swaps the workspace out from under an active
  // edit (that swap re-mints the block id → the `block <id> [Retry]` error +
  // the freeze).
  let reloadPendingWhileEditing = false;

  /** Release the skeleton and dismiss the splash, in that order. */
  function finishLoad(failed: boolean, message: string | null): void {
    deps.setError(message);
    deps.setLoadFailed(failed);
    deps.setLoaded(true);
    // Even on failure: the failure state carries a Retry button, and a
    // splash over it is a dead end.
    dismissBootSplash();
  }

  async function loadTodayWithRetry(): Promise<void> {
    // The skeleton placeholder is the user-facing "still loading"; we wait
    // for the workspace rather than polling for it, and only flip
    // `loadFailed` when the budget is gone or an attempt stalls past its
    // cap.
    deps.setLoadFailed(false);
    for (let i = 0; i < BOOT_ATTEMPTS; i += 1) {
      // Wait for the open instead of racing it. A rejection here is a
      // backend that could not take the wait at all, not a slow open, so
      // it falls through to the attempt below and is reported by it.
      try {
        await withTimeout(
          waitWorkspaceReady(READY_WAIT_MS),
          OPEN_TIMEOUT_MS,
          "Waiting for the workspace",
        );
      } catch {
        // An older backend has no such command. Asking for the journal
        // is still the right next move — it answers `workspace_loading`
        // if the open is in flight, which is what the loop handles.
      }
      try {
        const v = await withTimeout(
          openTodayJournal(),
          OPEN_TIMEOUT_MS,
          "Opening the workspace",
        );
        deps.applyView(v);
        // The first real content is on screen, which is both the
        // measurement and the signal that releases the backend's
        // deferred boot work (sync, the orphan sweep, the snapshot).
        bootMark("first_content");
        finishLoad(false, null);
        return;
      } catch (e) {
        const msg = String(e);
        if (msg.includes("workspace_loading")) {
          // Opener still in flight. No sleep: the wait above is what
          // spends the time now, and it returns the moment the
          // workspace is published.
          continue;
        }
        // Anything else — including the timeout above — is the end of the
        // load, not a retry: `loaded` is released so the skeleton gives way
        // to the failure state and its Retry button.
        finishLoad(true, msg);
        return;
      }
    }
    finishLoad(true, "Workspace took too long to open.");
  }

  /**
   * Core P2P pull, shared by the manual pull-to-refresh and the automatic
   * open/foreground sync. Force a sync pass against every iroh peer NOW (dial
   * instead of waiting for the catch-up tick), reload the local op log, and
   * reopen the current page so the re-render reflects what peers delivered.
   * Best-effort: `syncNow` is a no-op when iroh isn't wired, and tolerated
   * (toast, don't wedge) on a flaky peer so it never blocks the local reload.
   */
  async function pullAndReload(opts?: { background?: boolean }): Promise<void> {
    // `background` = the silent 3s poll. It still pulls + replays the op log,
    // but it only swaps the rendered view when the content ACTUALLY changed and
    // the user isn't editing — so an unchanged poll never re-renders (no scroll
    // jump, no cursor churn) and a desktop/TUI edit arriving mid-typing never
    // yanks the textarea out from under the user. The foreground paths (button,
    // app open, resume) always apply and show the spinner.
    // Input is sacred: never swap the workspace while the user is editing.
    // Reloading re-materializes the tree (which can re-mint the block id under
    // the cursor → `block <id> [Retry]`) and a slow reload freezes the UI. So
    // if a block is being edited, pull the peer's ops to disk in the background
    // (no `await` that blocks the user, capped so a dead peer can't hang it)
    // and mark the reload pending — `Journal`'s `editingId` effect drains it
    // the instant they leave edit mode.
    if (deps.editingId()) {
      reloadPendingWhileEditing = true;
      void deps.withError(() =>
        withTimeout(syncNow(), SYNC_TIMEOUT_MS, "Sync timed out"),
      );
      return;
    }
    const bg = opts?.background ?? false;
    const gen = ++reloadGen;
    if (!bg) deps.setSyncing(true);
    // `finally`, not a trailing call: `syncing` is what `handleRefresh` and
    // `PullToRefresh` both refuse to start a second pass under, so one pass
    // that throws on its way out takes the user's only recovery gesture with
    // it — permanently, since nothing else clears the flag.
    try {
      // Cap the force-sync: with an unreachable peer, `syncNow` waits out the
      // 10–30s connect timeout, and awaiting it here froze the reload for that
      // whole window. Time it out so the local reload always proceeds promptly.
      await deps.withError(() =>
        withTimeout(syncNow(), SYNC_TIMEOUT_MS, "Sync timed out"),
      );
      await deps.withError(() =>
        withTimeout(reloadWorkspace(), OPEN_TIMEOUT_MS, "Reloading the workspace"),
      );
      const cur = deps.view();
      if (cur) {
        const reopen = cur.page.kind === "journal" ? openJournalFor : openPageBySlug;
        const next = await deps.withError(() =>
          withTimeout(reopen(cur.page.slug), OPEN_TIMEOUT_MS, "Opening the page"),
        );
        if (next) {
          // A reload that comes back EMPTY while we already have content is
          // a transient partial read — the op log is mid-ingest / being
          // re-indexed by an inbound sync, not a real "everything was
          // deleted". Never clobber real content with it; the next poll
          // re-reads the settled log. This is what produced the "flip to
          // an empty page (0 ops)" flicker on the 3s poll.
          const clobbersContentWithEmpty =
            next.outline.length === 0 && cur.outline.length > 0;
          const changed =
            JSON.stringify(next.outline) !== JSON.stringify(cur.outline);
          // A newer reload started while our (possibly slow `syncNow`) read was
          // in flight — it read a fresher op log, so applying ours now would flip
          // the page back to the older state. That out-of-order apply is the
          // flicker; drop the superseded read.
          const superseded = gen !== reloadGen;
          if (
            !superseded &&
            !clobbersContentWithEmpty &&
            (!bg || changed) &&
            !deps.editingId()
          ) {
            deps.applyView(next);
          }
        }
      }
      // Re-read the dot off the fresh dial outcomes the force-sync produced.
      void deps.refreshPeerStatus();
    } finally {
      if (!bg) deps.setSyncing(false);
    }
  }

  return {
    loadTodayWithRetry,
    pullAndReload,
    takePendingReload: () => {
      if (!reloadPendingWhileEditing) return false;
      reloadPendingWhileEditing = false;
      return true;
    },
  };
}

/**
 * Resolve once the browser has painted the frame the caller just produced.
 *
 * Two frames, not one: a `requestAnimationFrame` callback runs *before*
 * the paint it belongs to, so a Tauri command started there is still
 * ahead of the pixels on the thread that draws them. The second frame can
 * only run after the first has been presented. A `setTimeout(0)` is worse
 * than nothing — the task queue drains before the frame.
 *
 * Falls back to a macrotask where `requestAnimationFrame` does not exist,
 * so the boot sequence always advances.
 */
export function nextPaint(): Promise<void> {
  if (typeof requestAnimationFrame !== "function") {
    return new Promise((resolve) => {
      setTimeout(resolve, 0);
    });
  }
  return new Promise((resolve) => {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => resolve());
    });
  });
}

/** The steps of a cold start, in the order {@link runBootSequence} runs them. */
export interface BootSequence {
  /** Open today's journal and release the skeleton. Nothing runs before it. */
  load: () => Promise<void>;
  /** Resolves once the frame `load` produced is on screen ({@link nextPaint}). */
  painted: () => Promise<void>;
  /** A URL that *launched* the app, buffered by the backend. Capped by its caller. */
  deepLink: () => Promise<void>;
  /** The first P2P pull. Started, never awaited. */
  firstSync: () => void;
  /** Plugin registries, the foreground watcher, the recurring tickers. */
  catchUp: () => void;
}

/**
 * What a cold start does, in order, and the only place that order is
 * written down.
 *
 * **The rule: nothing heavy goes in front of `load`.** The journal used
 * to boot with a force-sync fired in the first tick, on the argument that
 * dialling early starts punching the NAT path sooner. True, and paid for
 * with the only thing the user is actually waiting for: `outl_sync_now`
 * dials every known peer, and on iOS `wry` serves Tauri IPC from the main
 * thread, so that dial ran on the thread that had to draw today's
 * journal. A first sync 300ms later costs nothing a user can name; a
 * first paint two seconds later is the entire complaint.
 *
 * Dialling first is still what opens a path to a phone behind carrier
 * NAT, so it is not dropped — it moved behind the paint, where
 * `firstSync` is the same mobile-initiated dial one frame later.
 *
 * **Why `painted` and not just "after the await".** `load` resolving
 * means the signals are set, not that anything is on screen: Solid has
 * applied the DOM mutations and the frame has not been presented yet. A
 * command issued in that continuation lands on the main thread *before*
 * the pixels, which is the bug reordering was supposed to fix.
 *
 * `deepLink` stays awaited, ahead of the two background steps: it may
 * navigate away from today, and `firstSync` reopens whatever page is
 * current when it starts — so letting them race flips a cold-start deep
 * link back to the journal. Waiting on it is also why it is in a
 * `finally`: the sync and the tickers are not the deep link's
 * dependents, and a drain that throws must not leave a running app with
 * no peer poll and no reminder delivery.
 */
export async function runBootSequence(seq: BootSequence): Promise<void> {
  await seq.load();
  await seq.painted();
  try {
    await seq.deepLink();
  } finally {
    seq.firstSync();
    seq.catchUp();
    // Last, inside the `finally`, for the same reason the two steps
    // above are: a deep-link drain that throws must not leave a running
    // app whose launch never finished as far as any measurement knows.
    // It emits the summary line, so a launch missing it in the log is a
    // launch that did not get here.
    bootMark("interactive");
  }
}
