/**
 * Opening a page, as opposed to showing a reply about the page already
 * open.
 *
 * The two look identical from here — both end in a `PageView` reaching
 * the screen — and they are not the same act. A navigation is the user
 * asking to be somewhere else, so it throws away what was true of where
 * they were: the zoom, and a range selection whose block ids mean
 * nothing on another page. An edit commit, a background poll, a peer
 * reload and a capture into today are not that, and must leave both
 * standing.
 *
 * Its own module because `applyView` cannot answer the question. It
 * sees a `PageView` and compares slugs, which is the right net for "the
 * page changed however it happened" and is blind to the case that
 * actually bit: **asking for the page you are already on.** Today's
 * journal is usually the open page, so the Today button, the week
 * strip's current day, a `[[ref]]` pointing back at this page and a
 * deep link to it all resolved to the same slug — and the zoom they
 * were supposed to leave stayed up, with nothing on screen moving.
 */

import type { PageView } from "@outl/shared/api/types";

export interface NavigationDeps {
  /** Fold a reply into the page on screen. Never resets anything. */
  applyView: (v: PageView) => void;
  /** The zoom root (`focusBlockId` in `Journal.tsx`). */
  setFocusBlockId: (id: string | null) => void;
  /** The range selection, scoped to the page it was started on. */
  setSelection: (s: null) => void;
}

export function createNavigation(deps: NavigationDeps) {
  /** Exposed because `applyView` is the second caller: it runs this on
   *  a slug change, as the net for a navigation that did not come
   *  through `openView`. */
  function resetForNavigation() {
    deps.setFocusBlockId(null);
    deps.setSelection(null);
  }

  return {
    resetForNavigation,
    /** Show a page the user asked for. */
    openView(v: PageView) {
      // Before the view lands, so no frame renders the new page through
      // the old page's zoom.
      resetForNavigation();
      deps.applyView(v);
    },
  };
}
