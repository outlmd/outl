/**
 * Which day it is, for a client that stays open across midnight.
 *
 * A GUI left running overnight keeps answering with the day it booted
 * on. The desktop header read `Today · November 13` at 08:29 on the
 * 14th and the sidebar calendar ringed the 13th, because each of them
 * took the answer once — a `createResource` keyed on the page slug, a
 * `new Date()` in a component body — and nothing asked again. Nothing
 * was stale in the sense of "needs a refetch": the app was asserting a
 * falsehood about the day, on the surface the user writes into.
 *
 * So "today" is one signal with one owner, and every surface reads it.
 *
 * **The value is always the backend's `today_slug`**, resolved in the
 * configured `[calendar] timezone` (`outl_actions::clock`), never the
 * browser clock — the two disagree in a container, on Crostini, and
 * whenever the user set a zone. The browser clock decides only *when to
 * ask*. Because that schedule can therefore be wrong, the delay is
 * capped: a midnight the browser puts in the wrong place costs at most
 * an hour of a wrong label, not a day.
 *
 * `createDayClock` must be called from inside a component body — it
 * arms a timer and window listeners, and its `onCleanup` needs that
 * owner.
 */

import { createSignal, onCleanup } from "solid-js";

/** Away this long and the next return counts as opening the app. */
export const WAKE_AFTER_MS = 30 * 60 * 1000;

/**
 * Whether coming back to the app should move the user to today.
 *
 * Opening the app lands on today (`UX.md` principle 1), and that has to
 * hold for a client that was backgrounded rather than launched: a phone
 * keeps its webview alive for days, and a desktop window can sit behind
 * a terminal all night.
 *
 * Never mid-edit: the open textarea is the user's, and moving it away
 * would be the interruption principle 6 forbids.
 */
export function shouldWakeToToday(s: {
  /** Today's slug when the client went away. */
  lastSeenToday: string | null;
  /** Today's slug now, from the backend. */
  today: string;
  /** Slug on screen. */
  currentSlug: string | null;
  hiddenForMs: number;
  editing: boolean;
}): boolean {
  if (s.editing) return false;
  if (s.currentSlug === s.today) return false;
  if (s.lastSeenToday !== null && s.lastSeenToday !== s.today) return true;
  return s.hiddenForMs >= WAKE_AFTER_MS;
}

/**
 * Longest a client goes without re-asking which day it is.
 *
 * The cap is what covers a configured timezone the browser does not
 * share: midnight lands somewhere this schedule did not predict, and an
 * hour is the worst case for noticing.
 */
export const MAX_DAY_CHECK_MS = 60 * 60 * 1000;

/** Milliseconds until the next check: local midnight, or the cap. */
export function nextDayCheckDelay(now: Date, maxMs = MAX_DAY_CHECK_MS): number {
  const midnight = new Date(
    now.getFullYear(),
    now.getMonth(),
    now.getDate() + 1,
    0,
    0,
    0,
    0,
  );
  return Math.min(Math.max(midnight.getTime() - now.getTime(), 1), maxMs);
}

/** What a client needs for the clock to send it back to today. */
export interface DayClockWake {
  /** Slug currently on screen. */
  currentSlug: () => string | null;
  /** True while the user is in a block. */
  editing: () => boolean;
  /** Navigate to today's journal. */
  jumpToday: () => void;
}

export interface DayClockDeps {
  /** Resolve today's slug — the backend's `today_slug`. */
  resolve: () => Promise<string>;
  /**
   * Opt-in return-to-today. Omit it and the clock only relabels, which
   * is what a surface with nothing to navigate (a panel) wants.
   */
  wake?: DayClockWake;
}

export interface DayClock {
  /** Today's slug, or `null` until the backend answers. */
  today: () => string | null;
  /** Re-ask now; returns the new slug, or `null` if the call failed. */
  refresh: () => Promise<string | null>;
}

export function createDayClock(deps: DayClockDeps): DayClock {
  const [today, setToday] = createSignal<string | null>(null);
  let disposed = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  /** When the client went away, or `null` while it is here. */
  let awayAt: number | null = null;
  let todayWhenAway: string | null = null;

  async function refresh(): Promise<string | null> {
    try {
      const t = await deps.resolve();
      if (!disposed) setToday(t);
      return t;
    } catch (e) {
      // Surfaces that need it keep the day they have; a wrong label is
      // what this exists to fix, and a crash would be worse.
      console.warn("failed to resolve today's slug", e);
      return null;
    }
  }

  function arm() {
    if (disposed) return;
    timer = setTimeout(() => {
      void refresh().finally(arm);
    }, nextDayCheckDelay(new Date()));
  }

  /**
   * Both `blur` and `visibilitychange` fire for one trip away on some
   * platforms, so the first one wins and the timestamp is not reset by
   * the second — otherwise a 9-hour night reads as 0ms.
   */
  function leaving() {
    if (awayAt !== null) return;
    awayAt = Date.now();
    todayWhenAway = today();
  }

  function returning() {
    const hiddenForMs = awayAt === null ? 0 : Date.now() - awayAt;
    const lastSeenToday = todayWhenAway;
    awayAt = null;
    void refresh().then((t) => {
      const wake = deps.wake;
      if (!t || disposed || !wake) return;
      const go = shouldWakeToToday({
        lastSeenToday,
        today: t,
        currentSlug: wake.currentSlug(),
        hiddenForMs,
        editing: wake.editing(),
      });
      if (go) wake.jumpToday();
    });
  }

  if (typeof document !== "undefined") {
    const onVisibility = () =>
      document.visibilityState === "visible" ? returning() : leaving();
    document.addEventListener("visibilitychange", onVisibility);
    // A desktop window loses and regains focus without a visibility
    // change (alt-tab to a terminal and back), the same reason
    // `installForegroundRefresh` listens to `focus` as well.
    window.addEventListener("focus", returning);
    window.addEventListener("blur", leaving);
    onCleanup(() => {
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("focus", returning);
      window.removeEventListener("blur", leaving);
    });
  }

  onCleanup(() => {
    disposed = true;
    if (timer !== undefined) clearTimeout(timer);
  });

  void refresh();
  arm();

  return { today, refresh };
}
