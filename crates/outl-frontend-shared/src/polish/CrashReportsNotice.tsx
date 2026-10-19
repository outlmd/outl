import { Show, createSignal } from "solid-js";

import { CRASH_REPORTS_NOTICE } from "../onboarding";
import { CRASH_NOTICE_KEY, dismissOnce, dismissedOnce } from "./firstRun";

/**
 * The first-run crash-reporting strip — surface 1 of the three in
 * RFC 0377 section 6, and the only one that arrives uninvited.
 *
 * Reporting is on by default, so the other two surfaces (the Settings
 * section and the published privacy policy) are both places a user has
 * to decide to go. This is the one that reaches somebody who did not go
 * looking, which is what makes prior disclosure a fact rather than an
 * offer.
 *
 * ## Why it is built the way it is
 *
 * `UX.md` principle 6 bars onboarding that must be dismissed before the
 * first bullet, and principle 2 bars anything that blocks a keystroke.
 * So this is deliberately not a modal and deliberately not a step:
 *
 * - **It never takes focus.** No `autofocus`, no `.focus()`, no focus
 *   trap. The caret stays wherever it was, so the first bullet is
 *   typeable with the strip on screen. Pinned by
 *   `CrashReportsNotice.test.tsx`.
 * - **`role="status"`, not `alert` or `dialog`.** A screen reader
 *   announces it politely, after whatever it was reading. `alert` is
 *   assertive and would interrupt; `dialog` would imply a modal.
 * - **It cannot swallow a click.** The positioned wrapper is
 *   `pointer-events: none` and only the card itself takes the pointer,
 *   the same construction as the desktop's `ErrorToast`.
 * - **Dismissal is persistent, per device.** It goes to
 *   `localStorage` via {@link dismissOnce}, not to an Op: "I have read
 *   this" is view state and must not converge to another machine
 *   (`UX.md` principle 5).
 *
 * It mounts at each client's shell level rather than above the outline,
 * because it is a statement about the app and not about the page in
 * front of you. Mounting it per page would re-run the dismissed check
 * on every navigation and put an app-level notice inside the stack of
 * banners that report *this page's* health.
 *
 * ## What it is not
 *
 * Not a gate. The reporter is live from the first boot, before this has
 * rendered — holding events until the strip had been seen would blind
 * it to boot crashes, which is the class that motivated the RFC.
 *
 * Every word comes from `CRASH_REPORTS_NOTICE` in
 * `@outl/shared/onboarding`. A client does not phrase this: the same
 * claim goes to Apple and Google, so a second wording is how one of
 * those declarations becomes false.
 */
export function CrashReportsNotice() {
  const [hidden, setHidden] = createSignal(dismissedOnce(CRASH_NOTICE_KEY));
  const close = () => {
    dismissOnce(CRASH_NOTICE_KEY);
    setHidden(true);
  };
  return (
    <Show when={!hidden()}>
      <div class="outl-crash-notice" role="status" aria-label="Crash reports">
        <div class="outl-crash-notice__card">
          <p class="outl-crash-notice__body">{CRASH_REPORTS_NOTICE.body}</p>
          <button type="button" class="outl-crash-notice__dismiss" onClick={close}>
            {CRASH_REPORTS_NOTICE.dismiss}
          </button>
        </div>
      </div>
    </Show>
  );
}
