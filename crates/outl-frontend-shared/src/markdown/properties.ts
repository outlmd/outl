/**
 * Which of a block's `key:: value` properties surface in the outline,
 * and how each one reads.
 *
 * The outline draws them as an **attribute line** ({@link attributeFields}),
 * not as pills. A pill means "flat label you tap to filter", which is
 * a tag; a `key:: value` is a field. {@link propertyChips} is the
 * older chip projection, still used where a client wants the pair
 * flattened into one string.
 *
 * Both GUI clients rendered **nothing** here until now, which made
 * `remind::` invisible at the point of use: you pressed the chord, the
 * backend wrote the rule, and the block looked identical. In a
 * journal-first product a reminder you can't see from the journal is a
 * contradiction.
 *
 * The policy lives here, not in a client, because "which props are
 * chrome and which are prose" is the same question on both. The TUI
 * asks it in Rust; keep the two answers aligned by editing
 * {@link KNOWN_PROPERTIES} and `outl-tui`'s renderer together.
 */

import { REMIND_KEY } from "../api/types";

/** One property, ready to render as a chip. */
export interface PropertyChip {
  key: string;
  value: string;
  /** Leading glyph, when the key has a well-known meaning. */
  icon?: string;
  /** `true` when the key is machinery the user shouldn't read as prose. */
  known: boolean;
}

/**
 * Keys outl gives a glyph to. Everything else renders as a plain
 * `key: value` chip — a user's own `priority:: high` is theirs, not
 * ours to interpret.
 */
const KNOWN_PROPERTIES: Record<string, string> = {
  [REMIND_KEY]: "⏰",
  "auto-run": "▶",
  template: "📋",
};

/**
 * Properties outl writes for its own bookkeeping. They round-trip
 * through the `.md` and belong there, but showing them in the outline
 * is noise — the user didn't type them and can't act on them.
 *
 * These are outl's own *model* state, so they are hidden from the
 * property editors too: offering to edit `collapsed` is offering to
 * corrupt a fold. {@link isImportStamp} is the softer sibling — an
 * import's leftovers, hidden from the outline but still listed (and
 * deletable) in an editor.
 */
const INTERNAL_KEYS = new Set(["id", "from-template", "collapsed"]);

/**
 * Is `key` outl's own bookkeeping rather than the user's metadata?
 *
 * The single owner of the internal-key policy: {@link propertyChips}
 * hides these from the chip row, and the property editors (the mobile
 * sheet's key chips, the desktop's catalogue popup) must not offer
 * them either. A per-client copy of the list drifts the first time a
 * bookkeeping key is added — one client keeps hiding it, the other
 * starts suggesting it.
 *
 * Case-insensitive, matching the dialect's own key folding. Note the
 * page-identity keys (`page-slug`, `page-kind`) are *not* internal in
 * this sense — they are structural and owned by
 * `outl_actions::tree::is_page_model_key` on the Rust side.
 */
export function isInternalKey(key: string): boolean {
  return INTERNAL_KEYS.has(key.toLowerCase());
}

/**
 * An ISO-8601 instant or date, which is what an importer writes and
 * what a person does not type into a note.
 */
const ISO_STAMP = /^\d{4}-\d{2}-\d{2}([T ]\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:?\d{2})?)?$/;

/**
 * Is this pair an import's leftover rather than the user's metadata?
 *
 * `outl import` writes three: `roam-uid` under `--keep-source-ids`, so
 * a re-import matches what it already landed, and `created` / `edited`
 * under `--preserve-timestamps`, copied off the source graph's own
 * clocks. All three round-trip through the `.md` and belong there.
 * None belongs in the outline: this row exists so an authored
 * `remind::` is visible at the point of use, and on a migrated graph
 * these put two or three chips on **every** block, carrying values
 * nobody reads while writing notes.
 *
 * **`roam-uid` is decided by its key, `created` / `edited` by their
 * value**, and the asymmetry is the point. The first is namespaced, so
 * nothing a user writes can collide with it; the other two are
 * ordinary English words a user may well want (`created:: by Roberta`).
 * Hiding those by key alone would silently swallow the user's own
 * metadata, which is the failure this policy exists to prevent — so
 * only an ISO-8601 instant counts, because that is what a machine
 * wrote.
 *
 * Separate from {@link isInternalKey} because the two answer different
 * questions. Both are hidden from the outline; only outl's own model
 * state is also hidden from the property editors. A stamp stays
 * listed there, so a user who wants it gone can delete it.
 */
