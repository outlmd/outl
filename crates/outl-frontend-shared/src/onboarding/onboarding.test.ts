import { describe, expect, it } from "vitest";

import {
  CRASH_REPORTS_NOTICE,
  CRASH_REPORTS_SETTING,
  FINISH_CTA,
  STORAGE_STEP,
  SYNC_STEP,
} from "./index";

/**
 * Words that would turn a disclosure into a release note. One list, used
 * by every crash-reporting surface below — two copies of a checklist is
 * how one surface quietly stops being checked for a word the other
 * still is.
 */
const DEV_WORDS = ["telemetry", "sentry", "sdk", "dsn", "stack trace"];

describe("onboarding copy", () => {
  it("storage step has a title and an honest, lock-in-free body", () => {
    expect(STORAGE_STEP.title.length).toBeGreaterThan(0);
    expect(STORAGE_STEP.body.toLowerCase()).toContain("markdown");
    // The promise we must not break: no account.
    expect(STORAGE_STEP.body.toLowerCase()).toContain("no account");
  });

  it("sync step frames pairing as optional and account-free", () => {
    expect(SYNC_STEP.title.length).toBeGreaterThan(0);
    expect(SYNC_STEP.body.toLowerCase()).toContain("peer-to-peer");
    expect(SYNC_STEP.body.toLowerCase()).toContain("no account");
    // Must stay skippable — a single device is a first-class setup.
    expect(SYNC_STEP.skipCta.length).toBeGreaterThan(0);
    expect(SYNC_STEP.pairCta.length).toBeGreaterThan(0);
  });

  it("sync step reassures that one device is fine", () => {
    const joined = SYNC_STEP.bullets.join(" ").toLowerCase();
    expect(SYNC_STEP.bullets.length).toBeGreaterThan(0);
    expect(joined).toContain("one device");
  });

  it("exposes a finish CTA", () => {
    expect(FINISH_CTA.length).toBeGreaterThan(0);
  });
});

// Reporting is on by default, so this copy carries the disclosure the
// opt-out footing rests on. These assert on the **promise**, never the
// sentence, so a copy edit is free and dropping a promise is not. The
// same claim also goes to Apple and Google, which is why neither client
// may phrase it itself.
describe("crash-reporting settings copy", () => {
  it("says what is sent and why that helps", () => {
    const body = CRASH_REPORTS_SETTING.body.toLowerCase();
    expect(CRASH_REPORTS_SETTING.title.length).toBeGreaterThan(0);
    expect(body).toContain("crash");
    // The contribution framing: the user is helping fix a device we do
    // not own and cannot reproduce on.
    expect(body).toContain("fixed");
  });

  it("keeps the promise that notes are never part of a report", () => {
    const body = CRASH_REPORTS_SETTING.body.toLowerCase();
    expect(body).toContain("never");
    expect(body).toContain("notes");
    // Three concrete exclusions, because "we respect your privacy" is
    // not auditable and these are.
    expect(body).toContain("page titles");
    expect(body).toContain("block text");
    expect(body).toContain("file paths");
  });

  it("passes the nudge rules the rest of outl's user-facing copy does", () => {
    // Over 20 characters, and it names what the user gets rather than
    // apologising or reaching for developer words (RFC 0377 section 6).
    expect(CRASH_REPORTS_SETTING.body.length).toBeGreaterThan(20);
    for (const word of DEV_WORDS) {
      expect(CRASH_REPORTS_SETTING.body.toLowerCase()).not.toContain(word);
    }
  });
});

// The first-run strip is the only one of RFC 0377's three disclosure
// surfaces that arrives uninvited, so it is the one carrying prior
// disclosure for a user who never opens Settings. Same rule as the
// describe above: assert the promise, never the sentence.
describe("crash-reporting first-run notice", () => {
  it("says a crash is sent and that sending it is how it gets fixed", () => {
    const body = CRASH_REPORTS_NOTICE.body.toLowerCase();
    expect(body).toContain("crash");
    // Contribution, not apology — the user is helping fix a device we
    // do not own and cannot reproduce on.
    expect(body).toContain("fixed");
  });

  it("keeps the promise that notes are never part of a report", () => {
    const body = CRASH_REPORTS_NOTICE.body.toLowerCase();
    expect(body).toContain("notes");
    expect(body).toContain("never");
  });

  it("names where the opt-out lives, because the strip is not one", () => {
    // The only button on it dismisses the strip. If the copy did not
    // point at Settings, a user who wants out would have nowhere to go
    // from here — and reporting is already on.
    expect(CRASH_REPORTS_NOTICE.body).toContain("Settings");
    expect(CRASH_REPORTS_NOTICE.dismiss.length).toBeGreaterThan(0);
  });

  it("passes the nudge rules the rest of outl's user-facing copy does", () => {
    expect(CRASH_REPORTS_NOTICE.body.length).toBeGreaterThan(20);
    for (const word of DEV_WORDS) {
      expect(CRASH_REPORTS_NOTICE.body.toLowerCase()).not.toContain(word);
    }
  });

  it("stays shorter than the Settings section it points at", () => {
    // Not a style preference. A strip nobody asked for has to be
    // readable in a glance, and the field list it would have to repeat
    // to match Settings is exactly what makes Settings long.
    expect(CRASH_REPORTS_NOTICE.body.length).toBeLessThan(
      CRASH_REPORTS_SETTING.body.length,
    );
  });
});
