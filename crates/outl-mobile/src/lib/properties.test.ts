import { describe, expect, it } from "vitest";

import {
  editableProperties,
  normalizeKey,
  suggestedKeys,
} from "./properties";

describe("normalizeKey", () => {
  it("strips the `::` a user copies off the markdown", () => {
    expect(normalizeKey("oura-date::")).toBe("oura-date");
    expect(normalizeKey("  related::  ")).toBe("related");
  });

  it("leaves a bare key alone", () => {
    expect(normalizeKey("status")).toBe("status");
  });

  it("keeps a single colon inside the key (not a terminator)", () => {
    expect(normalizeKey("ns:key")).toBe("ns:key");
  });

  it("collapses whitespace so the key matches what it looks like", () => {
    expect(normalizeKey(" page   type ")).toBe("page type");
  });

  it("returns empty for a key that was only punctuation", () => {
    expect(normalizeKey(" :: ")).toBe("");
  });
});

describe("suggestedKeys", () => {
  const catalogue = [
    { key: "icon", uses: 120 },
    { key: "related", uses: 90 },
    { key: "status", uses: 40 },
    { key: "oura-date", uses: 12 },
  ];

  it("keeps the backend's most-used-first order", () => {
    expect(suggestedKeys(catalogue)).toEqual([
      "icon",
      "related",
      "status",
      "oura-date",
    ]);
  });

  it("drops keys the target already carries — tapping one would mean edit", () => {
    expect(suggestedKeys(catalogue, ["icon", "status"])).toEqual([
      "related",
      "oura-date",
    ]);
  });

  it("matches existing keys case-insensitively, like the dialect does", () => {
    // `Remind::` and `remind::` are one property in outl markdown, so
    // offering `remind` on a block that has `Remind` would read as new.
    expect(suggestedKeys([{ key: "remind", uses: 3 }], ["Remind"])).toEqual([]);
  });

  it("takes the catalogue as given — bookkeeping is dropped upstream", () => {
    // `outl_actions::known_keys` filters `page-slug`, `page-kind`,
    // `from-template`, `id` and `collapsed` before they ever reach a
    // client, so all three clients get the same menu. Re-filtering here
    // would be a second owner of that rule, and the desktop (which did
    // not have one) proved the two drift.
    const catalogue = [
      { key: "related", uses: 40 },
      { key: "icon", uses: 1 },
    ];
    expect(suggestedKeys(catalogue)).toEqual(["related", "icon"]);
  });

  it("caps the list — the overflow is reachable through Other…", () => {
    const many = Array.from({ length: 30 }, (_, i) => ({
      key: `k${i}`,
      uses: 30 - i,
    }));
    expect(suggestedKeys(many, [], 4)).toEqual(["k0", "k1", "k2", "k3"]);
  });

  it("survives an empty workspace", () => {
    expect(suggestedKeys([], ["icon"])).toEqual([]);
  });
});

describe("editableProperties", () => {
  it("hides bookkeeping keys the user cannot act on", () => {
    expect(
      editableProperties([
        ["collapsed", "true"],
        ["priority", "high"],
        ["page-slug", "inbox"],
      ]),
    ).toEqual([["priority", "high"]]);
  });

  it("preserves the backend's order", () => {
    expect(
      editableProperties([
        ["a", "1"],
        ["b", "2"],
      ]),
    ).toEqual([
      ["a", "1"],
      ["b", "2"],
    ]);
  });

  it("treats a missing list as empty", () => {
    expect(editableProperties(undefined)).toEqual([]);
  });
});

/**
 * A stamp is listed so it can be deleted, not so it can be read — so
 * it must not be the first thing the sheet shows.
 *
 * The backend sorts alphabetically, which put `created` at the top of
 * every block's sheet on a migrated graph: the dimmest, least useful
 * row opened the list. The outline's own line already ranks by what
 * the user can act on (`attributeFields` puts a live field first);
 * this is the same rule on the other surface.
 */
describe("editableProperties ranking", () => {
  it("sinks an import stamp below what the user wrote", () => {
    const rows = editableProperties([
      ["created", "2026-11-12T09:14:07.690+00:00"],
      ["priority", "high"],
      ["remind", "9am every 1h"],
      ["roam-uid", "abc-123"],
      ["status", "waiting on Ana"],
    ]);
    expect(rows.map(([k]) => k)).toEqual([
      "priority",
      "remind",
      "status",
      "created",
      "roam-uid",
    ]);
  });

  it("keeps the backend's order inside each group", () => {
    // Stable, so the sheet does not reshuffle between two reads of
    // the same block.
    const rows = editableProperties([
      ["zeta", "1"],
      ["alpha", "2"],
    ]);
    expect(rows.map(([k]) => k)).toEqual(["zeta", "alpha"]);
  });

  it("leaves a `created::` the user wrote where it was", () => {
    // Judged by value, like everywhere else: this one is not a stamp.
    const rows = editableProperties([
      ["created", "by Roberta"],
      ["status", "doing"],
    ]);
    expect(rows.map(([k]) => k)).toEqual(["created", "status"]);
  });
});
