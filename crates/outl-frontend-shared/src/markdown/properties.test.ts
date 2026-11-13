import { describe, expect, it } from "vitest";

import {
  attributeFields,
  isImportStamp,
  propertyChips,
  remindRule,
} from "./properties";

describe("propertyChips", () => {
  it("gives a known key its glyph and hides the key name", () => {
    const [chip] = propertyChips([["remind", "3pm every 1h"]]);
    expect(chip.icon).toBe("⏰");
    expect(chip.known).toBe(true);
    expect(chip.value).toBe("3pm every 1h");
  });

  it("leaves a user's own property as key + value", () => {
    // `priority:: high` is theirs, not ours to interpret.
    const [chip] = propertyChips([["priority", "high"]]);
    expect(chip.icon).toBeUndefined();
    expect(chip.known).toBe(false);
    expect(chip.key).toBe("priority");
  });

  it("matches the key case-insensitively", () => {
    expect(propertyChips([["Remind", "10am"]])[0].icon).toBe("⏰");
  });

  it("hides outl's own bookkeeping", () => {
    // The user never typed these and can't act on them.
    expect(propertyChips([["id", "01ABC"], ["from-template", "daily"]])).toEqual(
      [],
    );
  });

  it("hides the importer's source id", () => {
    // `roam-uid` is written by `outl-import --keep-source-ids` so a
    // re-import can match a block it already landed. The user never
    // typed it and cannot act on it, which is the same test `id` and
    // `from-template` pass — and `outl_actions::is_suggestable_key`
    // already refuses to offer it as a key to add. This list drifted
    // from that one, so a Roam-imported graph paid a third chip on
    // every block for a value only the importer reads.
    expect(propertyChips([["roam-uid", "radartech-20261112-0611"]])).toEqual(
      [],
    );
    expect(propertyChips([["Roam-UID", "x"]])).toEqual([]);
  });

  it("hides an import's machine timestamps", () => {
    // `outl import --preserve-timestamps` writes these from the source
    // graph's own `:create/time` / `:edit/time`. On a migrated graph
    // that is two chips on *every* block, carrying a value nobody
    // reads while writing notes — the chip row exists so an authored
    // `remind::` is visible, not so a machine stamp is.
    expect(
      propertyChips([
        ["created", "2026-11-12T09:14:07.690+00:00"],
        ["edited", "2026-11-12T09:14:07.690+00:00"],
      ]),
    ).toEqual([]);
  });

  it("keeps a `created::` the user wrote themselves", () => {
    // The key is an ordinary English word, so the rule is about the
    // *value*: a machine stamp is an ISO-8601 instant. Hiding every
    // key spelled `created` would silently swallow a user's own
    // metadata, which is the failure this policy exists to avoid.
    const [chip] = propertyChips([["created", "by Roberta"]]);
    expect(chip?.value).toBe("by Roberta");
  });

  it("asks about the value, not just the key", () => {
    expect(isImportStamp("created", "2026-11-12T09:14:07.690+00:00")).toBe(true);
    expect(isImportStamp("edited", "2026-11-12")).toBe(true);
    expect(isImportStamp("created", "ontem de noite")).toBe(false);
    // `roam-uid` is namespaced, so no value can make it the user's.
    expect(isImportStamp("roam-uid", "anything")).toBe(true);
    expect(isImportStamp("priority", "2026-11-12")).toBe(false);
  });

  it("keeps the backend's order so two clients agree", () => {
    const chips = propertyChips([
      ["auto-run", "true"],
      ["priority", "high"],
      ["remind", "9am"],
    ]);
    expect(chips.map((c) => c.key)).toEqual(["auto-run", "priority", "remind"]);
  });

  it("is empty for a block with no properties", () => {
    expect(propertyChips([])).toEqual([]);
    expect(propertyChips(undefined)).toEqual([]);
  });
});

describe("remindRule", () => {
  it("finds the rule whatever the key casing", () => {
    expect(remindRule([["REMIND", "3pm"]])).toBe("3pm");
  });

  it("is null when the block has no reminder", () => {
    expect(remindRule([["priority", "high"]])).toBeNull();
    expect(remindRule(undefined)).toBeNull();
  });
});

/**
 * The outline shows a block's properties as an **attribute line**, not
 * as a row of pills.
 *
 * A pill means "flat label you tap to filter" — that is a tag. A
 * `key:: value` is a *field*: a named attribute with a value. Painting
 * fields as pills is a category error, and it read as a row of fake
 * buttons that were not buttons. Things, Linear, Apple Reminders and
 * GitHub mobile all render item metadata as a quiet subtitle line and
 * keep the pill for tags.
 */
describe("attributeFields", () => {
  it("keeps the key and value apart so each can be styled", () => {
    // A chip baked them into one string (`status: waiting`), which is
    // why the key could not be dimmed below the value.
    const { fields } = attributeFields([["status", "waiting"]]);
    expect(fields).toEqual([{ key: "status", value: "waiting", live: false }]);
  });

  it("marks the field that will act on the user", () => {
    // Colour is reserved for what interrupts you — Todoist's overdue
    // date, Things' due today. `remind::` fires a notification; a
    // `priority::` just sits there.
    const { fields } = attributeFields([
      ["priority", "high"],
      ["remind", "9am every 1h"],
    ]);
    expect(fields.find((f) => f.key === "remind")?.live).toBe(true);
    expect(fields.find((f) => f.key === "priority")?.live).toBe(false);
  });

  it("puts the live field first, whatever the backend's order", () => {
    // The backend sorts alphabetically, so `remind` lands after
    // `priority` and the one thing worth seeing is the one that falls
    // off the end of a two-field line.
    const { fields } = attributeFields([
      ["aaa", "1"],
      ["remind", "9am"],
      ["zzz", "2"],
    ]);
    expect(fields[0].key).toBe("remind");
  });

  it("caps the line and counts what it dropped", () => {
    // The line is one line. Everything past the cap is reachable
    // through the sheet the line opens.
    const { fields, overflow } = attributeFields(
      [
        ["a", "1"],
        ["b", "2"],
        ["c", "3"],
        ["d", "4"],
      ],
      2,
    );
    expect(fields).toHaveLength(2);
    expect(overflow).toBe(2);
  });

  it("counts nothing when everything fits", () => {
    const { overflow } = attributeFields([["a", "1"]], 2);
    expect(overflow).toBe(0);
  });

  it("hides the same keys the chip row hid", () => {
    // One policy, not two: bookkeeping and import stamps are noise in
    // the outline whatever shape the outline draws them in.
    const { fields, overflow } = attributeFields([
      ["id", "01ABC"],
      ["created", "2026-11-12T09:14:07.690+00:00"],
      ["roam-uid", "x"],
    ]);
    expect(fields).toEqual([]);
    expect(overflow).toBe(0);
  });

  it("is empty for a block with no properties", () => {
    expect(attributeFields(undefined).fields).toEqual([]);
    expect(attributeFields([]).fields).toEqual([]);
  });
});
