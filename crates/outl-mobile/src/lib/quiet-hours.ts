/**
 * Split / join for the `[reminders] quiet_hours` string.
 *
 * The wire format is a single `"22:00-07:00"` (what
 * `outl_config::RemindersCfg::quiet_window` parses), but a phone has
 * no good way to type that: hyphen and colon each need a keyboard
 * layout switch. Mobile renders two native `<input type="time">`
 * pickers instead, so it needs to take the string apart and put it
 * back together.
 *
 * **The format is owned by Rust, not by this file.**
 * `RemindersCfg::quiet_window` + `parse_hhmm`
 * (`crates/outl-config/src/schema.rs`) decide what a quiet window is;
 * everything here only has to agree with them, and
 * `quiet-hours.test.ts` pins that agreement row for row against the
 * Rust accept / reject lists. Drift in **either** direction costs the
 * user their setting:
 *
 * - stricter here than in Rust => a valid `outl.toml` renders blank in
 *   the pickers, and the next picker touch rebuilds from the blank
 *   pair and writes `""` over a window the user typed by hand;
 * - looser here than in Rust => the pickers show a window the backend
 *   drops on the next read, so quiet hours silently never fire.
 *
 * Lives in the mobile client, not `@outl/shared`, because the desktop
 * edits the same setting as a plain text field. If it ever moves to
 * pickers too, promote this then — shipping it shared today would be
 * speculative.
 */

/**
 * `"22:00-07:00"` -> `["22:00", "07:00"]`.
 *
 * Anything the backend wouldn't accept comes back as a pair of empty
 * strings, so the pickers render blank rather than showing half of a
 * malformed value as if it were configured. "Wouldn't accept" is
 * `quiet_window`'s definition, which includes rejecting `start == end`
 * — a zero-width window read as "quiet all day" would silence every
 * reminder the user asked for.
 *
 * Both ends come back zero-padded. Rust bounds the hour by value, not
 * by digit count, so `"9:00"` is a window it accepts — but an
 * `<input type="time">` only renders a two-digit hour, so handing the
 * picker `"9:00"` blanks it just as surely as rejecting the string
 * did. Padding is what makes accepting it actually reach the user.
 */
export function splitQuietHours(raw: string): [string, string] {
  const parts = raw.split("-");
  if (parts.length !== 2) return ["", ""];
  const from = normalizeTime(parts[0]);
  const to = normalizeTime(parts[1]);
  if (!from || !to || from === to) return ["", ""];
  return [from, to];
}

/**
 * Put one end back and rebuild the wire string.
 *
 * Returns `""` unless **both** ends are set and differ: a half-filled
 * window is not a window, and neither is a zero-width one — persisting
 * `"22:00-"` or `"22:00-22:00"` would only give the backend something
 * it drops on the next read, leaving the user with quiet hours that
 * silently do nothing. Clearing either picker, or setting one end to
 * the other, is therefore how you turn quiet hours off.
 */
export function withQuietEnd(
  raw: string,
  which: 0 | 1,
  value: string,
): string {
  const parts = splitQuietHours(raw);
  parts[which] = normalizeTime(value);
  if (!parts[0] || !parts[1] || parts[0] === parts[1]) return "";
  return `${parts[0]}-${parts[1]}`;
}

/**
 * One end -> canonical `HH:MM`, or `""` when Rust wouldn't take it.
 *
 * Mirrors `parse_hhmm`: split on `:`, bound each half by **value**
 * (`hour < 24 && minute < 60`) rather than by digit count, after
 * trimming the surrounding space `quiet_window` trims.
 *
 * Two inputs Rust accepts and this deliberately does not: a leading
 * `+` and unbounded leading zeros (`"+9:00"`, `"009:0000"`), both
 * artifacts of `u32::from_str` rather than of the wire format. No
 * `<input type="time">` can produce either, so widening the digit
 * bound to match would only let a typo through. The divergence is
 * pinned in the test file so it stays recorded, not rediscovered.
 */
function normalizeTime(v: string): string {
  const m = /^(\d{1,2}):(\d{1,2})$/.exec(v.trim());
  if (!m) return "";
  const hour = Number(m[1]);
  const minute = Number(m[2]);
  if (hour >= 24 || minute >= 60) return "";
  return `${pad(hour)}:${pad(minute)}`;
}

function pad(n: number): string {
  return n < 10 ? `0${n}` : `${n}`;
}
