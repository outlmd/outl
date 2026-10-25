/**
 * What mobile's bottom bar and week strip read: the workspace's open
 * tasks (the first 200, with the uncapped `total` beside them), and
 * which journal days hold text. Mirrors
 * `outl_tauri_shared::commands::{tasks, journal_days}`.
 */

import { invoke } from "@tauri-apps/api/core";

/** One open task. Mirrors `outl_tauri_shared::commands::tasks::TaskHit`. */
export interface TaskHit {
  /** Node id, to zoom into the block once its page is open. */
  id: string;
  source_slug: string;
  status: "todo" | "doing";
  /** Block text with the task marker stripped, on one line. */
  text: string;
}

/** Mirrors `outl_tauri_shared::commands::tasks::OpenTasks`. */
export interface OpenTasks {
  /** Every open task in the workspace, before the cap. */
  total: number;
  /** Started tasks first, then the latest journal days, then pages. */
  tasks: TaskHit[];
}

export function listOpenTasks(): Promise<OpenTasks> {
  return invoke<OpenTasks>("list_open_tasks");
}

/** Journal slugs in `from..=to` with at least one non-empty block, ascending. */
export function journalDaysWithContent(from: string, to: string): Promise<string[]> {
  return invoke<string[]>("journal_days_with_content", { from, to });
}
