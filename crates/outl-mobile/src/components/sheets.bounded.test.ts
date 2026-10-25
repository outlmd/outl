/**
 * A bottom sheet has to stay inside the screen.
 *
 * Every sheet here is `position: fixed` against `bottom-0`, so its
 * height comes from its content and nothing scrolls it: when the
 * content is taller than the viewport, the top of the card is simply
 * above the top edge, unreachable — the grab handle with it, so the
 * sheet cannot even be dismissed by dragging. That was reported from a
 * device for the long-press menu, with a photo.
 *
 * The same shape was live in four more sheets, and landscape is what
 * makes it certain rather than marginal: both platforms allow it (iOS
 * declares all four orientations, Android handles the config change),
 * which halves the height and nearly doubles the width. `Calendar`'s
 * day cells are `aspect-square` in a 7-column grid, so width drives
 * height: on a landscape iPhone the columns are ~101pt and the grid
 * alone came to 627pt inside a 402pt viewport.
 *
 * So each sheet declares how it is bounded, and this reads the source
 * to check the declaration is true. Reading rather than rendering is
 * deliberate, and the limit is worth stating: jsdom computes no layout,
 * so no test in this suite can measure an overflow. What it pins is
 * that the bound is still declared — which is the thing a later edit
 * removes by accident. The same trade as `BlockRow.recursion.test.ts`.
 *
 * A new sheet is caught by `every sheet is accounted for` below, which
 * is the point of the table: an unbounded sheet should fail here rather
 * than arrive in a photo.
 */

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const DIR = import.meta.dirname;
const read = (file: string) => readFileSync(join(DIR, file), "utf8");

/**
 * How a sheet's height is kept inside the viewport.
 *
 * - `dvh` — `max-h-dvh` on the positioned wrapper, which is transparent
 *   and holds the card, so the top padding it carries costs nothing
 *   while the sheet is short.
 * - `viewport-calc` — a `max-height` in the style object, for a sheet
 *   that *is* the card: a top padding there would open a gap above the
 *   grab handle on every portrait phone, where nothing was wrong.
 * - `top-anchored` — a `top` beside `bottom-0`, so both edges are
 *   pinned and the height can never exceed the gap between them.
 * - `vh-cap` — `max-h-[Nvh]` on the card itself.
 * - `fixed-content` — nothing inside can grow, so there is no height to
 *   bound. Needs a reason, and the reason has to be checkable.
 */
type Bound = "dvh" | "viewport-calc" | "top-anchored" | "vh-cap" | "fixed-content";

interface Sheet {
  file: string;
  bound: Bound;
  /** Identifies the positioned element; the default fits every sheet
   *  built on the shared slide-up animation. */
  marker?: string;
  /** Required for `fixed-content`: why nothing in it can grow. */
  reason?: string;
}

const SHEETS: Sheet[] = [
  { file: "BlockContextMenu.tsx", bound: "dvh" },
  { file: "SettingsSheet.tsx", bound: "dvh" },
  { file: "RemindersSheet.tsx", bound: "dvh" },
  { file: "PropertiesSheet.tsx", bound: "dvh" },
  { file: "TemplateSheet.tsx", bound: "dvh" },
  { file: "Calendar.tsx", bound: "viewport-calc" },
  { file: "HistorySheet.tsx", bound: "top-anchored" },
  { file: "TasksSheet.tsx", bound: "top-anchored" },
  { file: "PageSwitcher.tsx", bound: "top-anchored" },
  { file: "DevicesSheet.tsx", bound: "vh-cap" },
  {
    file: "PluginSheet.tsx",
    bound: "vh-cap",
    // Built on a Portal and a full-screen flex column rather than the
    // shared slide-up, so it carries no `outl-sheet-up`.
    marker: "rounded-t-2xl bg-(--color-outl-bg-elev)",
  },
  {
    file: "CaptureSheet.tsx",
    bound: "fixed-content",
    reason: "one textarea, rows={2} and resize-none, so the card cannot grow",
  },
];

/** The attributes of the positioned element, up to the tag's `>`. */
function sheetTag(src: string, marker: string): string {
  const at = src.indexOf(marker);
  expect(at, `marker not found: ${marker}`).toBeGreaterThan(-1);
  const open = src.lastIndexOf("<div", at);
  const rest = src.slice(open);
  const end = rest.search(/\n\s*>/);
  return end === -1 ? rest.slice(0, 600) : rest.slice(0, end);
}

const CHECKS: Record<Bound, RegExp> = {
  dvh: /max-h-dvh/,
  "viewport-calc": /"max-height":\s*"calc\(100dvh/,
  "top-anchored": /\btop:/,
  "vh-cap": /max-h-\[\d+vh\]/,
  // Pinned by the reason below, not by an attribute.
  "fixed-content": /.^/,
};

describe("every bottom sheet is bounded by the screen", () => {
  for (const sheet of SHEETS) {
    it(`${sheet.file} is bounded (${sheet.bound})`, () => {
      const src = read(sheet.file);
      if (sheet.bound === "fixed-content") {
        expect(sheet.reason).toBeTruthy();
        // The claim is "nothing in it grows", so the one input it holds
        // must still be the fixed-size one.
        expect(src).toContain("rows={2}");
        expect(src).toContain("resize-none");
        return;
      }
      expect(sheetTag(src, sheet.marker ?? "outl-sheet-up")).toMatch(
        CHECKS[sheet.bound],
      );
    });
  }

  /**
   * A bound alone only moves the problem: clamped at the viewport, a
   * card whose rows do not fit clips them instead of pushing them off
   * the top. So the rows need somewhere to scroll.
   *
   * A scrolling region is spelled two ways here and both count:
   * Tailwind's `overflow-y-auto`, and this client's own `.ios-scroll`,
   * which declares the same `overflow-y: auto` plus the momentum
   * (`-webkit-overflow-scrolling: touch`) a phone wants. Asking only
   * for the utility failed `PageSwitcher`, which scrolls through the
   * class alone.
   *
   * It asks for the region and nothing more. The first version also
   * demanded `min-h-0` beside `flex-1`, and the four sheets that
   * predate this fix failed it while being correct: flexbox already
   * drops a flex item's automatic minimum size to zero when its
   * main-axis `overflow` is not `visible`, which a scrolling region
   * always satisfies. The `min-h-0` the clamped sheets carry is
   * belt-and-braces, not the invariant, so pinning it would have been
   * pinning a habit.
   */
  for (const sheet of SHEETS) {
    if (sheet.bound === "fixed-content") continue;
    it(`${sheet.file} scrolls its rows instead of clipping them`, () => {
      expect(read(sheet.file)).toMatch(/overflow-y-auto|ios-scroll/);
    });
  }

  it("every sheet is accounted for", () => {
    const listed = new Set(SHEETS.map((s) => s.file));
    const found = readdirSync(DIR)
      .filter((f) => f.endsWith(".tsx") && !f.includes(".test."))
      .filter((f) => {
        const src = read(f);
        return (
          src.includes("outl-sheet-up") ||
          (src.includes("fixed inset-0") && src.includes("justify-end"))
        );
      });
    expect(found.length).toBeGreaterThan(0);
    expect(found.filter((f) => !listed.has(f))).toEqual([]);
  });
});
