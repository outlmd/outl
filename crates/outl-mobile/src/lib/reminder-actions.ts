/**
 * Which call a reminder banner's buttons and taps turn into.
 *
 * The decision table lives apart from the listener wiring in
 * `reminder-notifications.ts` so this half can be tested; that half
 * only runs on a device. Ids are never spelled here — they arrive
 * from the Rust catalog that stamps them onto the banner.
 *
 * Why any of this is mobile-only, and who owns the ids:
 * `crates/outl-mobile/CLAUDE.md` → Reminders → Actionable banners.
 */

/**
 * The part of the plugin's payload this module reads.
 *
 * Everything is optional because the Rust side does not type what
 * crosses the IPC boundary: an unexpected shape has to degrade to
 * "ignore it" rather than throw inside an event handler.
 */
export interface ReminderActionPayload {
  actionId?: string | null;
  extra?: { blockId?: string; pageSlug?: string } | null;
}

/** One banner button, as the Rust catalog declares it. */
export interface ReminderButton {
  id: string;
  kind: "snooze" | "done";
}

/** What the dispatcher needs from the app, injected so it can be tested. */
export interface ReminderActionDeps {
  /** The catalog's buttons, so dispatch keys on the declared `kind`. */
  buttons: readonly ReminderButton[];
  snoozeReminder(blockId: string, preset: string): Promise<void>;
  openPageBySlug(slug: string): Promise<{ page: { id: string } }>;
  markBlockDone(pageId: string, blockId: string): Promise<unknown>;
  /** Show the page the reminder is on, scrolled to its block. */
  navigateToBlock(slug: string, blockId: string): Promise<void>;
  onError(message: string): void;
}

/** What the dispatcher decided to do, so callers (and tests) can assert on it. */
export type ReminderActionOutcome = "snoozed" | "done" | "navigated" | "ignored";

/**
 * The banner has room for one snooze, and an hour is what the button
 * is for: the reminder landed at a bad moment. Longer snoozes are a
 * decision, and those belong in the sheet where every preset is shown.
 */
const BANNER_SNOOZE_PRESET = "1h";

/**
 * Act on a reminder banner.
 *
 * Returns what it did rather than throwing: this runs inside an OS
 * event callback, so an exception has nowhere to go, and a banner that
 * fails silently is exactly the dead end this whole feature exists to
 * remove. Failures reach the user through `onError`.
 */
export async function handleReminderAction(
  payload: ReminderActionPayload,
  deps: ReminderActionDeps,
): Promise<ReminderActionOutcome> {
  const blockId = payload.extra?.blockId;
  const pageSlug = payload.extra?.pageSlug;
  // A banner with no subject is not actionable. It can still happen:
  // a notification from an older build, delivered after an upgrade,
  // carries no extras.
  if (!blockId || !pageSlug) return "ignored";

  const action = payload.actionId ?? "";

  // No match means a plain tap: iOS sends its default identifier,
  // Android sends nothing, and neither is a button.
  const kind = deps.buttons.find((b) => b.id === action)?.kind;

  try {
    if (kind === "snooze") {
      await deps.snoozeReminder(blockId, BANNER_SNOOZE_PRESET);
      return "snoozed";
    }

    if (kind === "done") {
      // The block's own page, not the open one: the banner can arrive
      // while the user is looking at anything. Same resolution the
      // reminders sheet does, for the same reason.
      const target = await deps.openPageBySlug(pageSlug);
      await deps.markBlockDone(target.page.id, blockId);
      return "done";
    }

    await deps.navigateToBlock(pageSlug, blockId);
    return "navigated";
  } catch (e) {
    deps.onError(e instanceof Error ? e.message : String(e));
    return "ignored";
  }
}
