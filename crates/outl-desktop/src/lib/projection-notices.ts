/**
 * The two projection-failure events, and what the shell does with them.
 *
 * A projection failure is a mutation that reached the op log and could
 * not reach `pages/<slug>.md`. Root `CLAUDE.md` invariant 8 is why a
 * refusal happens at all; the reason it has to reach the user is that a
 * refusal swallowed into a log line ships a page that silently stopped
 * syncing, which is the condition the ahead-of-log banner exists for.
 *
 * Both events carry a backend-authored sentence, so nothing here writes
 * copy: the one routing decision is which surface it lands on.
 *
 * - `projection-write-failed` is the async writer's refusal. When it
 *   names the open page and classifies as ahead-of-log it becomes that
 *   page's sticky banner; otherwise it is an error toast, because an
 *   off-screen refusal froze a page just the same.
 * - `ref-projection-failed` is `open_ref`'s, and it is always a toast:
 *   the op log already holds the page, the next save or the orphan
 *   scanner retries the write, and the backend deliberately keeps
 *   ahead-of-log off this event for that reason. Transient does not mean
 *   invisible — until that retry lands the ref the user just inserted
 *   points at a page no peer can see.
 *
 * Installed once by `<AppShell />`. It lives here rather than inline in
 * the shell's `onMount` so the routing is reachable from a test without
 * mounting the whole app, the same split mobile's `Journal.listeners.ts`
 * has.
 */
import type { ProjectionWriteFailed } from "@outl/shared/api/types";

import {
  onProjectionWriteFailed,
  onRefProjectionFailed,
  type RefProjectionFailedPayload,
} from "./events";
import { appState, setAppState } from "./store";

/** Route one async projection refusal to the banner or the toast. */
export function applyProjectionWriteFailed(failure: ProjectionWriteFailed) {
  if (failure.md_ahead_of_log && appState.page?.id === failure.page_id) {
    setAppState("mdAheadOfLog", failure.md_ahead_of_log);
  } else {
    setAppState("lastError", failure.error);
  }
}

/** Route one `open_ref` projection refusal to the toast. */
export function applyRefProjectionFailed(payload: RefProjectionFailedPayload) {
  setAppState("lastError", payload.error);
}

/**
 * Subscribe both events. Returns the combined unlisten — call it on
 * shell cleanup.
 */
export async function installProjectionNotices(): Promise<() => void> {
  const unlisten = await Promise.all([
    onProjectionWriteFailed(applyProjectionWriteFailed),
    onRefProjectionFailed(applyRefProjectionFailed),
  ]);
  return () => {
    for (const off of unlisten) off();
  };
}
