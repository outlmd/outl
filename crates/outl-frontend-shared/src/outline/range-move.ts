import type { BlockNode } from "../api/types";

/**
 * Which blocks a Visual range move actually moves, and in what order.
 *
 * A range is a flat DFS slice, so it can hold a parent **and** its
 * children. Moving each of those ids one by one drags the children out
 * of their parent (a child "moves up" past its own first sibling slot
 * and the tree scrambles). A move is defined on the roots of the
 * selection: ids under another selected id travel with their ancestor.
 *
 * Returns `[]` (move nothing) when the roots do not share one parent,
 * are not contiguous siblings, or the run already sits at the edge in
 * `dir`: a block never crosses its parent, and a half-applied range is
 * worse than none. Otherwise the roots ordered for the walk: top-down
 * for `up` (the first slides past the block above, the rest follow into
 * the slot it left), bottom-up for `down`.
 *
 * Only the move ops use this. `indent` / `outdent` keep walking the raw
 * slice, where a child of a selected parent is meant to be re-levelled.
 */
export function rangeMoveTargets(
  ids: readonly string[],
  blocks: readonly BlockNode[],
  dir: "up" | "down",
): string[] {
  const selected = new Set(ids);
  const roots: { id: string; parent: string | null; index: number; len: number }[] = [];

  const walk = (siblings: readonly BlockNode[], parent: string | null, covered: boolean) => {
    siblings.forEach((b, index) => {
      const mine = selected.has(b.id);
      if (mine && !covered) {
        roots.push({ id: b.id, parent, index, len: siblings.length });
      }
      walk(b.children, b.id, covered || mine);
    });
  };
  walk(blocks, null, false);

  if (roots.length === 0) return [];

  const { parent, len } = roots[0];
  if (roots.some((r) => r.parent !== parent)) return [];
  const run = [...roots].sort((a, b) => a.index - b.index);
  const contiguous = run.every((r, i) => i === 0 || r.index === run[i - 1].index + 1);
  if (!contiguous) return [];
  if (dir === "up" && run[0].index === 0) return [];
  if (dir === "down" && run[run.length - 1].index === len - 1) return [];

  const ordered = run.map((r) => r.id);
  return dir === "up" ? ordered : ordered.reverse();
}
