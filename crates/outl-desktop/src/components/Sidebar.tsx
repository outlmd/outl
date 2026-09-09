import {
  For,
  Show,
  createEffect,
  createMemo,
  createResource,
  createSignal,
  on,
} from "solid-js";

import {
  deletePage,
  listPages,
  openJournalFor,
  openPageBySlug,
} from "@outl/shared/api/commands";
import type { PageMeta, PageView } from "@outl/shared/api/types";
import {
  DAY_LABELS_MONDAY_FIRST,
  daysInMonth,
  formatJournalSlug,
  journalSlugToDate,
  mondayIndex,
} from "@outl/shared/journal";

import { appState, setAppState } from "../lib/store";

/**
 * Left pane — mirrors the TUI's `view::sidebar`.
 *
 * Three stacked sections, top to bottom:
 *
 *   1. **Mini-calendar.** Month view of the journal the user is
 *      currently on (or today's month if they're on a regular page).
 *      Days that have a journal `.md` get a filled dot; today is a
 *      bullseye; the day being viewed is accent-highlighted. Click
 *      any day → opens / creates that day's journal.
 *   2. **Pinned.** Pages with `pinned:: true` (the `pinned` field
 *      now rides `PageMeta` from `outl-actions`). Alphabetical.
 *   3. **Recent.** Pages the user opened most recently in this
 *      session. Tracked client-side in `localStorage` because
 *      "recent for this device" is the right scope — it's not state
 *      that should converge across devices through the op log.
 *
 * The list of journals from the previous Sidebar version is gone:
 * the calendar replaces it (clicking a date IS the journal pick).
 */
