import { describe, expect, it } from "vitest";

import { aheadOfLogNotice, RECONCILE_COMMAND } from "./ahead-of-log";
import type { MdAheadOfLog } from "../api/types";

const info: MdAheadOfLog = {
  path: "/Users/x/notes/pages/infra.md",
  lines: 12,
  sample: '"restarted the ingest worker at 03:14"',
};

describe("aheadOfLogNotice", () => {
  it("says the page is not syncing, not that something is merely odd", () => {
    // The whole point of the notice: the user must learn the page has
    // stopped converging, not that a file "has a problem".
    expect(aheadOfLogNotice(info, "desktop").title).toBe("This page isn't syncing");
  });

  it("explains why without op-log vocabulary", () => {
    const { body } = aheadOfLogNotice(info, "desktop");
    expect(body).toContain("12 lines");
    expect(body).toContain("only on this device");
    // A user does not have to know what an op log or a sidecar is to act.
    expect(body).not.toMatch(/op log|sidecar|projection|CRDT/i);
  });

  it("names one of the lines at risk instead of only counting them", () => {
    expect(aheadOfLogNotice(info, "desktop").sample).toContain("ingest worker");
  });

  it("gives the desktop user the command to run where they are", () => {
    const { action, command } = aheadOfLogNotice(info, "desktop");
    expect(command).toBe(RECONCILE_COMMAND);
    expect(action).toContain("workspace folder");
  });

  it("tells the mobile user the truth: there is nothing to run from the phone", () => {
    // iOS ships no `outl` binary, so pointing at a terminal here would
    // be an instruction the user cannot follow.
    const { action, command } = aheadOfLogNotice(info, "mobile");
    expect(action).toContain("on your computer");
    expect(action).toContain("nothing to run from the phone");
    // Same command on both clients — only where you run it differs.
    expect(command).toBe(RECONCILE_COMMAND);
  });

  it("tells the user their edits are safe, because they are", () => {
    // `ProjectionWriter` routes through `apply_page_md_with_sidecar_guarded`,
    // which refuses to project over unlogged content exactly like the open
    // path does. An earlier draft of this string said the opposite — written
    // before that guard existed, in the same change — and frightened the user
    // about the one thing the release had just made safe.
    //
    // Asserted on meaning, not on a word. The previous version of this test
    // checked `toContain("overwrite")` and went on passing once the sentence
    // was rewritten to "won't overwrite": the word survived inside its own
    // negation. A copy test that matches a substring of a claim cannot tell
    // the claim from its opposite.
    const { caution } = aheadOfLogNotice(info, "mobile");
    expect(caution).toMatch(/won't overwrite|will not overwrite/);
    expect(caution).not.toMatch(/copy anything you need|will overwrite those/i);
  });

  it("uses the singular for a single line, verb included", () => {
    // The plural was baked into the sentence, so the one branch that
    // cannot use it read "1 line in this file were never recorded".
    const one = aheadOfLogNotice({ ...info, lines: 1 }, "desktop").body;
    expect(one).toContain("1 line in this file was never recorded");
    expect(aheadOfLogNotice(info, "desktop").body).toContain("12 lines in this file were");
  });

  it("carries the file path so the user can go look at it", () => {
    expect(aheadOfLogNotice(info, "desktop").path).toBe(info.path);
  });
});
