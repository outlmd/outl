/**
 * Rename and merge a page: the two page actions that sit beside delete.
 *
 * The flow (a taken name offers a merge, a merge is always confirmed,
 * which wording each dialog and summary uses) is `@outl/shared`'s
 * `pages/rename-merge`, shared with mobile. This file is only the
 * desktop half: `askConfirm` for the questions (the same dialog the
 * delete paths now use), the toast for the outcome, and the error
 * surface.
 *
 * The questions used to be `window.confirm`. A native dialog can take
 * no palette token and give no destructive signal, and a merge deletes
 * a page with no undo in the UI — see `lib/confirm.ts` for the full
 * argument. The **sentences are unchanged** and still come from the
 * shared module: only the surface that shows them moved.
 *
 * Both functions resolve to the page to show, or `null` when nothing
 * happened: the user declined the confirm (no write command ran) or the
 * backend refused (the error is already on the toast).
 */

import type { PageMeta, PageView } from "@outl/shared/api/types";
import {
  mergeConfirmMessage,
  mergeInto,
  mergeSummary,
  nameTakenMessage,
  renameOrMerge,
  renameSummary,
} from "@outl/shared/pages/rename-merge";

import { askConfirm } from "./confirm";
import { bumpPageList, setAppState } from "./store";

const label = (p: PageMeta) => p.title || p.slug;

function fail(e: unknown): null {
  setAppState("lastError", e instanceof Error ? e.message : String(e));
  return null;
}

/** A rename or a merge landed: the page list on screen is stale. */
function landed(view: PageView, lines: string[]): PageView {
  setAppState("notice", lines);
  bumpPageList();
  return view;
}

export async function renamePageFlow(
  page: PageMeta,
  newName: string,
): Promise<PageView | null> {
  try {
    const result = await renameOrMerge(page.slug, newName.trim(), (existing) =>
      askConfirm({
        message: nameTakenMessage(label(existing)),
        confirmLabel: "Merge",
        label: "Merge pages",
        destructive: true,
      }),
    );
    if (result.kind === "cancelled") return null;
    return landed(
      result.view,
      result.kind === "renamed"
        ? [renameSummary(result.report)]
        : mergeSummary(result.report),
    );
  } catch (e) {
    return fail(e);
  }
}

export async function mergePageFlow(
  source: PageMeta,
  target: PageMeta,
): Promise<PageView | null> {
  // Always asked: a merge deletes the source page and cannot be undone
  // from the page itself.
  const ok = await askConfirm({
    message: mergeConfirmMessage(label(source), label(target)),
    confirmLabel: "Merge",
    label: "Merge pages",
    destructive: true,
  });
  if (!ok) return null;
  try {
    const result = await mergeInto(source.slug, target.slug);
    return landed(result.view, mergeSummary(result.report));
  } catch (e) {
    return fail(e);
  }
}
