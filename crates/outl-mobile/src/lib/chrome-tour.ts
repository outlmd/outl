/**
 * What the phone's buttons do, in the words a new user needs
 * ([#574](https://github.com/outlmd/outl/issues/574)).
 *
 * The existing first-run tour (`firstRunTips`, `@outl/shared/polish`)
 * teaches the **outline**: every line is a block, `[[` links a page,
 * Today brings you back. It is right and it is not this. It never
 * names the bottom bar, the week strip, the calendar, the `+`, or the
 * long-press — so after it, the only way to learn the chrome is to tap
 * it and see, which on a phone is a lot of unmarked surface.
 *
 * **Mobile-only, deliberately.** Every row below names a control that
 * exists on this client and nowhere else; a desktop user has a
 * sidebar, a menu bar and chords. The shared half is the "show it
 * once, then remember" mechanism (`CHROME_TOUR_KEY`), not the copy.
 *
 * Kept as data, apart from the sheet that draws it, so the wording is
 * reviewable on its own and a test can assert on it without rendering.
 */

export interface ChromeTourRow {
  /** Stable id, so a row can be pinned by test rather than by position. */
  id: string;
  /** One or two words, what the control is called on screen. */
  title: string;
  body: string;
}

/**
 * The rows, in the order a thumb meets them: the bar along the bottom
 * first, then the dates at the top, then the gestures.
 *
 * Short on purpose. This is not a product tour and it stands between
 * the user and their first bullet, so every row has to earn its line —
 * `UX.md`'s voice rules apply to it as much as to a nudge.
 */
export const CHROME_TOUR: readonly ChromeTourRow[] = [
  {
    id: "journal",
    title: "Journal",
    body: "Today's page. Swipe left or right to move a day.",
  },
  {
    id: "search",
    title: "Search",
    body: "Pages, tasks and text. The + in there starts a page that isn't a daily note.",
  },
  {
    id: "tasks",
    title: "Tasks",
    body: "Everything still open, across every page.",
  },
  {
    id: "reminders",
    title: "Reminders",
    body: "What's scheduled to buzz, and when. Snooze or tick one off from the row.",
  },
  {
    id: "plus",
    title: "The + button",
    body: "Writes a line into today without leaving the page you're on.",
  },
  {
    id: "dates",
    title: "The dates up top",
    body: "Tap a day to open it. Tap the month to get the full calendar.",
  },
  {
    id: "long-press",
    title: "Press and hold a block",
    body: "Remind me, properties, copy, delete, and the rest of what one block can do.",
  },
] as const;
