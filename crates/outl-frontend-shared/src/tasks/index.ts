/**
 * How a task list is sectioned: one heading per page, placed where that
 * page's best-ranked row landed.
 *
 * **Why this is shared.** Both GUI clients had their own copy of this
 * memo — `TasksPanel.tsx` on the desktop, `TasksSheet.tsx` on the phone
 * — and both carried the same defect, folding only *adjacent* rows. The
 * backend sorts started tasks ahead of every TODO, so a page holding a
 * `DOING` and a `TODO` arrives as two runs and got two headings, which
 * reads as a rendering bug. Two copies of one pure function is the drift
 * `docs/contributing.md` → Reuse-first exists to prevent, and this one
 * was caught with both copies broken identically.
 *
 * **Why not `@outl/shared/api/overview`**, where `listOpenTasks` and
 * `TaskHit` live: both clients' tests mock that module wholesale, which
 * would replace this function with `undefined` in exactly the tests that
 * render a grouped list. Same reason `EMPTY_REMINDER_NOTICE` sits apart
 * from `commands-reminders.ts`. The parameter is structural for the same
 * reason — nothing here imports a DTO, so nothing here can be mocked
 * away.
 *
 * **Nothing here ranks.** Order is `outl_tauri_shared::commands::tasks`'
 * (started first, then journals newest-first, then named pages
 * alphabetically, each page in outline order); this only collects.
 */

/** A page's rows. The caller turns `slug` into a heading. */
export interface TaskGroup<T> {
  slug: string;
  tasks: T[];
}

/**
 * Group tasks by their page, in order of first appearance.
 *
 * Each page appears once, at the position its first row held, and its
 * remaining rows follow underneath in the order they arrived. A page
 * whose rows are not contiguous therefore still gets one section, and no
 * row changes rank relative to the page it belongs to.
 */
export function groupTasksByPage<T extends { source_slug: string }>(
  tasks: readonly T[],
): TaskGroup<T>[] {
  const out: TaskGroup<T>[] = [];
  const bySlug = new Map<string, T[]>();
  for (const t of tasks) {
    const seen = bySlug.get(t.source_slug);
    if (seen) {
      seen.push(t);
      continue;
    }
    const rows = [t];
    bySlug.set(t.source_slug, rows);
    out.push({ slug: t.source_slug, tasks: rows });
  }
  return out;
}
