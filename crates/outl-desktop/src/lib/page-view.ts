/**
 * The one owner of "a `PageView` landed — what does the store now say
 * about the page?".
 *
 * Four facts come off a reply and are **not** derivable from anything
 * else on screen: the page itself, its own `key:: value` properties, the
 * parser's recovery records, and whether it has stopped syncing. They
 * used to be written in one `applyView` (`<OutlineView />`) while a
 * second `applyView` (`<AppShell />`) wrote the page and the outline and
 * nothing else.
 *
 * That second one is every navigation the chrome owns — the sidebar, the
 * picker, Today, a task row, a trash row, a deep link, "Open With" — so
 * opening a page that way left the **previous** page's property chips
 * under the new title, its parse warnings above the outline, and its
 * "this page isn't syncing" banner on a page that syncs. The chips were
 * what the user saw (a `type:: person` row pinned under every journal
 * day), and the banner was the dangerous half: invariant 8's notice is
 * only useful if it names the page it is about.
 *
 * So the split is by **question**, not by caller: this answers "what is
 * on screen", and a caller adds only what is its own — cursor repair in
 * the shell, auto-run blocks and embed resolution in the outline.
 */

import type { PageView } from "@outl/shared/api/types";

import { appState, setAppState, setOutline } from "./store";

/**
 * "Is this page still not syncing?", carried across same-page refreshes
 * — but only across the replies that cannot answer.
 *
 * Only the open commands attempt the re-projection that discovers the
 * condition; a mutation reply is built from the tree and never carries
 * the flag. Reading it straight off every view would therefore clear the
 * banner on the user's first edit — the exact action the banner warns
 * against, since a local edit re-projects the page and overwrites the
 * unlogged lines.
 *
 * `md_ahead_of_log_checked` is what separates the two: a reply that ran
 * the check is authoritative in **both** directions, so an absent notice
 * means the page is healthy again (the user ran
 * `outl reconcile --ahead-of-log`) and the banner clears. Sticking past
 * that would leave a permanent "this page isn't syncing" on a page that
 * syncs — the mirror of the silence the banner exists to end, and a
 * banner users learn to ignore.
 */
function stickyAheadOfLog(view: PageView): PageView["md_ahead_of_log"] {
  if (view.md_ahead_of_log_checked) return view.md_ahead_of_log;
  if (view.md_ahead_of_log) return view.md_ahead_of_log;
  return appState.page?.id === view.page.id ? appState.mdAheadOfLog : undefined;
}

/**
 * Fold a reply into the page's store state.
 *
 * Reads `appState.page` before writing it (`stickyAheadOfLog` needs the
 * page that was on screen), so the order of the writes below is
 * load-bearing.
 */
export function applyPageState(view: PageView): void {
  if (view.projection_error) setAppState("lastError", view.projection_error);
  setAppState({
    page: view.page,
    parseWarnings: view.warnings ?? [],
    mdAheadOfLog: stickyAheadOfLog(view),
    pageProperties: view.page_properties ?? [],
  });
  // Reconcile the outline (see `setOutline`): only the block that
  // actually changed re-renders, not all N rows.
  setOutline(view.outline);
}
