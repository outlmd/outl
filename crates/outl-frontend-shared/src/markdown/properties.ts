/**
 * Which of a block's `key:: value` properties surface in the outline,
 * and how each one reads.
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
    .filter(([key]) => !isInternalKey(key))
    .map(([key, value]) => {
      const icon = KNOWN_PROPERTIES[key.toLowerCase()];
      return { key, value, icon, known: icon !== undefined };
    });
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
