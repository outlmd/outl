/**
 * The one sentence both GUI clients show for a reminder whose block has no
 * text of its own (`remind:: 3pm` on an otherwise empty bullet).
 *
 * The block is alive on a live page, so the copy must never claim it was
 * deleted; a deleted block never reaches a client at all.
 */

import { describe, expect, it } from "vitest";

import { EMPTY_REMINDER_NOTICE, reminderHasText } from "./index";

describe("reminderHasText", () => {
  it("is false for empty and whitespace-only text", () => {
    expect(reminderHasText({ text: "" })).toBe(false);
    expect(reminderHasText({ text: " \t\n " })).toBe(false);
  });

  it("is true once there is anything written", () => {
    expect(reminderHasText({ text: "call mum" })).toBe(true);
  });
});

describe("EMPTY_REMINDER_NOTICE", () => {
  it("is long enough to help", () => {
    expect(EMPTY_REMINDER_NOTICE.length).toBeGreaterThanOrEqual(20);
  });

  it("does not say the block is gone, and leaks no developer vocabulary", () => {
    expect(EMPTY_REMINDER_NOTICE).not.toMatch(/delet|gone|removed|missing|trash/i);
    expect(EMPTY_REMINDER_NOTICE).not.toMatch(
      /unimplemented|not implemented|todo|fixme|no handler|handler|dispatcher/i,
    );
  });

  it("says what the user can do instead", () => {
    expect(EMPTY_REMINDER_NOTICE).toMatch(/add|write/i);
  });
});
