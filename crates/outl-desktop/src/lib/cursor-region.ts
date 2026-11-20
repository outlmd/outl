/**
 * The backlinks section as a **cursor region**: which of its rows the
 * keyboard can be on, which one it is on, and entering or leaving Insert
 * on one.
 *
 * Split out of `store.ts` rather than grown there: that file sits at its
 * size ratchet, and these four answers are one subject with one rule
 * underneath them. `lib/cursor-region.test.tsx` was already named after
 * that rule before this module existed. They import the store and the
 * store imports nothing back, so the dependency stays one-way.
 *
 * The rule, in one sentence: **a cursor cannot be in a section that is
 * not on screen.** Everything here is that sentence applied to a
 * different question, and {@link backlinkRowIds} is where it lives.
 */

import type { Backlink } from "@outl/shared/api/types";

import { appState, setAppState, zoomView } from "./store";

/**
 * The backlink rows the keyboard may be on: the section's rows when it is
 * drawn, and none when it is not.
 *
 * **The single owner of "is this section reachable"**, and every question
 * about the region resolves through it — may `j` enter it
 * ({@link backlinkRowIds}'s own callers), is the cursor in it
 * ({@link backlinkCursor}), is one of its editors on screen
 * (`isEditing`, `lib/mode.ts`).
 *
 * Two conditions, and neither is optional:
 *
 * - **`backlinksOpen`.** `ToggleBacklinks` is a **`Global`** binding
 *   (`defaults.rs`), so `Cmd+Shift+B` fires from anywhere, including from
 *   inside a row's textarea — and `togglePanel` flips the flag and
 *   nothing else. No writer of `selectedBacklinkBlockId` or
 *   `editingBacklink` is reached, and removing a node from the DOM does
 *   not reliably fire `blur` in either engine. So the fields survive a
 *   section that has left the screen.
 * - **No zoom.** `<PageSections zoomed>` draws nothing belonging to the
 *   page, so the rows are gone with `backlinksOpen` still true.
 *
 * **Clearing the fields in `togglePanel` was the alternative, and it is
 * the wrong shape**: it spreads the rule to a writer, which is exactly
 * what the cursor-exclusion rule below was rewritten to stop. The
 * accessor is where a rule survives a new writer.
 *
 * Bought by this: with the panel closed under the cursor, `dd` deleted a
 * block on another page that nothing on screen showed, and `Tab`, `o`,
 * `O`, `Cmd+T` and `g D` were the same shape. Before backlink rows became
 * editable the same state was harmless — `targetBlockId()` answered
 * `null` and the keystroke did nothing.
 */
export function backlinkRowIds(): string[] {
  if (!sectionOnScreen()) return [];
  return appState.backlinks.map((b: Backlink) => b.block_id);
}

/**
 * The two conditions above, without materializing the ids.
 *
 * Private, and the reason it exists is a cost rather than a rule: it is
 * the same rule, asked by a caller that only needs the yes/no.
 * {@link backlinkCursor} is evaluated **once per rendered row** (the
 * row's own highlight, `<InlineBacklinks />`), so answering it through
 * `backlinkRowIds().length` allocated an N-string array per row — O(N²)
 * per section render, measured at **965 ms** against 116 ms on the
 * busiest real page (1,556 rows) and 52× slower at 50. Same shape as
 * `<BlockRow />`'s memoised `visualSet`, which exists because a per-row
 * predicate is O(N²) per keystroke.
 *
 * It reads `appState.backlinks.length` rather than every row's
 * `block_id`, which also shrinks each row's subscription set from N+1
 * signals to two.
 *
 * **It is still not O(1), and that is the caller's to handle, not
 * this predicate's.** `zoomView()` is `focusSubtree(outline,
 * focusBlockId)` — an O(page) walk — whenever `focusBlockId` is set but
 * names a block the outline has lost, which is the one render between a
 * peer deleting the zoomed block and `<OutlineView />`'s `createEffect`
 * self-heal clearing it. In that state a per-row reader paid the whole
 * page per row: **851 ms against 166 ms** to mount 1,556 rows over a
 * 688-block outline, and **724.7 ms against 2.6 ms** for one cursor
 * move. `<InlineBacklinks />` wraps {@link backlinkCursor} in a
 * `createMemo` for that reason, the shape `<OutlineView />`'s `focus()`
 * already uses; `InlineBacklinks.cursor-cost.test.tsx` counts the walks
 * and refuses one per row.
 *
 * Reading `appState.focusBlockId` here instead would be *wrong*, not
 * merely cheaper: a stale id means "not zoomed" to what is drawn, so it
 * would put the cursor in a section nothing painted — the divergence
 * {@link backlinkRowIds}'s own doc exists to prevent.
 */
function sectionOnScreen(): boolean {
  return appState.backlinksOpen && zoomView() === null;
}

/**
 * Which backlink row holds the keyboard cursor, or `null` when the cursor
 * is in the outline — or when the section is not on screen at all.
 *
 * **The single owner of that precedence**, and it is the mechanism behind
 * the "mutually exclusive" the two fields are declared with.
 * That used to be a sentence: ~20 call sites write `selectedBlockId`
 * directly and each had to remember to clear `selectedBacklinkBlockId`.
 * The ones that did not left both set, and since every reader checked
 * the backlink field *first*, the keyboard was in the backlinks section
 * while the outline still drew a highlight on the block the user left
 * the cursor on — `j` walked someone else's page and `Enter` navigated
 * away instead of editing.
 *
 * The outline wins because releasing it is the deliberate act: the two
 * navigation handlers that cross the separator write `selectedBlockId`
 * to `null` in the same breath. So a writer that sets one field without
 * clearing the other can no longer put the cursor in two places, which
 * is what the TUI gets from `Focus` being an enum.
 *
 * Every reader of `appState.selectedBacklinkBlockId` goes through this,
 * including the section's own highlight — a stale id painting an accent
 * bar is the visible half of the same bug.
 *
 * **It does not check that the id names a live row.** A row the list
 * dropped is a different fact with a different answer: the keystroke was
 * aimed here and has to say so, rather than fall through to the outline
 * and act on whatever the other cursor happens to hold.
 *
 * It asks {@link sectionOnScreen} and a length, never
 * {@link backlinkRowIds} — identical answer, because `map` preserves
 * length, and this one runs per row. See that predicate for the cost.
 */
export function backlinkCursor(): string | null {
  if (appState.selectedBlockId !== null) return null;
  if (!sectionOnScreen() || appState.backlinks.length === 0) return null;
  return appState.selectedBacklinkBlockId;
}

/**
 * Enter Insert on a backlink row, from whichever gesture asked (a
 * click, `i`, `Enter`).
 *
 * The one writer of `editingBacklink`, because entering Insert here has
 * to release the outline's own editing id in the same breath — two
 * mounted editors is two drafts, and the second commit overwrites the
 * first. Same shape as the cursor rule {@link backlinkCursor} owns, and
 * for the same reason: a writer should not have to know the rule exists.
 *
 * It also parks the keyboard cursor on the row, so leaving Insert
 * returns to the row the user was on rather than to wherever the
 * outline's cursor last was.
 */
export function startBacklinkEdit(target: {
  pageId: string;
  blockId: string;
}): void {
  setAppState({
    editingBlockId: null,
    editingBacklink: target,
    selectedBlockId: null,
    selectedBacklinkBlockId: target.blockId,
  });
}

/** Leave Insert on a backlink row, keeping the cursor on it. */
export function stopBacklinkEdit(): void {
  setAppState("editingBacklink", null);
}
