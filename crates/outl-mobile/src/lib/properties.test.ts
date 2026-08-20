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
