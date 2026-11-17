/**
 * The decisions behind the "Remind me…" sheet's controls.
 *
 * Nothing here tests the grammar — `outl_md::remind` owns that and has
 * its own round-trip suite. What is pinned is the part a picker can
 * get wrong on its own: a repeat the list does not offer going
 * invisible, and a stop clause that opens unsendable.
 */

import { describe, expect, it } from "vitest";

import type { RemindFields } from "@outl/shared/api/types";
import {
  DEFAULT_REMIND_HOUR,
  REPEAT_CHOICES,
  clockValue,
  defaultRemindFields,
  parseClock,
  repeatLabel,
  todayDateValue,
  withUntil,
} from "./remind-fields";

const TODAY = "2026-11-16";

describe("the time picker's value", () => {
  it("pads both halves, because `<input type=time>` takes HH:MM", () => {
    expect(clockValue(9, 0)).toBe("09:00");
    expect(clockValue(15, 5)).toBe("15:05");
  });

  it("round-trips through the parse", () => {
    expect(parseClock(clockValue(23, 59))).toEqual({ hour: 23, minute: 59 });
  });

  /**
   * A cleared picker gives `""`, and reading that as 00:00 would write
   * a reminder for midnight nobody chose. The caller keeps its
   * previous value on `null`.
   */
  it("refuses an empty or out-of-range value instead of guessing", () => {
    for (const bad of ["", "  ", "24:00", "12:60", "noon", "9"]) {
      expect(parseClock(bad), bad).toBeNull();
    }
  });
});

describe("the repeat list", () => {
  it("opens on Once, so a rule is a single shot until asked otherwise", () => {
    expect(defaultRemindFields().every_minutes).toBeNull();
    expect(REPEAT_CHOICES[0]).toEqual({ minutes: null, label: "Once" });
  });

  /**
   * The case this function exists for: `every 45min` is writable in
   * the TUI and in the sheet's own text field, and a list that cannot
   * name it would show nothing selected — which reads as "Once" and
   * drops the repeat on the next save.
   */
  it("names an interval it does not offer, rather than reading as Once", () => {
    expect(repeatLabel(45)).toBe("Every 45min");
    expect(repeatLabel(60 * 3)).toBe("Every 3h");
    expect(repeatLabel(60 * 24 * 2)).toBe("Every 2d");
    expect(repeatLabel(null)).toBe("Once");
  });

  it("prefers the list's own wording for an interval it offers", () => {
    expect(repeatLabel(60)).toBe("Every hour");
    expect(repeatLabel(60 * 24)).toBe("Every day");
  });
});

describe("switching the stop clause", () => {
  const base: RemindFields = { ...defaultRemindFields(), every_minutes: 60 };

  it("clears the other clause's fields so only one can be set", () => {
    const timed = withUntil(base, "time", TODAY);
    expect(timed.until_date).toBeNull();
    const dated = withUntil(timed, "date", TODAY);
    expect(dated.until_hour).toBeNull();
    expect(dated.until_minute).toBeNull();
    expect(withUntil(dated, "done", TODAY)).toMatchObject({
      until: "done",
      until_hour: null,
      until_minute: null,
      until_date: null,
    });
  });

  /**
   * Both seeds exist because the backend refuses a half-filled clause,
   * and a sheet that can reach one has a Save button that does nothing
   * and explains nothing.
   */
  it("seeds a time past the anchor, since a stop at or before it is dropped", () => {
    const seeded = withUntil({ ...base, hour: 15, minute: 30 }, "time", TODAY);
    expect(seeded.until_hour).toBe(16);
    expect(seeded.until_minute).toBe(0);
  });

  it("seeds a date rather than leaving the field empty", () => {
    expect(withUntil(base, "date", TODAY).until_date).toBe(TODAY);
  });

  it("keeps a value the user already chose", () => {
    const chosen = { ...base, until: "time", until_hour: 21, until_minute: 15 };
    expect(withUntil(chosen, "time", TODAY)).toMatchObject({
      until_hour: 21,
      until_minute: 15,
    });
  });

  /** A `now` anchor carries `0, 0`, so seeding off it would offer a
   *  stop at 01:00 for a rule that starts this afternoon. */
  it("seeds off the default hour when the anchor is `now`", () => {
    const seeded = withUntil({ ...base, anchor: "now", hour: 0, minute: 0 }, "time", TODAY);
    expect(seeded.until_hour).toBe(DEFAULT_REMIND_HOUR + 1);
  });

  it("does not run past the end of the day", () => {
    expect(withUntil({ ...base, hour: 23, minute: 0 }, "time", TODAY).until_hour).toBe(23);
  });
});

describe("today's date", () => {
  it("is the device's own calendar day, not a UTC one", () => {
    // 23:30 local on the 16th is already the 17th in UTC; a date
    // picker that opened on tomorrow would be wrong on half the
    // planet every evening.
    expect(todayDateValue(new Date(2026, 10, 16, 23, 30))).toBe("2026-11-16");
  });
});
