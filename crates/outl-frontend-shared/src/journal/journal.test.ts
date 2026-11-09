import { describe, expect, it } from "vitest";

import {
  journalHeading,
  DAY_LABELS,
  DAY_LABELS_MONDAY_FIRST,
  MONTH_NAMES,
  daysInMonth,
  formatJournalSlug,
  journalSlugToDate,
  weekStartIndex,
  nextMonth,
  parseJournalSlug,
  prevMonth,
} from "./index";

describe("parseJournalSlug", () => {
  it("parses a valid slug into 0-based monthIndex parts", () => {
    expect(parseJournalSlug("2026-07-04")).toEqual({
      year: 2026,
      monthIndex: 6,
      day: 4,
    });
  });

  it("rejects non-journal slugs and empty input", () => {
    expect(parseJournalSlug("ai-agent/learning")).toBeNull();
    expect(parseJournalSlug("2026-7-4")).toBeNull(); // unpadded
    expect(parseJournalSlug("2026-07-04-extra")).toBeNull();
    expect(parseJournalSlug("")).toBeNull();
    expect(parseJournalSlug(null)).toBeNull();
    expect(parseJournalSlug(undefined)).toBeNull();
  });

  it("roundtrips through formatJournalSlug", () => {
    for (const slug of ["2026-01-01", "2026-12-31", "1999-02-28"]) {
      const p = parseJournalSlug(slug);
      expect(p).not.toBeNull();
      if (!p) continue;
      expect(formatJournalSlug(p.year, p.monthIndex, p.day)).toBe(slug);
    }
  });
});

describe("formatJournalSlug", () => {
  it("zero-pads month and day", () => {
    expect(formatJournalSlug(2026, 0, 5)).toBe("2026-01-05");
    expect(formatJournalSlug(2026, 11, 31)).toBe("2026-12-31");
  });
});

describe("journalSlugToDate", () => {
  it("builds a local-time date (not midnight UTC)", () => {
    const d = journalSlugToDate("2026-06-02");
    expect(d).not.toBeNull();
    // Local getters must return the slug's own parts regardless of the
    // host timezone — the whole point of parsing parts explicitly.
    expect(d?.getFullYear()).toBe(2026);
    expect(d?.getMonth()).toBe(5);
    expect(d?.getDate()).toBe(2);
  });

  it("returns null for non-journal slugs", () => {
    expect(journalSlugToDate("not-a-date")).toBeNull();
    expect(journalSlugToDate(null)).toBeNull();
  });
});

describe("daysInMonth", () => {
  it("knows 31/30-day months", () => {
    expect(daysInMonth(2026, 0)).toBe(31); // January
    expect(daysInMonth(2026, 3)).toBe(30); // April
  });

  it("handles February: leap vs non-leap", () => {
    expect(daysInMonth(2026, 1)).toBe(28);
    expect(daysInMonth(2024, 1)).toBe(29); // divisible by 4
    expect(daysInMonth(2000, 1)).toBe(29); // divisible by 400
    expect(daysInMonth(1900, 1)).toBe(28); // divisible by 100, not 400
  });
});

describe("prevMonth / nextMonth", () => {
  it("steps within the same year", () => {
    expect(prevMonth(2026, 6)).toEqual({ year: 2026, monthIndex: 5 });
    expect(nextMonth(2026, 6)).toEqual({ year: 2026, monthIndex: 7 });
  });

  it("rolls the year at the boundaries", () => {
    expect(prevMonth(2026, 0)).toEqual({ year: 2025, monthIndex: 11 });
    expect(nextMonth(2026, 11)).toEqual({ year: 2027, monthIndex: 0 });
  });

  it("prev and next are inverses", () => {
    const start = { year: 2026, monthIndex: 0 };
    const forward = nextMonth(start.year, start.monthIndex);
    expect(prevMonth(forward.year, forward.monthIndex)).toEqual(start);
  });
});

describe("labels + weekStartIndex", () => {
  it("exposes 12 month names and 7 weekday labels per convention", () => {
    expect(MONTH_NAMES).toHaveLength(12);
    expect(DAY_LABELS).toHaveLength(7);
    expect(DAY_LABELS_MONDAY_FIRST).toHaveLength(7);
  });

  it("weekStartIndex remaps getDay() onto each configured first day", () => {
    // Monday-first: matches the Rust WeekStart::offset_from_sunday_index.
    expect(weekStartIndex(1, "monday")).toBe(0); // Monday
    expect(weekStartIndex(0, "monday")).toBe(6); // Sunday
    expect(weekStartIndex(6, "monday")).toBe(5); // Saturday
    // Sunday-first: getDay() is already the offset.
    expect(weekStartIndex(0, "sunday")).toBe(0); // Sunday
    expect(weekStartIndex(1, "sunday")).toBe(1); // Monday
    expect(weekStartIndex(6, "sunday")).toBe(6); // Saturday
  });
});

describe("journalHeading", () => {
  const today = "2026-10-11";

  it("names the nearby days relative to today", () => {
    expect(journalHeading("2026-10-11", today, "en-US")?.title).toBe("Today");
    expect(journalHeading("2026-10-10", today, "en-US")?.title).toBe("Yesterday");
    expect(journalHeading("2026-10-12", today, "en-US")?.title).toBe("Tomorrow");
  });

  it("uses the weekday further out and always carries the full date", () => {
    expect(journalHeading("2026-10-08", today, "en-US")).toEqual({
      title: "Thursday",
      date: "October 8, 2026",
    });
  });

  it("does not shift across a DST change", () => {
    // Brazil and the US both move clocks in this window in some years;
    // the day count must round, not floor.
    expect(journalHeading("2026-11-02", "2026-11-03", "en-US")?.title).toBe("Yesterday");
  });

  it("falls back to the weekday while today is unknown", () => {
    expect(journalHeading("2026-10-11", null, "en-US")?.title).toBe("Sunday");
  });

  it("returns null for a slug that is not a date", () => {
    expect(journalHeading("not-a-date", today)).toBeNull();
  });

  // The title is English prose the function writes itself (`Today`), so a
  // date formatted in the host's locale puts two languages in one heading
  // — `Today 12 de outubro de 2026` on a pt-BR machine. The app is English
  // until it is translated; the caller has to ask for anything else.
  it("formats in English when the caller names no locale", () => {
    expect(journalHeading("2026-10-08", today)).toEqual({
      title: "Thursday",
      date: "October 8, 2026",
    });
  });
});
