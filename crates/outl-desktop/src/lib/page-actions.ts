/**
 * Rename and merge a page: the two page actions that sit beside delete.
 *
 * The flow (a taken name offers a merge, a merge is always confirmed,
 * which wording each dialog and summary uses) is `@outl/shared`'s
 * `pages/rename-merge`, shared with mobile. This file is only the
 * desktop half: `window.confirm` for the questions (the same dialog the
 * delete path uses), the toast for the outcome, and the error surface.
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

import { setAppState } from "./store";

const label = (p: PageMeta) => p.title || p.slug;

function fail(e: unknown): null {
  setAppState("lastError", e instanceof Error ? e.message : String(e));
  return null;
}

export async function renamePageFlow(
  page: PageMeta,
  newName: string,
): Promise<PageView | null> {
  try {
    const result = await renameOrMerge(page.slug, newName.trim(), async (existing) =>
      window.confirm(nameTakenMessage(label(existing))),
    );
    if (result.kind === "cancelled") return null;
    setAppState(
      "notice",
      result.kind === "renamed" ? [renameSummary(result.report)] : mergeSummary(result.report),
    );
    return result.view;
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
  if (!window.confirm(mergeConfirmMessage(label(source), label(target)))) return null;
  try {
    const result = await mergeInto(source.slug, target.slug);
    setAppState("notice", mergeSummary(result.report));
    return result.view;
  } catch (e) {
    return fail(e);
  }
}
