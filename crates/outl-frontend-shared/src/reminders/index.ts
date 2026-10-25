/**
 * The wording both GUI clients use for a reminder row that has nothing
 * written on it.
 *
 * Kept apart from `commands-reminders.ts` (which wraps `invoke`) so it is
 * pure copy, and so a test that mocks the commands module still sees the
 * real sentence.
 *
 * What produces such a row is a block that is alive on a live page with no
 * body of its own, e.g. `remind:: 3pm` on an empty bullet, or a bare `TODO`
 * marker. A *deleted* block never gets here: `outl_actions::reminders`
 * drops anything with no enclosing page. So this sentence must not say the
 * block is gone.
 */
export const EMPTY_REMINDER_NOTICE =
  "This reminder's block has no text yet. Write something in it and it will show up here.";

/** Whether a reminder has anything to show as its row title. */
export function reminderHasText(r: { text: string }): boolean {
  return r.text.trim() !== "";
}
