/**
 * The 30-second tour: three tips (blocks, links, daily) shown in the
 * empty state until the user dismisses them. Same words on both
 * clients; only the gesture hint differs.
 *
 * This file also owns **"show it once, then remember"** for every
 * first-run surface, which is why the crash-reports key sits here next
 * to the tour's. One pair of accessors over one defensive store, so a
 * second surface cannot pick up its own idea of what a failed write
 * means (it means: show it once more, never crash).
 */

export const TIPS_KEY = "outl.first-run-tips";

/**
 * Dismissal key for the crash-reporting strip (RFC 0377 section 6).
 *
 * Separate from {@link TIPS_KEY} because the two are not the same
 * event. The tour teaches the app and only ever appears on an empty
 * page; this strip is a disclosure that has to reach an existing user
 * on the first launch after upgrading, whose journal is not empty.
 * Sharing a key would mean anyone who had dismissed the tour never saw
 * the disclosure at all.
 */
export const CRASH_NOTICE_KEY = "outl.crash-reports-notice";

export type TipClient = "desktop" | "mobile";

export interface Tip {
  id: "blocks" | "links" | "daily";
  glyph: string;
  title: string;
  body: string;
}

export function firstRunTips(client: TipClient): Tip[] {
  const indent = client === "desktop" ? "Tab and Shift+Tab" : "the indent keys above the keyboard";
  const today = client === "desktop" ? "⌘J" : "the Today button";
  return [
    {
      id: "blocks",
      glyph: "•",
      title: "Every line is a block",
      body: `Press Enter for a new one, ${indent} to nest it. Fold a block to hide its children.`,
    },
    {
      id: "links",
      glyph: "[[",
      title: "Link pages as you type",
      body: "Type [[ to link a page or create it. Every link shows up as a backlink on the other side.",
    },
    {
      id: "daily",
      glyph: "◷",
      title: "Today has a page",
      body: `Your journal opens on today. Jump back to it any time with ${today}.`,
    },
  ];
}

/**
 * A page that exists but holds nothing yet: at least one block, every
 * one empty and childless. A fresh journal day lands here (it opens on
 * one empty block), which is where a new user first looks.
 */
export function isBlankOutline(blocks: readonly { text: string; children: readonly unknown[] }[]): boolean {
  return blocks.length > 0 && blocks.every((b) => b.text.trim() === "" && b.children.length === 0);
}

type Store = Pick<Storage, "getItem" | "setItem">;

function safeStore(): Store | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

/**
 * Has this device already dismissed the surface stored under `key`?
 *
 * Device-local on purpose (`UX.md` principle 5): "I have read this" is
 * view state and must not converge to another machine, so it never
 * becomes an Op.
 */
export function dismissedOnce(key: string, store: Store | null = safeStore()): boolean {
  return store?.getItem(key) === "done";
}

/** Remember that the surface under `key` was dismissed. */
export function dismissOnce(key: string, store: Store | null = safeStore()): void {
  try {
    store?.setItem(key, "done");
  } catch {
    // A store that will not take the flag just shows the surface once
    // more. Never a thrown error in front of someone who was writing.
  }
}
