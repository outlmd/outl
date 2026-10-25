/**
 * What a search box lists for a query, and in what order. Pure, so the
 * ranking is testable without a DOM or a backend.
 *
 * The desktop palette and the phone's search sheet used to each decide
 * this on their own, which is how one client came to list things the
 * other could not find. This module is the one owner; each client keeps
 * only what it cannot share (the desktop builds action rows from chords,
 * the phone has none).
 *
 * One box finds four kinds of thing, in this order:
 *
 * - **pages**, ranked by the backend's `search_pages` (exact, prefix,
 *   substring), so the box and `[[` autocomplete agree;
 * - **tasks**, the `search_blocks` hits that carry a task marker, read
 *   with the same `taskState` the bullet toggle uses, listed before
 *   plain blocks because "find that todo" is the commonest reason to
 *   search;
 * - **blocks**, the rest of the hits, once the query is long enough to
 *   mean something (a single letter matches half the workspace);
 * - **actions**, matched on label and keywords.
 *
 * A leading `>` narrows the list to actions, the convention most
 * editors use.
 */
import type { BlockHit } from "../api/commands";
import type { PageMeta } from "../api/types";
import { taskState } from "../outline";

export const COMMAND_PREFIX = ">";
/** Blocks start showing at this many characters. */
export const BLOCK_QUERY_MIN = 2;

export type PaletteSection = "pages" | "tasks" | "blocks" | "actions";
export interface PaletteLimits {
  pages: number;
  tasks: number;
  blocks: number;
  actions: number;
}
/** Not exported: a caller overrides what it cares about through
 *  `limits`, which is a `Partial`, so nothing needs to read the rest. */
const DEFAULT_LIMITS: PaletteLimits = { pages: 8, tasks: 5, blocks: 6, actions: 5 };

/** `C` is the client's chord type; the phone has none and leaves it `never`. */
export type PaletteActionRow<C = never> = {
  kind: "action";
  action: string;
  label: string;
  chord: C[] | null;
  keywords?: string;
};

export type PaletteItem<C = never> =
  | { kind: "page"; page: PageMeta }
  | { kind: "block"; hit: BlockHit }
  | { kind: "task"; hit: BlockHit; state: "todo" | "doing" | "done" }
  | PaletteActionRow<C>;

export interface PaletteResults<C = never> {
  items: PaletteItem<C>[];
  /** Per section, how many matches the limit dropped. */
  hidden: Record<PaletteSection, number>;
}

function matchesAction(row: PaletteActionRow<unknown>, q: string): boolean {
  return `${row.label} ${row.keywords ?? ""}`.toLowerCase().includes(q);
}

export function paletteResults<C = never>(input: {
  query: string;
  pages: PageMeta[];
  blocks: BlockHit[];
  actions: PaletteActionRow<C>[];
  limits?: Partial<PaletteLimits>;
}): PaletteResults<C> {
  const limits = { ...DEFAULT_LIMITS, ...input.limits };
  const hidden: Record<PaletteSection, number> = { pages: 0, tasks: 0, blocks: 0, actions: 0 };
  const raw = input.query.trim();

  const pickActions = (q: string): PaletteActionRow<C>[] => {
    const found = input.actions.filter((a) => matchesAction(a, q));
    hidden.actions = Math.max(0, found.length - limits.actions);
    return found.slice(0, limits.actions);
  };

  if (raw.startsWith(COMMAND_PREFIX)) {
    const q = raw.slice(COMMAND_PREFIX.length).trim().toLowerCase();
    return { items: pickActions(q), hidden };
  }

  const q = raw.toLowerCase();
  const pages: PaletteItem<C>[] = input.pages
    .slice(0, limits.pages)
    .map((page) => ({ kind: "page", page }));
  hidden.pages = Math.max(0, input.pages.length - pages.length);
  if (!q) return { items: pages, hidden };

  const tasks: PaletteItem<C>[] = [];
  const blocks: PaletteItem<C>[] = [];
  if (q.length >= BLOCK_QUERY_MIN) {
    for (const hit of input.blocks) {
      const state = taskState(hit.text);
      if (state) {
        if (tasks.length < limits.tasks) tasks.push({ kind: "task", hit, state });
        else hidden.tasks++;
      } else if (blocks.length < limits.blocks) {
        blocks.push({ kind: "block", hit });
      } else {
        hidden.blocks++;
      }
    }
  }
  const actions = pickActions(q);
  return { items: [...pages, ...tasks, ...blocks, ...actions], hidden };
}

/** Section heading for an item, used to draw a label when it changes. */
export function sectionOf(item: PaletteItem<unknown>): string {
  const names = { page: "Pages", task: "Tasks", block: "Blocks", action: "Actions" } as const;
  return names[item.kind];
}
