/**
 * Press-and-hold, as one implementation.
 *
 * Touch has no right-click, so long-press is how mobile opens a
 * context menu — on a block, and now on the page title. Two copies of
 * "how long is long" and "how far may the finger drift" drift apart,
 * and the user feels it as one of the two gestures being fussier than
 * the other for no reason they can name.
 */

/** Hold time before the gesture counts. iOS uses ~500ms for its own
 *  context menus; 450 lands just inside that so ours never feels
 *  slower than the system's. */
const HOLD_MS = 450;

/** How far the finger may travel before this is a scroll, not a hold.
 *  Fingers are not styluses: zero tolerance makes the gesture fail for
 *  anyone not resting their hand on the phone. */
const DRIFT_PX = 8;

export interface LongPressOptions {
  /** Fired once the hold completes. */
  onLongPress: () => void;
  /** Skip the gesture entirely (e.g. the row is already editing). */
  disabled?: () => boolean;
}

export interface LongPressHandlers {
  onPointerDown: (e: PointerEvent) => void;
  onPointerMove: (e: PointerEvent) => void;
  onPointerUp: () => void;
  /** Drop a hold in flight. Same effect as lifting the finger, named
   *  for the callers that are not a pointer event (unmount, a sheet
   *  opening over the element). */
  cancel: () => void;
  /** True when the click that follows was produced by a completed
   *  hold, so the caller can swallow it. Reading it clears the flag. */
  consumedClick: () => boolean;
}

/**
 * A press-and-hold recogniser over pointer events.
 *
 * Deliberately not a Solid directive: the two call sites attach these
 * to very different elements (a block row, a header title), and a
 * plain handler bag composes with whatever else those elements need.
 */
export function createLongPress(opts: LongPressOptions): LongPressHandlers {
  let timer: number | undefined;
  let downX = 0;
  let downY = 0;
  let fired = false;

  function cancel() {
    if (timer !== undefined) {
      window.clearTimeout(timer);
      timer = undefined;
    }
  }

  return {
    onPointerDown(e) {
      if (opts.disabled?.()) return;
      downX = e.clientX;
      downY = e.clientY;
      fired = false;
      cancel();
      timer = window.setTimeout(() => {
        fired = true;
        timer = undefined;
        opts.onLongPress();
      }, HOLD_MS);
    },
    onPointerMove(e) {
      if (timer === undefined) return;
      if (
        Math.abs(e.clientX - downX) > DRIFT_PX ||
        Math.abs(e.clientY - downY) > DRIFT_PX
      ) {
        cancel();
      }
    },
    onPointerUp: cancel,
    cancel,
    consumedClick() {
      const was = fired;
      fired = false;
      return was;
    },
  };
}
