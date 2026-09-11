import { describe, expect, it } from "vitest";

import { splitQuietHours, withQuietEnd } from "./quiet-hours";

describe("splitQuietHours", () => {
  it("takes a wrapping window apart", () => {
    expect(splitQuietHours("22:00-07:00")).toEqual(["22:00", "07:00"]);
  });

  it("takes a same-day window apart", () => {
    expect(splitQuietHours("13:00-14:00")).toEqual(["13:00", "14:00"]);
  });

  it("tolerates spaces around the separator", () => {
    expect(splitQuietHours(" 22:00 - 07:00 ")).toEqual(["22:00", "07:00"]);
  });

  it("renders blank for anything the backend would reject", () => {
    // Showing half of a malformed value in a picker reads as
    // "configured" when it isn't.
    for (const bad of ["", "22:00", "banana", "25:00-07:00", "22:00-07:70"]) {
      expect(splitQuietHours(bad)).toEqual(["", ""]);
    }
  });
});

describe("withQuietEnd", () => {
  it("replaces one end and keeps the other", () => {
    expect(withQuietEnd("22:00-07:00", 0, "23:00")).toBe("23:00-07:00");
    expect(withQuietEnd("22:00-07:00", 1, "08:00")).toBe("22:00-08:00");
  });

  it("stays empty until both ends are set", () => {
    // A half-filled window is not a window; `"22:00-"` would just be
    // an unparseable value the backend drops on the next read.
    const half = withQuietEnd("", 0, "22:00");
    expect(half).toBe("");
    expect(withQuietEnd(half, 1, "07:00")).toBe("");
  });

  it("builds the window once both ends land", () => {
    // The component holds the in-progress pair, so drive it the way
    // the two pickers do: set one, then the other, against the value
    // the component is tracking.
    let raw = "22:00-07:00";
    raw = withQuietEnd(raw, 0, "23:30");
    raw = withQuietEnd(raw, 1, "06:15");
    expect(raw).toBe("23:30-06:15");
  });

  it("clearing either end turns quiet hours off", () => {
    expect(withQuietEnd("22:00-07:00", 0, "")).toBe("");
    expect(withQuietEnd("22:00-07:00", 1, "")).toBe("");
  });

  it("ignores a value the picker could never produce", () => {
    expect(withQuietEnd("22:00-07:00", 1, "99:99")).toBe("");
  });
});

/**
 * The parse rules are owned by Rust, not by this file.
 *
 * `outl_config::RemindersCfg::quiet_window` +`parse_hhmm`
 * (`crates/outl-config/src/schema.rs`) decide what a quiet window is;
 * everything here only has to agree with them. Both directions of a
 * disagreement cost the user their setting:
 *
 * - TS stricter than Rust => a valid `outl.toml` renders blank, and
 *   the next picker touch writes the blank back over it.
 * - TS looser than Rust => the picker shows a window the backend
 *   drops on the next read, so quiet hours silently never fire.
 *
 * So the table below is the Rust accept/reject list, row for row.
 */
describe("agreement with the Rust owner", () => {
  // `parse_hhmm` parses the hour with `u32::from_str`, which is bounded
  // by value (< 24), not by digit count. `"9:00"` is a window Rust
  // accepts and this file used to render blank.
  it("accepts a one-digit hour, the way parse_hhmm does", () => {
    expect(splitQuietHours("9:00-17:00")).toEqual(["09:00", "17:00"]);
  });

  // Same rule on the minute, and on both ends of the window.
  it("accepts a one-digit minute and a one-digit end", () => {
    expect(splitQuietHours("22:5-7:00")).toEqual(["22:05", "07:00"]);
  });

  // `<input type="time">` only renders a value with a two-digit hour;
  // handing it `"9:00"` blanks the picker just as surely as rejecting
  // the string did. Padding is what makes accepting it reach the user.
  it("pads to what a time picker can actually display", () => {
    expect(splitQuietHours("9:5-17:00")).toEqual(["09:05", "17:00"]);
  });

  // `quiet_window` ends with `(start != end).then_some(...)`: a
  // zero-width window is a typo, and reading it as "quiet all day"
  // would silence every reminder the user asked for.
  it("rejects a zero-width window, the way quiet_window does", () => {
    expect(splitQuietHours("22:00-22:00")).toEqual(["", ""]);
    expect(splitQuietHours("9:00-09:00")).toEqual(["", ""]);
  });

  it("never builds a zero-width window out of the pickers", () => {
    // The backend would drop this on the next read, so persisting it
    // hands the user quiet hours that silently do nothing. Spell it
    // "off" instead, which is what it effectively is.
    expect(withQuietEnd("22:00-07:00", 1, "22:00")).toBe("");
    expect(withQuietEnd("22:00-07:00", 0, "07:00")).toBe("");
  });

  it("keeps the other end when a one-digit hour is on disk", () => {
    // The regression that made this worth pinning: the pair came back
    // `["", ""]`, so touching either picker rebuilt from nothing and
    // wrote `""` over a quiet window the user had typed by hand.
    expect(withQuietEnd("9:00-17:00", 1, "18:00")).toBe("09:00-18:00");
    expect(withQuietEnd("9:00-17:00", 0, "10:00")).toBe("10:00-17:00");
  });

  it("mirrors the Rust reject list row for row", () => {
    // `unparseable_quiet_hours_is_ignored_not_fatal`
    // (crates/outl-config/src/schema.rs). Keep the two lists equal.
    for (const bad of ["22:00", "banana", "25:00-07:00", "22:00-22:00", ""]) {
      expect(splitQuietHours(bad)).toEqual(["", ""]);
    }
  });

  it("mirrors the Rust accept list row for row", () => {
    expect(splitQuietHours("22:00-07:00")).toEqual(["22:00", "07:00"]);
    expect(splitQuietHours(" 22:00 - 07:00 ")).toEqual(["22:00", "07:00"]);
    expect(splitQuietHours("00:00-23:59")).toEqual(["00:00", "23:59"]);
    expect(splitQuietHours("9:00-17:00")).toEqual(["09:00", "17:00"]);
  });

  it("rejects out-of-range values on either end", () => {
    // `(hour < 24 && minute < 60)` — the bound is the value, and it
    // applies to start and end alike.
    for (const bad of ["24:00-07:00", "22:60-07:00", "22:00-24:00", "22:00-07:60"]) {
      expect(splitQuietHours(bad)).toEqual(["", ""]);
    }
  });

  it("records the two places we deliberately do NOT mirror Rust", () => {
    // Both are artifacts of `u32::from_str`, not of the wire format:
    // it accepts a leading `+` and unbounded leading zeros, so Rust
    // reads `"+9:00-17:00"` and `"009:0000-17:00"` as 09:00-17:00.
    // No `<input type="time">` can produce either, and widening the
    // regex to match would only let a typo through. Pinned so the
    // divergence is recorded rather than rediscovered.
    expect(splitQuietHours("+9:00-17:00")).toEqual(["", ""]);
    expect(splitQuietHours("009:0000-17:00")).toEqual(["", ""]);
  });
});
