/**
 * Presentational chrome for the journal / page header.
 *
 * Pure render, no state and no Tauri commands: every one of these
 * takes props and returns markup. They lived inside `Journal.tsx`
 * only because that is where they were first written, and they were
 * part of why that file reached 3,212 lines.
 *
 * `ChevronLeft` / `ChevronRight` are exported because `Journal` uses
 * `ChevronLeft` directly for its "back to today" affordance, not only
 * through `JournalHeader`.
 */
export function JournalHeader(props: {
  slug: string;
  /** Today's slug, resolved once by the parent `Journal` so the header
   *  and the "back to today" button share a single source of truth.
   *  `null` while the parent is still resolving it. */
  todaySlug: string | null;
  onPrev: () => void;
  onNext: () => void;
  onToday: () => void;
}) {
  const isToday = () =>
    props.todaySlug !== null && props.todaySlug === props.slug;
  return (
    <div class="min-w-0">
      <div class="flex items-center justify-center gap-1.5">
        <button
          type="button"
          aria-label="Previous day"
          onClick={props.onPrev}
          class="shrink-0 rounded-full p-1 text-(--color-outl-accent) active:opacity-50"
        >
          <ChevronLeft />
        </button>
        <h1
          class="cursor-pointer whitespace-nowrap text-[17px] font-semibold leading-tight tracking-tight tabular-nums active:opacity-60"
          onClick={props.onToday}
        >
          {props.slug}
        </h1>
        <button
          type="button"
          aria-label="Next day"
          onClick={props.onNext}
          class="shrink-0 rounded-full p-1 text-(--color-outl-accent) active:opacity-50"
        >
          <ChevronRight />
        </button>
      </div>
      {/* Always rendered (just hidden when not today) so the header
          keeps the same height across day navigation — otherwise the
          whole outline below jumps by ~14px every time the user pages
          past today, which reads as the header "dancing". */}
      <p
        class="mt-0.5 text-center text-[11px] font-medium uppercase tracking-[0.08em] text-(--color-outl-accent)"
        classList={{ invisible: !isToday() }}
        aria-hidden={!isToday()}
      >
        Today
      </p>
    </div>
  );
}

export function PageHeader(props: { title: string; kind: "page" | "journal" | null }) {
  return (
    <div class="min-w-0 text-center">
      <p class="text-[11px] font-medium uppercase tracking-wider text-(--color-outl-fg-dimmer)">
        {props.kind === "journal" ? "Journal" : "Page"}
      </p>
      <h1 class="truncate text-[17px] font-semibold leading-tight tracking-tight">
        {props.title}
      </h1>
    </div>
  );
}

export function ChevronLeft() {
  return (
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
  );
}

export function ChevronRight() {
  return (
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
  );
}
