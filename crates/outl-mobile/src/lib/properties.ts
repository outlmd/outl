/**
 * Pure helpers behind the mobile Properties sheet.
 *
 * Everything here is string work with no Solid and no Tauri, so the
 * sheet's two load-bearing decisions — *which keys to offer as chips*
 * and *what a typed key actually means* — are testable without a
 * device.
 *
 * They live under `outl-mobile/src/lib/` rather than `@outl/shared`
 * because the chip affordance is a phone-keyboard answer: on desktop
 * the same catalogue drives a `<datalist>`-style completion, which
 * needs no cap and no "other…" escape. If the desktop ever wants the
 * same shape, promote `suggestedKeys` (see this file's note in the
 * issue #13 report) — the ranking itself already lives once, in
 * `outl_actions::known_keys`.
 */

import type { PropertyKey } from "./api";

/**
 * Keys outl writes for its own bookkeeping, plus the two that *are*
 * the page's identity. Never offered as a chip: `page-slug` /
 * `page-kind` are refused by the backend (renaming is `page_rename`),
 * and the rest are machinery the user did not type.
 *
 * Mirrors `outl_actions::tree::is_page_model_key` plus the
 * `INTERNAL_KEYS` set in `@outl/shared/markdown::propertyChips`, which
 * is why the sheet does not show them either.
 */
const NEVER_SUGGEST = new Set([
  "page-slug",
  "page-kind",
  "id",
  "from-template",
  "collapsed",
]);

/**
 * Normalise what the user typed into a property key.
 *
 * Trailing `::` is the interesting case: the key is spelled
 * `oura-date::` everywhere in the markdown and in every doc, so a user
 * copying it types the colons too. Sending them through would create a
 * distinct `oura-date::` key that renders as `oura-date:::: value`.
 *
 * Whitespace collapses because a key with an inner run of spaces
 * round-trips through the `.md` as a different key than it looks like.
 */
export function normalizeKey(raw: string): string {
  return raw
    .trim()
    .replace(/:+$/, "")
    .trim()
    .replace(/\s+/g, " ");
}

/**
 * The keys to paint as tappable chips in the Add step.
 *
 * Ranked by the backend (most-used first, ties alphabetical), then:
 *
 * - keys the target already carries drop out — tapping one would mean
 *   "edit", and editing is what tapping the existing row does; showing
 *   both makes the same key look like two different actions;
 * - bookkeeping / structural keys drop out (see {@link NEVER_SUGGEST});
 * - the list is capped, because 40 chips is a keyboard with extra
 *   steps. What falls off the end is reachable through "Other…".
 *
 * Matching is case-insensitive to match the catalogue's own folding
 * (`Remind::` and `remind::` are one property in the dialect), so a
 * block carrying `Remind::` is not offered `remind` as if it were new.
 */
export function suggestedKeys(
  known: readonly PropertyKey[],
  existing: readonly string[] = [],
  limit = 10,
): string[] {
  const taken = new Set(existing.map((k) => k.toLowerCase()));
  const out: string[] = [];
  for (const { key } of known) {
    const lower = key.toLowerCase();
    if (taken.has(lower) || NEVER_SUGGEST.has(lower)) continue;
    taken.add(lower); // the catalogue is already folded; belt + braces
    out.push(key);
    if (out.length >= limit) break;
  }
  return out;
}

/**
 * Properties of a block / page as the sheet lists them: whatever the
 * backend sent, minus the bookkeeping keys the user cannot act on.
 *
 * The backend already alpha-sorts, so the order is left alone — two
 * clients showing the same block show the same sequence.
 */
export function editableProperties(
  properties: ReadonlyArray<readonly [string, string]> | undefined,
): Array<[string, string]> {
  if (!properties) return [];
  return properties
    .filter(([key]) => !NEVER_SUGGEST.has(key.toLowerCase()))
    .map(([key, value]) => [key, value] as [string, string]);
}
