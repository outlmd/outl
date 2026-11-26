import {
  openAsset,
  openExternalUrl,
  openPageBySlug,
  openRef,
  runAutoRunBlocks,
} from "@outl/shared/api/commands";
import type { PageView } from "@outl/shared/api/types";
import { isAssetLink } from "@outl/shared/links";

import { resetForNavigation } from "./page-change";
import { applyPageState } from "./page-view";
import { setAppState } from "./store";

/**
 * Where a click inside a rendered block goes: a ref, a tag, a link, or
 * a `call:` fence's template page.
 *
 * One copy for every surface that renders a block through
 * `<BlockBody />` — the outline and the "Linked from" rows — so a
 * `[[ref]]` cannot navigate one way on the page and another way in the
 * backlink citing it. These handlers only navigate; none of them writes
 * to the page the block lives on, which is why a backlink row can take
 * them unchanged.
 */

function reportError(e: unknown): void {
  setAppState("lastError", e instanceof Error ? e.message : String(e));
}

/**
 * Fold a reply into the page's state — `applyPageState` owns which
 * fields that is and why the ahead-of-log notice is sticky — then
 * auto-run the page's query blocks. The transclusion resolve rides the
 * outline effect, so it is not repeated here.
 */
export function applyView(view: PageView): void {
  applyPageState(view);
  void runAutoRunBlocks(view.page.id)
    .then((reply) => {
      if (reply.ran > 0) applyPageState(reply.view);
    })
    .catch(() => {});
}

/** `[[target]]` — `openRef` creates the page when nothing resolves. */
export async function handleRefClick(target: string): Promise<void> {
  let view: PageView;
  try {
    view = await openRef(target);
  } catch (e) {
    reportError(e);
    return;
  }
  // A navigation, even when the ref points back at the open page —
  // so the zoom goes, the same way `Cmd+J` drops it.
  resetForNavigation();
  applyView(view);
}

export function handleTagClick(tag: string): void {
  void handleRefClick(tag);
}

export function handleLinkClick(href: string): void {
  // A `[label](assets/…)` link opens the uploaded file in the OS
  // default app (`open_asset`); everything else is an external
  // `[label](url)` opened in the system browser (scheme-guarded to
  // http(s)/mailto). Errors land on the status line instead of
  // throwing into the click handler.
  const opening = isAssetLink(href) ? openAsset(href) : openExternalUrl(href);
  void opening.catch(reportError);
}

/** A `call:<name>` fence's template page, by exact slug. Unlike
 *  {@link handleRefClick} this never creates a page on a miss. */
export async function handleOpenPage(slug: string): Promise<void> {
  try {
    applyView(await openPageBySlug(slug));
  } catch (e) {
    reportError(e);
  }
}
