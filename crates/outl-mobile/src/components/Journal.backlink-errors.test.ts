import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";

import { backlinkToast } from "./Journal.backlinks";

vi.mock("../lib/haptics", () => ({ haptic: vi.fn() }));

/**
 * A backlink refusal must not inherit the Retry of an unrelated failed
 * outline save. `onNotice` and `onError` land in the same toast on
 * purpose, so both are run.
 */
describe("backlink card toast", () => {
  for (const callback of ["onError", "onNotice"] as const) {
    it(`${callback} clears the pinned retry before showing the message`, () => {
      const calls: string[] = [];
      const toast = backlinkToast(
        () => calls.push("clear"),
        (m) => calls.push(`show:${m}`),
      );
      toast[callback]("nothing was written");
      expect(calls).toEqual(["clear", "show:nothing was written"]);
    });
  }
});

/**
 * The one thing left as source: that `Journal.tsx` hands the card this
 * helper rather than a bare `setError`. Mounting `Journal` needs
 * haptics, a Tauri bridge and a webview (`Journal.load-gate.test.ts`
 * reads it as text for the same reason), and the wiring is the only
 * part a behavioural test of the helper cannot reach. The pattern
 * matches the call, not the arrow shapes around it, so a harmless
 * rewrite of the callbacks does not fail it.
 */
describe("Journal's wiring of the backlink card", () => {
  const JOURNAL = readFileSync(join(import.meta.dirname, "Journal.tsx"), "utf8");

  it("takes the card's messages from backlinkToast, with the retry cleared", () => {
    const start = JOURNAL.indexOf("createBacklinkCard({");
    expect(start).toBeGreaterThan(-1);
    const block = JOURNAL.slice(start, JOURNAL.indexOf("});", start));
    expect(block).toMatch(/\.\.\.backlinkToast\(\s*\(\)\s*=>\s*setErrorRetry\(null\)/);
    expect(block).not.toMatch(/\bonError\s*:|\bonNotice\s*:/);
  });
});
