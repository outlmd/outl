import { describe, expect, it } from "vitest";
import { MOBILE_ACTIONS } from "./palette-actions";

describe("mobile palette actions", () => {
  it("are chordless rows, so the shared ranker can list them", () => {
    for (const row of MOBILE_ACTIONS) {
      expect(row.kind).toBe("action");
      expect(row.chord).toBeNull();
    }
  });

  it("names every action once", () => {
    const names = MOBILE_ACTIONS.map((a) => a.action);
    expect(new Set(names).size).toBe(names.length);
  });
});
