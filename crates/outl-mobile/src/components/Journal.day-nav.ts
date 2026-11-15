/**
 * Moving between journal days, and knowing which one is today.
 *
 * Every way of changing day lands here: the header chevrons, the week
 * strip (tap and swipe), the calendar, the bottom bar's Journal tab,
 * and coming back to the app after a while. "Today" is the backend's
 * `today_slug`, re-read on every return to the foreground so a phone
 * left open past midnight agrees with the desktop about the date.
 *
 * Must be created from inside the `Journal` component body: it arms a
 * `visibilitychange` listener and its `onCleanup` needs that owner.
 */

import { createSignal, onCleanup } from "solid-js";
import type { PageView } from "@outl/shared/api/types";
import { nextDay, openJournalFor, openTodayJournal, previousDay, todaySlug } from "@outl/shared/api/commands";
import { shouldWakeToToday } from "@outl/shared/journal";

import { haptic } from "../lib/haptics";

export interface DayNavDeps {
  view: () => PageView | null;
  editingId: () => string | null;
  applyView: (v: PageView) => void;
  withError: <T>(fn: () => Promise<T>) => Promise<T | undefined>;
}

export function createDayNav(deps: DayNavDeps) {
  const [today, setToday] = createSignal<string | null>(null);
  let disposed = false;
  let hiddenAt: number | null = null;
  let todayWhenHidden: string | null = null;

  async function refreshToday(): Promise<string | null> {
    try {
      const t = await todaySlug();
      if (!disposed) setToday(t);
      return t;
    } catch (e) {
      // The affordances that need it stay hidden until it resolves.
      console.warn("failed to resolve today's slug", e);
      return null;
    }
  }

  async function open(slug: string) {
    haptic("light");
    const next = await deps.withError(() => openJournalFor(slug));
    if (next) deps.applyView(next);
  }

  async function step(by: -1 | 1) {
    const cur = deps.view();
    if (!cur || cur.page.kind !== "journal") return;
    const slug = await deps.withError(() =>
      by < 0 ? previousDay(cur.page.slug) : nextDay(cur.page.slug),
    );
    if (slug) await open(slug);
  }

  /** One day either side of **today**, whatever page is open.
   *  `step` walks from the open journal, which on a 2020 page means
   *  2020 — `/yesterday` has to mean yesterday there too. */
  async function dayFromToday(by: -1 | 1) {
    const anchor = today() ?? (await refreshToday());
    if (!anchor) return;
    const slug = await deps.withError(() =>
      by < 0 ? previousDay(anchor) : nextDay(anchor),
    );
    if (slug) await open(slug);
  }

  async function jumpToday() {
    haptic("light");
    const next = await deps.withError(openTodayJournal);
    if (next) deps.applyView(next);
  }

  void refreshToday();
  if (typeof document !== "undefined") {
    const onVisibility = () => {
      if (document.visibilityState !== "visible") {
        hiddenAt = Date.now();
        todayWhenHidden = today();
        return;
      }
      const away = hiddenAt === null ? 0 : Date.now() - hiddenAt;
      hiddenAt = null;
      void refreshToday().then((t) => {
        if (!t || disposed) return;
        const wake = shouldWakeToToday({
          lastSeenToday: todayWhenHidden,
          today: t,
          currentSlug: deps.view()?.page.slug ?? null,
          hiddenForMs: away,
          editing: deps.editingId() !== null,
        });
        if (wake) void jumpToday();
      });
    };
    document.addEventListener("visibilitychange", onVisibility);
    onCleanup(() => document.removeEventListener("visibilitychange", onVisibility));
  }
  onCleanup(() => {
    disposed = true;
  });

  return {
    /** Today's slug, or `null` until the backend answers. */
    today,
    prevDay: () => step(-1),
    nextDay: () => step(1),
    yesterday: () => dayFromToday(-1),
    tomorrow: () => dayFromToday(1),
    jumpToday,
    /** Open a day; `open_journal_for` creates it, so any date lands. */
    pickDay: open,
  };
}
