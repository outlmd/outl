/**
 * Journal slug + calendar math shared by every client.
 *
 * The calendar *chrome* stays per-client (mobile: bottom-sheet
 * `Calendar.tsx`; desktop: sidebar mini-grid) — only the pure
 * parsing / date arithmetic lives here, so the two grids can't
 * disagree on what `YYYY-MM-DD` means.
 *
 * Convention: `monthIndex` is **0-based** everywhere in this module
 * (JS `Date` convention). The slug's month is 1-based on the wire;
 * the conversion happens inside parse/format so callers never do
 * the `±1` themselves.
 */

/** Parsed journal slug. `monthIndex` is 0-based (JS `Date` convention). */
export interface JournalDateParts {
  year: number;
  monthIndex: number;
  day: number;
}

const SLUG_RE = /^(\d{4})-(\d{2})-(\d{2})$/;

/**
 * Parse a journal slug (`YYYY-MM-DD`) into its parts, or `null` for
 * anything that doesn't match the shape (page slugs, `null`, …).
 * Purely syntactic — `2026-13-99` parses; use [`journalSlugToDate`]
 * when you need a real calendar date.
 */
export function parseJournalSlug(
  slug: string | null | undefined,
): JournalDateParts | null {
  if (!slug) return null;
  const m = slug.match(SLUG_RE);
  if (!m) return null;
  return {
    year: Number(m[1]),
    monthIndex: Number(m[2]) - 1,
    day: Number(m[3]),
  };
}

/** Render a journal slug (`YYYY-MM-DD`). `monthIndex` is 0-based. */
export function formatJournalSlug(
  year: number,
  monthIndex: number,
  day: number,
): string {
  const pad = (n: number) => n.toString().padStart(2, "0");
  return `${year}-${pad(monthIndex + 1)}-${pad(day)}`;
}

/**
 * Parse a journal slug into a **local-time** `Date` (midnight).
 *
 * Parses the parts explicitly instead of `new Date("2026-06-02")`
 * because the string form is interpreted as midnight **UTC**, which
 * renders the previous day in negative-offset timezones.
 */
export function journalSlugToDate(slug: string | null | undefined): Date | null {
  const parts = parseJournalSlug(slug);
  if (!parts) return null;
  return new Date(parts.year, parts.monthIndex, parts.day);
}

/** Number of days in a month. `monthIndex` is 0-based. */
export function daysInMonth(year: number, monthIndex: number): number {
  return new Date(year, monthIndex + 1, 0).getDate();
}

/** English month names, indexable by `monthIndex`. */
export const MONTH_NAMES = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
] as const;

/**
 * Full weekday names, indexable by `Date#getDay()` (0 = Sunday).
 * English, like {@link MONTH_NAMES}: these are what the UI falls back
 * to when no locale is named, so a journal heading never mixes the
 * host's language into prose this module writes in English.
 */
export const WEEKDAY_NAMES = [
  "Sunday",
  "Monday",
  "Tuesday",
  "Wednesday",
  "Thursday",
  "Friday",
  "Saturday",
] as const;

/**
 * Sunday-first single-letter weekday labels (iOS Calendar style).
 * Pairs with `Date#getDay()` used directly as the leading-pad count.
 * Used by the mobile bottom-sheet calendar.
 */
export const DAY_LABELS = ["S", "M", "T", "W", "T", "F", "S"] as const;

/**
 * Monday-first two-letter weekday labels — the desktop sidebar and the
 * TUI header (`Mo Tu We Th Fr Sa Su`).
 */
export const DAY_LABELS_MONDAY_FIRST = [
  "Mo",
  "Tu",
  "We",
  "Th",
  "Fr",
  "Sa",
  "Su",
] as const;

/** Which day the calendar week starts on. Mirrors `outl_config::WeekStart`. */
export type WeekStart = "monday" | "sunday";

/**
 * Remap a JS `Date#getDay()` (0 = Sunday) onto a weekday index relative
 * to the configured week start (0 = first day … 6 = last day). Every
 * calendar grid pads its first week with this many blanks, so a client
 * that forgot it renders dates one column off under its own header.
 */
export function weekStartIndex(jsDay: number, weekStart: WeekStart): number {
  return weekStart === "monday" ? (jsDay + 6) % 7 : jsDay;
}

/** Year/month pair for calendar navigation. `monthIndex` is 0-based. */
export interface YearMonth {
  year: number;
  monthIndex: number;
}

/** The month before `(year, monthIndex)`, rolling the year at January. */
export function prevMonth(year: number, monthIndex: number): YearMonth {
  if (monthIndex === 0) return { year: year - 1, monthIndex: 11 };
  return { year, monthIndex: monthIndex - 1 };
}

/** The month after `(year, monthIndex)`, rolling the year at December. */
export function nextMonth(year: number, monthIndex: number): YearMonth {
  if (monthIndex === 11) return { year: year + 1, monthIndex: 0 };
  return { year, monthIndex: monthIndex + 1 };
}

/** How a journal page names itself in its header. */
export interface JournalHeading {
  /** `Today`, `Yesterday`, `Tomorrow`, or the weekday (`Thursday`). */
  title: string;
  /** The full date in the user's locale (`October 11, 2026`). */
  date: string;
}

/**
 * Header text for a journal page, relative to `todaySlug`. The ISO slug
 * is a literal (it is the file name), so it stays in the eyebrow in
 * mono; the title is what a person calls the day. `todaySlug` is the
 * backend's `today_slug`, in the configured timezone, never the
 * browser's clock; while it is unknown (`null`) every day gets its
 * weekday. Returns `null` for a slug that is not a real date, so the
 * caller can fall back to the slug.
 *
 * `locale` is opt-in, and omitting it formats in English rather than in
 * the host's language. `Today` / `Yesterday` / `Tomorrow` are English
 * prose this function writes itself, so deferring the date to the host
 * put two languages in one heading — a macOS pt-BR machine read
 * `Today 12 de outubro de 2026`, and a day further out read
 * `quinta-feira`. The app is English until it is translated
 * (`CLAUDE.md` → Coding conventions), so a caller that wants another
 * language has to ask for it, and will have to translate the three
 * relative names in the same breath.
 */
export function journalHeading(
  slug: string,
  todaySlug: string | null,
  locale?: string,
): JournalHeading | null {
  const day = journalSlugToDate(slug);
  if (!day) return null;
  const today = journalSlugToDate(todaySlug);
  const offset = today
    ? Math.round((day.getTime() - today.getTime()) / 86_400_000)
    : Number.NaN;
  const relative: Record<number, string> = { [-1]: "Yesterday", 0: "Today", 1: "Tomorrow" };
  const weekday = locale
    ? day.toLocaleDateString(locale, { weekday: "long" })
    : WEEKDAY_NAMES[day.getDay()];
  const date = locale
    ? day.toLocaleDateString(locale, { month: "long", day: "numeric", year: "numeric" })
    : `${MONTH_NAMES[day.getMonth()]} ${day.getDate()}, ${day.getFullYear()}`;
  return { title: relative[offset] ?? weekday, date };
}
