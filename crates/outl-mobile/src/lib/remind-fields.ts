/**
 * What the "Remind me…" sheet's controls hold, between the two calls
 * that read and write the rule text.
 *
 * **No grammar lives here.** `parseRemindRule` / `formatRemindRule`
 * (`@outl/shared/api/commands`, over `outl_md::remind`) are the only
 * code that reads or writes a `remind::` value; this module moves the
 * same `RemindFields` between that shape and the shapes an `<input
 * type="time">` and a repeat list can bind to.
 *
 * It is a separate file from the sheet because these are the parts
 * with answers that can be wrong — a default time, which repeat the
 * list shows for an interval the user wrote in the TUI, whether a
 * half-filled "until" is sendable — and a component test would reach
 * them only through markup.
 */

import type { RemindFields } from "@outl/shared/api/types";

/**
 * The time a new reminder opens on.
 *
 * 09:00, which is what the prompt this sheet replaces defaulted to.
 * Deliberately **not** "now plus an hour": a reminder a user did not
 * finish choosing a time for should land somewhere predictable, and a
 * rolling default means the same two taps give a different rule each
 * time they are made.
 */
export const DEFAULT_REMIND_HOUR = 9;

/** Fields for a block that carries no rule yet. */
export function defaultRemindFields(): RemindFields {
  return {
    anchor: "at",
    hour: DEFAULT_REMIND_HOUR,
    minute: 0,
    every_minutes: null,
    until: "done",
    until_hour: null,
    until_minute: null,
    until_date: null,
    max_fires: null,
  };
}

/**
 * The repeats the list offers, in order.
 *
 * A closed list on purpose: the grammar takes any `1..=999` of
 * minutes, hours or days, and a phone is the wrong place to type one.
 * An interval outside this list still round-trips — `repeatLabel`
 * names it, and the "Edit as text" field can write one — so the list
 * narrows what is *offered*, never what is *expressible*.
 */
export const REPEAT_CHOICES: readonly { minutes: number | null; label: string }[] = [
  { minutes: null, label: "Once" },
  { minutes: 15, label: "Every 15min" },
  { minutes: 30, label: "Every 30min" },
  { minutes: 60, label: "Every hour" },
  { minutes: 60 * 2, label: "Every 2h" },
  { minutes: 60 * 24, label: "Every day" },
];

/**
 * What to call an interval, including one the list does not offer.
 *
 * A rule written in the TUI (`every 45min`) opens here without its
 * repeat becoming invisible — the alternative is a list where nothing
 * is selected, which reads as "once" and silently drops the repeat on
 * the next save.
 */
export function repeatLabel(minutes: number | null): string {
  if (minutes === null) return "Once";
  const known = REPEAT_CHOICES.find((c) => c.minutes === minutes);
  if (known) return known.label;
  if (minutes % (60 * 24) === 0) return `Every ${minutes / (60 * 24)}d`;
  if (minutes % 60 === 0) return `Every ${minutes / 60}h`;
  return `Every ${minutes}min`;
}

/** Two digits, what both `<input>` formats below want. */
const pad = (n: number) => String(n).padStart(2, "0");

/** `HH:MM`, what an `<input type="time">` binds to. */
export function clockValue(hour: number, minute: number): string {
  return `${pad(hour)}:${pad(minute)}`;
}

/**
 * Read an `<input type="time">` back.
 *
 * `null` for anything that is not `HH:MM` in range, including the
 * empty string a cleared picker gives — the caller keeps the previous
 * value rather than writing a reminder for midnight nobody chose.
 */
export function parseClock(value: string): { hour: number; minute: number } | null {
  const m = /^(\d{1,2}):(\d{2})$/.exec(value.trim());
  if (!m) return null;
  const hour = Number(m[1]);
  const minute = Number(m[2]);
  if (hour > 23 || minute > 59) return null;
  return { hour, minute };
}

/**
 * Switch which stop clause the rule carries, filling the new one with
 * something sendable.
 *
 * A `"time"` with no hour and a `"date"` with no date are both field
 * sets the backend refuses, and a sheet that can reach them has a Save
 * button that does nothing and says nothing. So the switch seeds: an
 * `until` time one hour past the anchor (past it, because a stop at or
 * before the anchor is a clause the parser drops), and today's date.
 */
export function withUntil(
  fields: RemindFields,
  until: "done" | "time" | "date",
  today: string,
): RemindFields {
  if (until === "done") {
    return { ...fields, until, until_hour: null, until_minute: null, until_date: null };
  }
  if (until === "time") {
    const hour = fields.until_hour ?? Math.min(23, hourOf(fields) + 1);
    return {
      ...fields,
      until,
      until_hour: hour,
      until_minute: fields.until_minute ?? 0,
      until_date: null,
    };
  }
  return {
    ...fields,
    until,
    until_hour: null,
    until_minute: null,
    until_date: fields.until_date ?? today,
  };
}

/**
 * The anchor's hour, as the "until" seed reads it.
 *
 * A `now` anchor has no hour of its own (the DTO carries `0, 0`), so
 * the seed falls back to the default rather than offering a stop at
 * 01:00 for a rule that starts this afternoon.
 */
function hourOf(fields: RemindFields): number {
  return fields.anchor === "now" ? DEFAULT_REMIND_HOUR : fields.hour;
}

/** Today as `YYYY-MM-DD`, in the device's own clock. */
export function todayDateValue(now: Date = new Date()): string {
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}
