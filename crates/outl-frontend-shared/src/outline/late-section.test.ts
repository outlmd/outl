import { describe, expect, it } from "vitest";

import { lateSectionVisible } from "./late-section";

/** The desktop's shape: a first load, then the reply. */
const desktop = (answered: boolean, count: number) =>
  lateSectionVisible({ answered, count, keepWhenEmpty: false });

/** Mobile's regular-page shape: the same, with a discoverability box. */
const page = (answered: boolean, count: number) =>
  lateSectionVisible({ answered, count, keepWhenEmpty: true });

describe("lateSectionVisible", () => {
  it("draws nothing while a first load runs", () => {
    expect(desktop(false, 0)).toBe(false);
    expect(page(false, 0)).toBe(false);
  });

  it("never appears and then disappears", () => {
    // The whole life of a page that turns out to have none: the
    // predicate is false at every step, so there is no frame in which
    // something was drawn that a later frame takes away.
    expect(desktop(false, 0)).toBe(false);
    expect(desktop(true, 0)).toBe(false);
  });

  it("appears once the answer carries rows", () => {
    expect(desktop(true, 3)).toBe(true);
  });

  it("keeps a section that already has rows through a refetch", () => {
    // `answered` is false again (a fetch is in flight) and the rows in
    // hand are the previous ones. Blanking them would read as "this
    // page lost its references".
    expect(desktop(false, 3)).toBe(true);
  });

  it("shows an empty state only as an answer, never as a guess", () => {
    expect(page(false, 0)).toBe(false);
    expect(page(true, 0)).toBe(true);
  });
});
