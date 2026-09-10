/**
 * `@outl/shared/toolbar` — catalog, MFU ordering and the order lock for
 * the mobile edit toolbar, shared across every web-rendered client bar
 * (Android today, iOS once the native bar retires). The rendering
 * (icons, capsule, keyboard docking) stays in client chrome; only the
 * pure logic lives here.
 *
 * The lock is read by the **iOS native bar** too, which pulls it out of
 * the webview's `localStorage` rather than keeping a `UserDefaults`
 * copy — see `./lock`.
 */
export {
  DEFAULT_ORDER,
  MIDDLE_ORDER,
  PINNED_FIRST,
  PINNED_LAST,
  TOOLBAR_META,
  type ToolbarAction,
  type ToolbarActionMeta,
  type ToolbarStyle,
} from "./actions";
export {
  clearCountsInStore,
  isToolbarAction,
  MFU_STORAGE_KEY,
  orderedMiddleActions,
  orderedMiddleFromStore,
  parseCounts,
  readCountsFromStore,
  record,
  recordToStore,
  type ToolbarCounts,
} from "./mfu";
// Only what a client actually calls. The pure internals
// (`reconcileLockedOrder`, `parseLockedOrder`, `resolveMiddleOrder`,
// `LOCK_STORAGE_KEY`) stay module-level exports for the tests that
// drive them from `./lock` directly — publishing them here would
// advertise a surface nothing consumes.
export {
  isOrderLocked,
  lockOrderInStore,
  resolveMiddleFromStore,
  unlockOrderInStore,
} from "./lock";
