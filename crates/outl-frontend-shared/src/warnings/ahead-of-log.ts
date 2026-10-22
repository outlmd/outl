/**
 * The user-facing copy for "this page stopped syncing", and the one
 * owner of it.
 *
 * Kept as a pure function rather than inline JSX because the two clients
 * must say the *same* thing about the same state, and because the piece
 * that actually matters here is the wording — a banner nobody
 * understands is the same defect as no banner at all, which is what this
 * state had before (the refusal died in a backend `tracing::warn!`).
 *
 * The only legitimate difference between clients is the recovery: the
 * desktop user has a terminal in the same folder, an iPhone user has no
 * `outl` binary at all. Inventing a mobile button that doesn't work
 * would be worse than saying so plainly.
 *
 * This file is a **declared mirror** of `outl_actions::refusal::ahead_of_log_notice`,
 * which every Rust surface (the TUI banner, the CLI, the MCP server) reads.
 * A Solid component cannot call into that crate, and shipping seven prose
 * strings over the IPC on every page open to avoid retyping two of them is
 * not a trade worth making — so the headline and the command are pinned
 * instead of trusted, by `the_shared_frontend_copy_says_the_same_thing`.
 *
 * Background: root `CLAUDE.md` invariant 8, `docs/rfcs/0210-md-content-outside-op-log.md`.
 */

import type { MdAheadOfLog } from "../api/types";

/** Which shell is asking — decides only the recovery sentence. */
export type AheadOfLogClient = "desktop" | "mobile";

/**
 * The command that brings the unlogged lines into the op log. One
 * literal, so the desktop instruction and the mobile "run it over there"
 * instruction can never name different commands.
 */
export const RECONCILE_COMMAND = "outl reconcile --ahead-of-log";

export interface AheadOfLogNotice {
  /** What is happening, in the user's terms. */
  title: string;
  /** Why, in one sentence, with no op-log vocabulary. */
  body: string;
  /** One of the at-risk lines, verbatim from the backend. */
  sample: string;
  /**
   * What to do about it, ending in a colon — the command follows on its
   * own line. Client-specific, and honest about it: a phone has no
   * terminal, so the mobile wording sends the user to a computer instead
   * of naming a command they cannot run.
   */
  action: string;
  /** The command the action sentence points at, rendered as code. */
  command: string;
  /**
   * What happens if the user keeps working here.
   *
   * **Not a warning about losing the lines.** Editing is safe: every
   * mutation goes through the op log, and `ProjectionWriter` routes its
   * write through `apply_page_md_with_sidecar_guarded`, which refuses to
   * project over unlogged content just as the open path does.
   *
   * An earlier draft of this string said the opposite — "editing will
   * overwrite those lines" — because it was written before the
   * post-mutation guard existed, in this same change. It survived into
   * review as a sentence that frightened the user about the one thing
   * the release had just made safe. Copy describing a behaviour is only
   * as current as the behaviour; re-read it when the behaviour moves.
   */
  caution: string;
  /** The file, for the user who is about to go look at it. */
  path: string;
}

export function aheadOfLogNotice(
  info: MdAheadOfLog,
  client: AheadOfLogClient,
): AheadOfLogNotice {
  const count = info.lines === 1 ? "1 line" : `${info.lines} lines`;
  const verb = info.lines === 1 ? "was" : "were";
  return {
    title: "This page isn't syncing",
    body:
      `${count} in this file ${verb} never recorded by outl, so they exist only on this device. ` +
      "Until that's fixed, changes from your other devices won't show up here either.",
    sample: info.sample,
    action:
      client === "desktop"
        ? "To record them, run this in the workspace folder:"
        : "Open this workspace on your computer and run this — there's nothing to run from the phone:",
    command: RECONCILE_COMMAND,
    caution:
      "You can keep editing — outl won't overwrite those lines, and your " +
      "changes are saved. The file just won't be rewritten until they're recorded.",
    path: info.path,
  };
}
