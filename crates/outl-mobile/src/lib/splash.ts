/**
 * Dismiss the boot splash when there is something behind it.
 *
 * The splash used to dismiss itself on a CSS timer: a 380ms delay, then
 * a 240ms fade. That is a **620ms floor on every launch**, paid whether
 * the journal was ready in 180ms or in four seconds, and it cannot be
 * tuned away — shortening the delay only moves the failure to the other
 * side, showing an empty page before the first view lands.
 *
 * So the app says when. `index.html` keeps the fade (and a 4s failsafe
 * animation for a launch that never gets here at all); this adds the
 * class that starts it.
 *
 * Mobile-only, and not a candidate for `@outl/shared`: the desktop has
 * no splash. Its equivalent is the loading screen `<App />` already
 * swaps out, which is a Solid branch rather than an element outside
 * `#root`.
 */

/** The element `index.html` renders outside `#root`. */
const SPLASH_ID = "boot-splash";

/** Class `index.html` hangs the fade-out animation on. */
const LEAVING_CLASS = "is-leaving";

/**
 * How long the fade in `index.html` runs. Kept here only to schedule
 * the `remove()`; the animation itself is CSS, so this number being
 * wrong costs a splash that lingers as an invisible, `pointer-events`
 * -free overlay, never a visible one.
 */
const FADE_MS = 240;

let dismissed = false;

/**
 * Start the splash's fade and remove it afterwards. Idempotent.
 *
 * Called when the first real view is applied — not when the workspace
 * opens, and not on a timer. A launch that fails to open a workspace
 * still calls it: the failure state with its Retry button is something
 * to show, and a splash over it is a dead end.
 */
export function dismissBootSplash(): void {
  if (dismissed) return;
  dismissed = true;
  const el = document.getElementById(SPLASH_ID);
  if (!el) return;
  el.classList.add(LEAVING_CLASS);
  // Removed rather than left faded: `visibility: hidden` already stops
  // it taking touches, but an element at `z-index: 2147483647` sitting
  // over the whole app is the kind of thing a later change breaks by
  // accident.
  window.setTimeout(() => el.remove(), FADE_MS);
}
