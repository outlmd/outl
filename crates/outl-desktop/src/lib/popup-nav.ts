/**
 * Shared keyboard navigation for the inline autocomplete popups in
 * `BlockRow` (slash `/`, emoji `:`, block-ref `((`, page-ref `[[`).
 *
 * The four popups never co-exist — one trigger is active at a time — and
 * they all move a highlight with the arrows, accept the active item on
 * Enter / Tab, and close on Escape. Before this helper each popup carried
 * its own ~30-line copy of that block; the page-ref one had drifted (a
 * bare `Tab` with no modifier guard, so `Shift+Tab` accepted). This
 * pins the one contract so they can't diverge again.
 */

/** Everything {@link handlePopupNav} needs to drive one popup. `index`
 *  is the current highlight; `setIndex` receives the next absolute
 *  index (not an updater). */
export interface PopupNav<T> {
  items: readonly T[];
  index: number;
  setIndex: (next: number) => void;
  onAccept: (item: T) => void;
  onClose: () => void;
}

/** Only the fields the handler reads — lets callers (and tests) pass a
 *  plain object without a full DOM `KeyboardEvent`. */
type NavKey = Pick<
  KeyboardEvent,
  "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey"
> & {
  preventDefault(): void;
  stopPropagation(): void;
};

/**
 * Route one keydown through the active popup. Returns `true` when the
 * key was consumed — the caller must then `return` from its own handler
 * so the keystroke doesn't also reach the textarea / global dispatcher.
 *
 * - `ArrowDown` / `ArrowUp` cycle the highlight (wrapping).
 * - `Enter` / `Tab` with **no** modifiers accept the highlighted item,
 *   so `Cmd+Enter`, `Shift+Tab`, etc. fall through untouched.
 * - `Escape` closes the popup.
 *
 * A no-op returning `false` when the popup is empty.
 */
export function handlePopupNav<T>(e: NavKey, nav: PopupNav<T>): boolean {
  const len = nav.items.length;
  if (len === 0) return false;

  // Clamp the highlight into range for this event. The list can shrink
  // asynchronously (a slower search result lands with fewer hits) while
  // the caller's `index` signal still holds the old, now out-of-bounds
  // value; without this `items[index]` would be `undefined` and accept
  // would insert nothing / crash.
  const index = Math.min(Math.max(nav.index, 0), len - 1);

  const consume = () => {
    e.preventDefault();
    e.stopPropagation();
  };

  // A modified arrow is a different chord (the block-move chords),
  // not a highlight step. Shift+arrow keeps stepping, as before.
  const chordArrow = e.metaKey || e.ctrlKey || e.altKey;
  if (chordArrow && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
    return false;
  }
  if (e.key === "ArrowDown") {
    consume();
    nav.setIndex((index + 1) % len);
    return true;
  }
  if (e.key === "ArrowUp") {
    consume();
    nav.setIndex((index - 1 + len) % len);
    return true;
  }
  if (
    (e.key === "Enter" || e.key === "Tab") &&
    !e.metaKey &&
    !e.ctrlKey &&
    !e.shiftKey &&
    !e.altKey
  ) {
    consume();
    nav.onAccept(nav.items[index]);
    return true;
  }
  if (e.key === "Escape") {
    consume();
    nav.onClose();
    return true;
  }
  return false;
}
