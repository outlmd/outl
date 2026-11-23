/**
 * What the one `<Toast>` is saying, and in what tone.
 *
 * The tone lives *inside* this state rather than beside it, and that is
 * the whole design. Before, the pill was unconditionally the destructive
 * fill, so "Renamed. 12 links updated." arrived in the colour a failure
 * wears — one token carrying two meanings, which is root `CLAUDE.md`
 * invariant 13. The obvious fix is a `tone` signal next to the message,
 * and it is the wrong one: there were already forty-odd `setError(…)`
 * call sites across `Journal.tsx` and its siblings, none of which would
 * set it, so the first confirmation would leave `success` behind for the
 * next error to wear.
 *
 * So there are exactly two ways to put text on the toast, and each one
 * sets both facts:
 *
 * - {@link JournalToast.setError} — a failure. Destructive pill, and the
 *   only tone a retry handler belongs to.
 * - {@link JournalToast.notify} — a confirmation. Accent pill, a success
 *   buzz, and any retry left over from a previous error is dropped,
 *   because it would retry something unrelated to the message on screen.
 *
 * `notify` owning the haptic is why every confirmation on this client
 * feels the same. `Journal.capture.ts` also buzzes on its own path (it
 * confirms a capture that does *not* reach the toast, when the user is
 * already looking at today), so the off-today path arms the pattern
 * twice in quick succession; `navigator.vibrate` replaces a running
 * pattern rather than queueing, so that is one buzz, not two.
 */

import { createSignal } from "solid-js";

import { haptic } from "../lib/haptics";
import type { ToastTone } from "./Toast";

export function createJournalToast() {
  const [message, setMessage] = createSignal<string | null>(null);
  const [tone, setTone] = createSignal<ToastTone>("error");
  // Optional retry handler tied to the most recent error. When set, the
  // toast pins (no auto-dismiss) and shows a "Retry" button. It is held
  // alongside the message so callers can offer the affordance without
  // plumbing it through every async helper.
  const [errorRetry, setErrorRetry] = createSignal<(() => void) | null>(null);

  /** A failure, or `null` to clear. Always the destructive pill. */
  const setError = (text: string | null) => {
    setTone("error");
    setMessage(text);
  };

  /** A confirmation: accent pill, success buzz, no stale retry. */
  const notify = (text: string) => {
    setErrorRetry(null);
    setTone("success");
    setMessage(text);
    haptic("success");
  };

  const dismissToast = () => {
    setMessage(null);
    setErrorRetry(null);
  };

  return {
    /** Named `error` because every existing reader spells it that way. */
    error: message,
    setError,
    errorRetry,
    setErrorRetry,
    toastTone: tone,
    notify,
    dismissToast,
  };
}

export type JournalToast = ReturnType<typeof createJournalToast>;