export function Sidebar(props: {
  onToday: () => void;
  onPickPage: (view: PageView) => void;
}) {
  const pageKey = () => appState.page?.id ?? "(none)";
  const [pages, { refetch }] = createResource(pageKey, async () => {
    try {
      return await listPages();
    } catch {
      return [] as PageMeta[];
    }
  });

  // ── recent tracking ───────────────────────────────────────────
  //
  // localStorage is the right scope: "recent for this device".
  // Storing it in the op log would force the recents list to
  // converge across every device the user signs into, which is the
  // opposite of useful.
  const RECENT_KEY = "outl-desktop:recent-slugs:v1";
  const RECENT_CAP = 10;

  const [recentSlugs, setRecentSlugs] = createSignal<string[]>(loadRecent());

  function loadRecent(): string[] {
    try {
      const raw = window.localStorage.getItem(RECENT_KEY);
      if (!raw) return [];
      const parsed = JSON.parse(raw) as unknown;
      if (Array.isArray(parsed))
        return parsed.filter((x) => typeof x === "string");
      return [];
    } catch {
      return [];
    }
  }

  function pushRecent(slug: string) {
    setRecentSlugs((prev) => {
      const next = [slug, ...prev.filter((s) => s !== slug)].slice(
        0,
        RECENT_CAP,
      );
      try {
        window.localStorage.setItem(RECENT_KEY, JSON.stringify(next));
      } catch {
        /* private mode / quota — ignore */
      }
      return next;
    });
  }

  // Whenever the active page changes, push its slug onto the
  // recents queue.
  createEffect(
    on(
      () => appState.page?.slug,
      (slug) => {
        if (slug) pushRecent(slug);
      },
    ),
  );

  // ── calendar state ────────────────────────────────────────────

  /**
   * Date the calendar header is anchored on. Tracks the active
   * journal's date when one is open; otherwise stays on today.
   * `viewedMonth` is what we render — derived but `setViewedMonth`
   * is exposed so prev/next month buttons can shift it without
   * changing the active page.
   */
  const activeJournalDate = createMemo(() => {
    const page = appState.page;
    if (page?.kind !== "journal") return null;
    return journalSlugToDate(page.slug);
  });

  const [viewedMonth, setViewedMonth] = createSignal<Date>(
    activeJournalDate() ?? new Date(),
  );

  // Snap the calendar header to the active journal's month whenever
  // it changes (user clicked a different day → calendar follows).
  createEffect(() => {
    const active = activeJournalDate();
    if (!active) return;
    const cur = viewedMonth();
    if (
      cur.getFullYear() !== active.getFullYear() ||
      cur.getMonth() !== active.getMonth()
    ) {
      setViewedMonth(new Date(active.getFullYear(), active.getMonth(), 1));
    }
  });

  /** Set of `YYYY-MM-DD` slugs that have a journal page. */
  const journalSlugSet = createMemo(() => {
    const s = new Set<string>();
    for (const p of pages() ?? []) {
      if (p.kind === "journal") s.add(p.slug);
    }
    return s;
  });

  async function openDay(year: number, monthIdx: number, day: number) {
    const slug = formatJournalSlug(year, monthIdx, day);
    try {
      const view = await openJournalFor(slug);
      props.onPickPage(view);
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    }
  }

  async function openSlug(slug: string) {
    try {
      const view = await openPageBySlug(slug);
      props.onPickPage(view);
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    }
  }

  // ── derived lists ─────────────────────────────────────────────

  const pinned = createMemo(() =>
    (pages() ?? [])
      .filter((p) => p.pinned)
      .sort((a, b) => a.title.localeCompare(b.title)),
  );

  const recentMetas = createMemo(() => {
    const byslug = new Map<string, PageMeta>();
    for (const p of pages() ?? []) byslug.set(p.slug, p);
    const out: PageMeta[] = [];
    for (const slug of recentSlugs()) {
      const meta = byslug.get(slug);
      if (meta) out.push(meta);
    }
    return out;
  });

  /**
   * Delete a page after confirming. Used by the hover × button on
   * Pinned / Recent rows. Journals are excluded at the call site so
   * the user can't accidentally delete a daily note from the sidebar.
   * Shared `deletePage` wrapper handles the backend round-trip +
   * returns today's journal, which we apply via `onPickPage`.
   */
  async function handleDelete(p: PageMeta) {
    const label = p.title || p.slug;
    const ok = window.confirm(
      `Delete page "${label}"?\n\nThis removes the page and all its blocks. ` +
        `The deletion syncs to paired devices.`,
    );
    if (!ok) return;
    try {
      const view = await deletePage(p.slug);
      props.onPickPage(view);
      // Force a re-fetch of the page list so the deleted row
      // disappears from Pinned / Recent immediately.
      void refetch();
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    }
  }

  // ── presentation primitives ───────────────────────────────────

  /** A list row. Active state shows a soft accent bar on the left
   *  instead of a full-row highlight — quieter, more Bear-like.
   *  Shows a hover-only × button when `onDelete` is provided. */
  function Row(props: {
    label: string;
    icon?: string;
    active: boolean;
    onClick: () => void;
    onDelete?: () => void;
  }) {
    return (
      <div class="group relative">
        <button
          type="button"
          onClick={props.onClick}
          class="block w-full truncate rounded-sm px-3 py-[3px] pl-4 text-left text-[12.5px] leading-[1.5] text-(--color-outl-fg-dim) hover:text-(--color-outl-fg)"
        >
          <Show when={props.active}>
            <span
              aria-hidden="true"
              class="absolute top-[6px] bottom-[6px] left-0 w-[2px] rounded-full bg-(--color-outl-accent)"
            />
          </Show>
          <span
            class={
              props.active
                ? "text-(--color-outl-fg)"
                : "text-(--color-outl-fg-dim) group-hover:text-(--color-outl-fg)"
            }
          >
            {props.icon ? (
              <span class="mr-1.5 opacity-70">{props.icon}</span>
            ) : null}
            {props.label}
          </span>
        </button>
        <Show when={props.onDelete}>
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              props.onDelete?.();
            }}
            aria-label={`Delete page "${props.label}"`}
            title="Delete page"
            class="absolute top-1/2 right-1.5 -translate-y-1/2 rounded px-1 text-[13px] text-(--color-outl-fg-dimmer) opacity-0 transition-opacity hover:text-(--color-outl-destructive) group-hover:opacity-100 focus:opacity-100"
          >
            ×
          </button>
        </Show>
      </div>
    );
  }

  function SectionHeader(props: { label: string }) {
    return (
      <div class="px-3 pb-1 text-[10px] font-medium uppercase tracking-[0.1em] text-(--color-outl-fg-dimmer)">
        {props.label}
      </div>
    );
  }

  // ── calendar render ───────────────────────────────────────────

  function Calendar() {
    const month = () => viewedMonth();
    const year = () => month().getFullYear();
    const monthIdx = () => month().getMonth();
    const monthLabel = () =>
      month().toLocaleDateString(undefined, { month: "long", year: "numeric" });
    const today = new Date();
    const todaySlug = formatJournalSlug(
      today.getFullYear(),
      today.getMonth(),
      today.getDate(),
    );

    function stepMonth(delta: number) {
      const cur = month();
      setViewedMonth(new Date(cur.getFullYear(), cur.getMonth() + delta, 1));
    }

    /** Cells for the 6-row × 7-col grid, padded with `null` for
     *  empty leading / trailing days. */
    function cells(): Array<{ day: number; slug: string } | null> {
      const first = new Date(year(), monthIdx(), 1);
      const lead = mondayIndex(first.getDay());
      const total = daysInMonth(year(), monthIdx());
      const out: Array<{ day: number; slug: string } | null> = [];
      for (let i = 0; i < lead; i++) out.push(null);
      for (let d = 1; d <= total; d++) {
        out.push({ day: d, slug: formatJournalSlug(year(), monthIdx(), d) });
      }
      // Pad trailing to a multiple of 7 so the grid keeps its shape
      // when the last week is short.
      while (out.length % 7 !== 0) out.push(null);
      return out;
    }

    return (
      <section class="px-3 pt-1 pb-3">
        <div class="mb-1 flex items-center justify-between">
          <button
            type="button"
            onClick={() => stepMonth(-1)}
            class="rounded px-1 py-0.5 text-[11px] text-(--color-outl-fg-dimmer) hover:text-(--color-outl-fg)"
            aria-label="Previous month"
          >
            ‹
          </button>
          <button
            type="button"
            onClick={() => setViewedMonth(new Date())}
            class="rounded px-1 text-[11px] font-medium text-(--color-outl-fg-dim) hover:text-(--color-outl-fg)"
            title="Jump to current month"
          >
            {monthLabel()}
          </button>
          <button
            type="button"
            onClick={() => stepMonth(1)}
            class="rounded px-1 py-0.5 text-[11px] text-(--color-outl-fg-dimmer) hover:text-(--color-outl-fg)"
            aria-label="Next month"
          >
            ›
          </button>
        </div>

        <div class="grid grid-cols-7 gap-[2px] text-center text-[9.5px] uppercase tracking-wider text-(--color-outl-fg-dimmer)">
          <For each={DAY_LABELS_MONDAY_FIRST}>
            {(d) => <div class="py-[2px]">{d}</div>}
          </For>
        </div>

        <div class="grid grid-cols-7 gap-[2px] text-center text-[11px]">
          <For each={cells()}>
            {(cell) => {
              if (!cell) return <div class="h-6" aria-hidden="true" />;
              const hasJournal = () => journalSlugSet().has(cell.slug);
              const isToday = () => cell.slug === todaySlug;
              const isViewing = () =>
                appState.page?.kind === "journal" &&
                appState.page.slug === cell.slug;

              const cls = () => {
                const base =
                  "h-6 w-full rounded-sm font-mono tabular-nums leading-6 transition-colors";
                if (isViewing()) {
                  return `${base} bg-(--color-outl-accent)/15 font-semibold text-(--color-outl-accent)`;
                }
                if (isToday()) {
                  return `${base} font-semibold text-(--color-outl-fg) ring-1 ring-(--color-outl-accent)/50`;
                }
                if (hasJournal()) {
                  return `${base} text-(--color-outl-fg) hover:bg-(--color-outl-bg-elev)/40`;
                }
                return `${base} text-(--color-outl-fg-dimmer) hover:bg-(--color-outl-bg-elev)/40 hover:text-(--color-outl-fg-dim)`;
              };

              return (
                <button
                  type="button"
                  onClick={() => void openDay(year(), monthIdx(), cell.day)}
                  class={cls()}
                  title={cell.slug}
                >
                  {cell.day}
                </button>
              );
            }}
          </For>
        </div>
      </section>
    );
  }

  return (
    <aside class="flex h-full flex-col text-sm">
      <div class="space-y-1 px-3 pt-6 pb-2">
        <button
          type="button"
          onClick={props.onToday}
          class="w-full rounded-sm px-2 py-1 text-left text-[12.5px] font-medium text-(--color-outl-fg) hover:bg-(--color-outl-bg-elev)/40"
        >
          <span class="mr-1.5 opacity-70">📅</span> Today
        </button>
      </div>

      <Calendar />

      <div class="flex-1 overflow-y-auto px-2 pb-6">
        <Show when={pinned().length > 0}>
          <section class="mb-4 mt-1">
            <SectionHeader label="⭐ Pinned" />
            <For each={pinned()}>
              {(p) => (
                <Row
                  label={p.title}
                  icon={p.icon || (p.kind === "journal" ? "📅" : "📄")}
                  active={appState.page?.slug === p.slug}
                  onClick={() => void openSlug(p.slug)}
                  onDelete={
                    p.kind === "journal" ? undefined : () => void handleDelete(p)
                  }
                />
              )}
            </For>
          </section>
        </Show>

        <Show when={recentMetas().length > 0}>
          <section>
            <SectionHeader label="🕘 Recent" />
            <For each={recentMetas()}>
              {(p) => (
                <Row
                  label={p.title}
                  icon={p.icon || (p.kind === "journal" ? "📅" : "📄")}
                  active={appState.page?.slug === p.slug}
                  onClick={() =>
                    void (p.kind === "journal"
                      ? openJournalFor(p.slug).then(props.onPickPage)
                      : openSlug(p.slug))
                  }
                  onDelete={
                    p.kind === "journal" ? undefined : () => void handleDelete(p)
                  }
                />
              )}
            </For>
          </section>
        </Show>

        <Show when={pinned().length === 0 && recentMetas().length === 0}>
          <div class="px-3 py-2 text-[11px] text-(--color-outl-fg-dimmer)">
            Open a journal day or pin a page (<code>pinned:: true</code>) to
            populate this column.
          </div>
        </Show>
      </div>
    </aside>
  );
}