export function isImportStamp(key: string, value: string): boolean {
  const k = key.toLowerCase();
  if (k === "roam-uid") return true;
  return (k === "created" || k === "edited") && ISO_STAMP.test(value.trim());
}

/**
 * Project a block's properties into renderable chips.
 *
 * Order is preserved (the backend already sorts alphabetically), so
 * two clients showing the same block show the same sequence.
 */
export function propertyChips(
  properties: ReadonlyArray<readonly [string, string]> | undefined,
): PropertyChip[] {
  if (!properties) return [];
  return properties
    .filter(([key, value]) => !isInternalKey(key) && !isImportStamp(key, value))
    .map(([key, value]) => {
      const icon = KNOWN_PROPERTIES[key.toLowerCase()];
      return { key, value, icon, known: icon !== undefined };
    });
}

/** One field of the outline's attribute line. */
export interface PropertyField {
  key: string;
  value: string;
  /**
   * Will this field act on the user? Colour is spent on these and on
   * nothing else — the same rule Todoist and Things follow for an
   * overdue date.
   */
  live: boolean;
}

/**
 * Fields that do something to the user rather than merely describe
 * the block.
 *
 * `remind` fires a notification, so it earns the accent and its own
 * icon. `auto-run` executes code, which is louder still, but only
 * when the user opens the page — it does not interrupt them, so it
 * stays quiet here. Adding a key to this set is a decision about
 * **interruption**, not about importance.
 */
const LIVE_KEYS = new Set<string>([REMIND_KEY]);

/** How many fields fit on the line before the rest become a count. */
export const ATTRIBUTE_LINE_LIMIT = 2;

/**
 * Project a block's properties into the outline's attribute line.
 *
 * Three decisions live here rather than in a renderer, so the two GUI
 * clients cannot disagree about them:
 *
 * - **Key and value stay apart.** A chip baked them into one string,
 *   which is precisely why the key could not recede behind the value.
 * - **A live field sorts first.** The backend sorts alphabetically, so
 *   `remind` lands after `priority` and the one field worth seeing was
 *   the one falling off the end of a capped line.
 * - **The line is one line.** Everything past the cap is counted, and
 *   the count is reachable by tapping, so nothing is lost — only
 *   deferred to the sheet, which is a real form.
 */
export function attributeFields(
  properties: ReadonlyArray<readonly [string, string]> | undefined,
  limit: number = ATTRIBUTE_LINE_LIMIT,
): { fields: PropertyField[]; overflow: number } {
  const visible = (properties ?? [])
    .filter(([key, value]) => !isInternalKey(key) && !isImportStamp(key, value))
    .map(([key, value]) => ({
      key,
      value,
      live: LIVE_KEYS.has(key.toLowerCase()),
    }));

  // Stable: `sort` is stable in every engine we target, so the
  // backend's order survives inside each group.
  visible.sort((a, b) => Number(b.live) - Number(a.live));

  return {
    fields: visible.slice(0, limit),
    overflow: Math.max(0, visible.length - limit),
  };
}

/**
 * The `remind::` rule on a block, or `null`.
 *
 * Sugar over {@link propertyChips} for the common "does this block
 * nag me, and what does it say" question, so a client doesn't
 * re-implement the key lookup (and get the casing wrong).
 */
export function remindRule(
  properties: ReadonlyArray<readonly [string, string]> | undefined,
): string | null {
  const hit = properties?.find(
    ([key]) => key.toLowerCase() === REMIND_KEY,
  );
  return hit ? hit[1] : null;
}
