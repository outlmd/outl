import { For, Show, createMemo, createSignal } from "solid-js";
import {
  DAY_LABELS,
  MONTH_NAMES,
  daysInMonth,
  formatJournalSlug,
  nextMonth,
  parseJournalSlug,
  prevMonth,
} from "@outl/shared/journal";
import { createSheetDrag } from "../lib/sheet-drag";

interface CalendarProps {
  open: boolean;
  /** Slug of currently displayed journal (`YYYY-MM-DD`), or `null` for
   *  a non-journal page. The matching day is highlighted as
   *  "selected" in the grid. */
  selectedSlug: string | null;
  /** Today's slug, resolved by the parent so the header and calendar
   *  can't disagree on what "today" means (midnight rollover). */
  todaySlug: string | null;
  onClose: () => void;
  /** User picked a date. `slug` is `YYYY-MM-DD`. The parent is
   *  responsible for opening the journal (creating it on demand if
   *  the day has no entry yet). */
  onPick: (slug: string) => void;
}

/**
 * Bottom-sheet mini-calendar. Navigates month-by-month and emits a
 * `YYYY-MM-DD` slug on tap. Today and the currently-viewed day are
 * highlighted distinctly; tapping the month/year label snaps the
 * grid back to today's month for quick orientation.
 *
 * Design intent mirrors iOS Calendar / Day One: chevron-based nav,
 * rounded "pill" day cells, accent fill for "selected" and accent
 * text for "today". Drag-to-dismiss handle matches the existing
 * `PageSwitcher` sheet so the two sheets feel like one family.
 */
export function Calendar(props: CalendarProps) {
  const initial =
    parseJournalSlug(props.selectedSlug) ?? parseJournalSlug(props.todaySlug);
  const [year, setYear] = createSignal(
    initial?.year ?? new Date().getFullYear(),
  );
  const [month, setMonth] = createSignal(
    initial?.monthIndex ?? new Date().getMonth(),
  );

  // When the sheet reopens (e.g. user navigated via header chevron and
  // tapped the calendar icon again), realign the visible month to the
  // current view's context so the user lands where they expect.
  let lastOpen = props.open;
  createMemo(() => {
    if (props.open && !lastOpen) {
      const next =
        parseJournalSlug(props.selectedSlug) ??
        parseJournalSlug(props.todaySlug);
      if (next) {
        setYear(next.year);
        setMonth(next.monthIndex);
      }
    }
    lastOpen = props.open;
  });

  const days = createMemo(() => {
    const firstDay = new Date(year(), month(), 1);
    const firstWeekday = firstDay.getDay();
    const total = daysInMonth(year(), month());
    const cells: Array<{ day: number; slug: string } | null> = [];
    for (let i = 0; i < firstWeekday; i += 1) cells.push(null);
    for (let d = 1; d <= total; d += 1) {
      cells.push({ day: d, slug: formatJournalSlug(year(), month(), d) });
    }
    // Pad to whole weeks so the grid stays rectangular.
    while (cells.length % 7 !== 0) cells.push(null);
    return cells;
  });

  function stepMonth(dir: -1 | 1) {
    const next =
      dir === -1 ? prevMonth(year(), month()) : nextMonth(year(), month());
    setYear(next.year);
    setMonth(next.monthIndex);
  }

  function jumpToTodayMonth() {
    const now = new Date();
    setYear(now.getFullYear());
    setMonth(now.getMonth());
  }

  // Sheet drag-to-dismiss — same hook the PageSwitcher uses so the
  // gesture feels identical across every bottom sheet.
  const drag = createSheetDrag(() => props.onClose());

  return (
    <Show when={props.open}>
      <div
        class="fixed inset-0 z-50 bg-black/40 backdrop-blur-md outl-fade-in"
        onClick={props.onClose}
      />
      <div
        class="outl-sheet-up fixed inset-x-0 bottom-0 z-50 flex flex-col overflow-hidden rounded-t-2xl bg-(--color-outl-bg)/85 shadow-2xl backdrop-blur-2xl backdrop-saturate-150"
        style={{
          "padding-bottom": "env(safe-area-inset-bottom)",
          transform: `translateY(${drag.translateY()}px)`,
          transition: drag.dragging()
            ? "none"
            : "transform 220ms var(--ease-spring-in)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <header class="flex items-center gap-3 px-4 py-3">
          <span
            class="mx-auto block h-3 w-16 cursor-grab py-1 active:cursor-grabbing"
            style={{ "touch-action": "none" }}
            onPointerDown={drag.onPointerDown}
            onPointerMove={drag.onPointerMove}
            onPointerUp={drag.onPointerUp}
            onPointerCancel={drag.onPointerCancel}
            aria-label="Drag to close"
            role="button"
          >
            <span
              aria-hidden="true"
              class="block h-1 w-10 mx-auto rounded-full bg-(--color-outl-border)"
            />
          </span>
        </header>

        <div class="flex items-center justify-between px-5 pb-3">
          <button
            type="button"
            aria-label="Previous month"
            onClick={() => stepMonth(-1)}
            class="rounded-full p-2 text-(--color-outl-accent) active:opacity-50"
          >
            <svg
              width="20"
              height="20"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2.5"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M15 18l-6-6 6-6" />
            </svg>
          </button>
          <button
            type="button"
            onClick={jumpToTodayMonth}
            class="text-[17px] font-semibold tabular-nums active:opacity-60"
          >
            {MONTH_NAMES[month()]} {year()}
          </button>
          <button
            type="button"
            aria-label="Next month"
            onClick={() => stepMonth(1)}
            class="rounded-full p-2 text-(--color-outl-accent) active:opacity-50"
          >
            <svg
              width="20"
              height="20"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2.5"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M9 18l6-6-6-6" />
            </svg>
          </button>
        </div>

        <div class="grid grid-cols-7 px-3 pb-1 text-center text-[11px] font-medium uppercase tracking-wider text-(--color-outl-fg-dim)">
          <For each={DAY_LABELS}>{(d) => <div>{d}</div>}</For>
        </div>

        <div class="grid grid-cols-7 gap-1 px-3 pb-5">
          <For each={days()}>
            {(cell) => {
              if (!cell) return <div class="aspect-square" />;
              const isToday = cell.slug === props.todaySlug;
              const isSelected = cell.slug === props.selectedSlug;
              return (
                <button
                  type="button"
                  onClick={() => props.onPick(cell.slug)}
                  class="relative flex aspect-square items-center justify-center rounded-full text-[15px] tabular-nums active:opacity-50"
                  classList={{
                    "bg-(--color-outl-accent) text-white font-semibold":
                      isSelected,
                    "text-(--color-outl-accent) font-semibold":
                      isToday && !isSelected,
                  }}
                >
                  {cell.day}
                  {/* Apple Calendar–style "today marker": a small dot
                      under the number, always visible regardless of
                      which day is currently selected. White on the
                      selected pill, accent purple otherwise. The
                      Calendar.tsx selected-pill state used to swallow
                      the today highlight (accent text → white text on
                      accent bg, indistinguishable from any other
                      selected day); this dot keeps "today" findable in
                      every state. */}
                  <Show when={isToday}>
                    <span
                      aria-hidden="true"
                      class="absolute h-1 w-1 rounded-full"
                      classList={{
                        "bg-white": isSelected,
                        "bg-(--color-outl-accent)": !isSelected,
                      }}
                      style="bottom: 4px;"
                    />
                  </Show>
                </button>
              );
            }}
          </For>
        </div>
      </div>
    </Show>
  );
}
