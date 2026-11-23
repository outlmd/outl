/**
 * The rename-or-merge flow and its wording, shared by both GUI clients.
 *
 * Renaming onto a name another page already owns is not an error, it is
 * a question: merge this page into that one? The backend answers it as a
 * typed `exists` reply and writes nothing, so asking is free. The
 * confirmation itself is the client's own chrome (a dialog on the
 * desktop, a sheet on the phone), passed in as `confirmMerge`; this
 * module owns the order of events and the sentences, so neither client
 * invents its own.
 *
 * Merging is not undoable in the UI, which is why a merge is never
 * reached without a confirmation: `renameOrMerge` asks, and a caller of
 * {@link mergeInto} has already asked.
 */

import { mergePages, renamePage } from "../api/page-merge";
import type { MergePagesReport, RenamePageReport } from "../api/page-merge";
import type { PageMeta, PageView } from "../api/types";

export type RenameMergeResult =
  | { kind: "renamed"; view: PageView; report: RenamePageReport }
  | { kind: "merged"; view: PageView; report: MergePagesReport }
  | { kind: "cancelled" };

/**
 * Rename `page` to `newName`. When the name is taken, ask
 * `confirmMerge(existing)` and merge this page into the existing one on
 * accept. A refusal writes nothing and resolves to `cancelled`. Errors
 * from the backend propagate as rejections.
 */
export async function renameOrMerge(
  page: string,
  newName: string,
  confirmMerge: (existing: PageMeta) => Promise<boolean>,
): Promise<RenameMergeResult> {
  const reply = await renamePage(page, newName);
  if (reply.status === "renamed") {
    return { kind: "renamed", view: reply.view, report: reply.report };
  }
  if (!(await confirmMerge(reply.existing))) {
    return { kind: "cancelled" };
  }
  return mergeInto(page, reply.existing.slug);
}

/** Merge after the caller already confirmed. */
export async function mergeInto(
  source: string,
  target: string,
): Promise<Extract<RenameMergeResult, { kind: "merged" }>> {
  const reply = await mergePages(source, target);
  return { kind: "merged", view: reply.view, report: reply.report };
}

/** The confirmation shown before every merge, whichever way it was started. */
export function mergeConfirmMessage(sourceTitle: string, targetTitle: string): string {
  return `Merge "${sourceTitle}" into "${targetTitle}"? Its blocks move to "${targetTitle}", links to it are rewritten and the page is deleted. This cannot be undone here.`;
}

/** The question asked when a rename lands on a name that is taken. */
export function nameTakenMessage(existingTitle: string): string {
  return `A page named "${existingTitle}" already exists. Merge this page into "${existingTitle}"?`;
}

const plural = (n: number, one: string, many: string): string => `${n} ${n === 1 ? one : many}`;

/**
 * A declared mirror of `outl_actions::page_summary`, pinned from the
 * Rust side by `the_shared_frontend_copy_counts_the_same_things`.
 *
 * `rewritten_blocks` counts **blocks whose text was edited**, and one
 * block can carry several mentions of the page — so it is worded as a
 * count of blocks. It used to read `3 links updated` here while the
 * terminal read `links updated in 3 blocks`, which was not a denser
 * phrasing of the same thing but a different, larger-sounding number.
 *
 * A mention left alone names a **nested page** (`[[old/child]]`), the
 * word `UX.md`'s glossary uses. The field keeps the implementation's
 * `namespace`; the user reads the glossary's.
 */
const linksUpdated = (n: number): string => `Links updated in ${plural(n, "block", "blocks")}`;

const skippedRefs = (n: number): string =>
  `${plural(n, "nested-page link", "nested-page links")} left unchanged`;

/** One sentence for the toast after a rename. */
export function renameSummary(report: RenamePageReport): string {
  const parts = [`Renamed. ${linksUpdated(report.rewritten_blocks)}.`];
  if (report.skipped_namespace_refs > 0) {
    parts.push(`${skippedRefs(report.skipped_namespace_refs)}.`);
  }
  return parts.join(" ");
}

/**
 * The lines shown after a merge: blocks moved, links updated, the alias
 * note, one line per property conflict and the nested-page links left
 * unchanged.
 * The alias and skipped-links lines appear only when there is something
 * to say. A retry that found nothing left to move says so instead of
 * reporting zero blocks.
 */
export function mergeSummary(report: MergePagesReport): string[] {
  const lines = [
    report.already_merged
      ? `Already merged into "${report.target_title}".`
      : `${plural(report.moved_blocks, "block", "blocks")} moved to "${report.target_title}".`,
    `${linksUpdated(report.rewritten_blocks)}.`,
  ];
  if (report.aliases_added.length > 0) {
    const names = report.aliases_added.map((a) => `"${a}"`).join(", ");
    lines.push(`${plural(report.aliases_added.length, "alias", "aliases")} added: ${names}.`);
  }
  for (const c of report.property_conflicts) {
    lines.push(`Property conflict: kept "${c.kept}", dropped "${c.dropped}" (${c.key}).`);
  }
  if (report.skipped_namespace_refs > 0) {
    lines.push(`${skippedRefs(report.skipped_namespace_refs)}.`);
  }
  return lines;
}
