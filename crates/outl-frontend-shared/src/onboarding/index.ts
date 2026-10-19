/**
 * First-run onboarding copy — the one piece both clients render
 * identically.
 *
 * The *chrome* of onboarding is client-specific (mobile shows a
 * full-screen sequence of bottom-sheet-styled cards with haptics;
 * desktop wraps the existing `<WorkspacePicker />` + a centered card),
 * so the components stay in each client. What does NOT differ is the
 * **words** — the honest, no-account explanation of where notes live
 * and how peer-to-peer sync works. Keeping the strings here means a
 * copy edit (or a future i18n pass) lands in one place instead of
 * drifting between iOS and desktop.
 *
 * Nothing here calls `invoke()` or touches Tauri — it is plain data.
 * The client decides *how* to present it.
 */

/**
 * Headline + body for the "where do your notes live?" step.
 *
 * Mobile renders the two storage choices (`local` / `iCloud`) below
 * this copy; desktop renders the folder picker. The copy is the same
 * framing either way: your notes are files, you choose where.
 */
export const STORAGE_STEP = {
  /** Short title shown at the top of the storage step. */
  title: "Where do your notes live?",
  /**
   * One honest sentence. outl notes are plain Markdown files in a
   * folder you control — no proprietary database, no lock-in.
   */
  body: "Your notes are plain Markdown files in a folder you control. No account, no proprietary database.",
} as const;

/**
 * Headline + body + bullet points for the optional "sync with another
 * device?" step.
 *
 * The promise we must keep honest: sync is peer-to-peer, needs no
 * account and no cloud, and a single device works fine. This is the
 * step the user can always skip.
 */
export const SYNC_STEP = {
  /** Short title shown at the top of the sync step. */
  title: "Sync with another device?",
  /**
   * The core reassurance: P2P, no account, no server, optional.
   */
  body: "outl syncs directly between your devices, peer-to-peer. No account, no cloud server, nothing to sign up for.",
  /**
   * Short reassurances rendered as a bulleted list under {@link body}.
   * Each is a complete, self-contained claim we can stand behind.
   */
  bullets: [
    "One device works perfectly on its own.",
    "Pairing is end-to-end between your devices only.",
    "You can pair a device any time later from settings.",
  ],
  /** Label for the button that opens the existing pairing flow. */
  pairCta: "Pair a device",
  /** Label for the button that skips pairing and finishes onboarding. */
  skipCta: "Skip for now",
} as const;

/** Label for the final "open my notes" button that lands on today's journal. */
export const FINISH_CTA = "Start writing";

/**
 * Headline + body for the Privacy section's crash-reporting toggle
 * (RFC 0377 section 6).
 *
 * Reporting is **on by default**, so this copy is not a nicety: prior
 * disclosure is what the opt-out footing rests on, and a user whose
 * first notice is the source code has not been disclosed to.
 *
 * Two things it has to say, and both are promises rather than wording:
 * that sending a crash is a *contribution* (we cannot reproduce a
 * failure on a device we do not own), and that notes are never part of
 * it. `onboarding.test.ts` asserts on those, not on the sentences — the
 * same shape as {@link STORAGE_STEP}'s "no account" check.
 *
 * It lives here, beside the other shared copy, because the desktop and
 * mobile Settings surfaces must make the identical claim. Three of
 * outl's privacy declarations go to an app store, so a client phrasing
 * this itself is how one of them becomes a false statement.
 */
export const CRASH_REPORTS_SETTING = {
  /** Section row title. */
  title: "Crash reports",
  /**
   * What is sent, why it helps, and what is never in it. The field list
   * is exhaustive on purpose: after this feature the privacy claim is
   * conditional, and a conditional claim is only auditable if it names
   * the condition.
   */
  body:
    "When outl crashes, it sends what broke: the error, where in the code it happened, your OS and the app version. " +
    "That is how a crash on a device we do not have gets fixed. " +
    "Your notes are never included — no page titles, no block text, no file paths, and no account, because there isn't one.",
  /**
   * Shown in place of {@link CRASH_REPORTS_SETTING.body} when the client
   * could not read or write the flag.
   *
   * Here rather than in each client because both need it and the row's
   * whole job is to not claim a state it does not know. The desktop
   * first shipped without any equivalent: its checkbox rendered
   * unchecked on a failed read, which reads as "off, and locked" over a
   * reporter whose default is on.
   *
   * One sentence for both directions — a failed read and a failed write
   * end the same way, with this device unable to say what the flag is.
   */
  unavailable:
    "outl could not read or save this setting. Reopen Settings to try again.",
} as const;

/**
 * The first-run strip: the proactive half of the same disclosure
 * {@link CRASH_REPORTS_SETTING} makes on demand (RFC 0377 section 6).
 *
 * Both exist because they answer different questions. Settings is a
 * place you go to *look*; nothing routes a user through it, so a feature
 * that is on by default and only documented there is a feature the user
 * may never learn about. This strip is the one surface that arrives
 * uninvited, which is what makes prior disclosure true rather than
 * available.
 *
 * Shorter than the Settings body on purpose. A strip the user did not
 * ask for has to be readable in a glance, so it carries the two
 * promises (sending a crash is what fixes it; notes are never in it) and
 * points at Settings for the field list instead of repeating it.
 *
 * It deliberately does **not** gate the reporter. Holding events until
 * this had been shown would blind the reporter to boot crashes, which is
 * the class that motivated the whole RFC — a safeguard that excuses
 * itself exactly where the bug lives is not one.
 */
export const CRASH_REPORTS_NOTICE = {
  /**
   * Two sentences and a way out. No title: a strip with a heading reads
   * as a dialog, and this one must not.
   */
  body:
    "When outl crashes, it sends the crash to us, which is how it gets fixed. " +
    "Your notes are never part of it. You can turn this off in Settings.",
  /**
   * Label for the only button on the strip. Acknowledgement, not
   * consent — reporting was already on, and a button that implied
   * otherwise would misdescribe what the user just did.
   */
  dismiss: "Got it",
} as const;
