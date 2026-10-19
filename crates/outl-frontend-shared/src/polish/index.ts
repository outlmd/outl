/**
 * Polish shared by both GUI clients: motion (fold, drag, sync settle),
 * the long-reading rhythm, focus mode, the first-run tips and the
 * crash-reporting strip.
 * Styles live in `@outl/shared/polish/styles`.
 */
export { createFocusMode, focusMode, FOCUS_KEY, type FocusMode } from "./focus";
export {
  isBlankOutline,
  firstRunTips,
  dismissedOnce,
  dismissOnce,
  TIPS_KEY,
  CRASH_NOTICE_KEY,
  type Tip,
  type TipClient,
} from "./firstRun";
export { FirstRunTips } from "./FirstRunTips";
export { CrashReportsNotice } from "./CrashReportsNotice";
export { contrastRatio } from "./contrast";
export { createSettle } from "./settle";
