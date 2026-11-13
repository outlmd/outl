/**
 * Whether a section fed by a reply that lands **after** the first paint
 * is on screen right now.
 *
 * Root [`CLAUDE.md`](../../../../CLAUDE.md) already says where such work
 * belongs: *"Anything landing after the first paint (backlinks, nested
 * pages, a query) shows its own placeholder in its own section, and
 * never unmounts, disables or re-keys the editor."*
 * ([`UX.md` → Never block a keystroke](../../../../UX.md).)
 * This is the half that rule leaves open — **when** the section itself
 * is allowed to be on screen — and the backlinks section got it wrong
 * in both clients.
 *
 * The rule:
 *
 * > A section whose existence is decided by a late reply never appears
 * > and then disappears, and never states an answer it does not have.
 *
 * So a **first load draws nothing at all**. A section that has never
 * had content owes the user no pending mark: there is nothing on screen
 * yet for the silence to be ambiguous against, and any chrome drawn
 * there is chrome that may have to be taken away again. A **refetch**
 * is the opposite case and keeps everything — the rows are already up,
 * the user is looking at them, and they are about to change, which is
 * exactly when a pending mark is owed.
 *
 * ## What this was measured against
 *
 * Driving the real desktop client with a 1.2s backlinks reply, on a
 * page with **no** references: the section mounted at `95px` (header +
 * `loading…` + three skeleton bars) in the same frame as the page, then
 * unmounted when the scan answered zero. Two geometry changes and a
 * pulsing grey box, on a page that ends with nothing — the blink. Most
 * journal days have no backlinks, so that was most journal days, and
 * every peer refresh re-armed it.
 *
 * Mobile had the mirror image: the card mounted immediately on a
 * regular page and said `Linked from 0` / "No backlinks yet" while the
 * scan ran, then contradicted itself. `0 refs` is a wrong answer, not a
 * pending one.
 *
 * ## What it costs, stated rather than discovered
 *
 * - **Zero case** — nothing is ever drawn, so nothing blinks and there
 *   is no reserved gap left under the page either. Reserving space
 *   (the first idea) fixes neither: a reservation that collapses still
 *   blinks, and one that does not is permanent dead space under every
 *   page.
 * - **Many case** — the section appears once, complete, in the frame
 *   the reply lands. It sits below the last block with nothing under it
 *   but the other sections of that same reply, so one geometry change,
 *   all of it below the outline, and none of it under the caret.
 * - **The cost paid** — on a first load the user is not told the scan
 *   is running. Deliberate: backlinks are ambient context nobody asked
 *   for, and `UX.md`'s principle 3 ("silence is the defect") is about
 *   refusals and failures reaching the user, not about announcing
 *   background work they did not request.
 */
export interface LateSection {
  /**
   * The reply for **this** page has landed. Not "a reply has landed" —
   * a stale one from the previous page answers nothing. On the desktop
   * that is `!appState.backlinksLoading`; on mobile the resource's
   * value, which `Journal.tsx` blanks while `loading` so a navigation
   * cannot leave the previous page's rows up.
   */
  answered: boolean;
  /** Rows in hand right now. During a refetch these are the old ones,
   *  which is the point: a refetch keeps what it has. */
  count: number;
  /**
   * This surface shows an empty state when the answer is genuinely
   * zero. Mobile does, on a regular page, so a newcomer discovers that
   * pages can cite each other; the desktop and mobile journals do not,
   * because an empty box every day does not earn its space.
   */
  keepWhenEmpty: boolean;
}

export function lateSectionVisible(s: LateSection): boolean {
  if (s.count > 0) return true;
  return s.answered && s.keepWhenEmpty;
}
