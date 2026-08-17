/**
 * Outline helpers shared by every client — pure functions over
 * `BlockNode[]` (no reactive setup, no invoke), so they're cheap to
 * call from action handlers and render memos alike.
 *
 * Two flatten families coexist here on purpose:
 *
 * - [`flattenNodes`] returns the **`BlockNode` objects** in DFS
 *   preorder — use it when you need the blocks themselves (text,
 *   todo state, children).
 * - [`flattenAll`] / [`flattenVisible`] / [`flattenParents`] return
 *   **ids only** — the selection-navigation walks (vim `j`/`k`,
 *   `zR`/`zM`, Visual ranges) operate on id lists.
 */
import { QUOTE_PREFIX } from "../markdown/quote";
import type { BlockNode, InlineToken } from "../api/types";

/**
 * Reconstruct the wire-format text of a block (TODO/DONE prefix
 * reattached). The API splits the TODO state out of `text` so the
 * checkbox can render it separately, but the editor needs the user to
 * see and be able to erase the prefix — otherwise dropping a TODO from
 * the editor is impossible.
 *
 * Mirror of `outl_actions::split_todo` in reverse. Keep in sync with
 * `outl_actions::TODO_PREFIX` / `DONE_PREFIX`.
 */
export function rawTextWithTodo(block: BlockNode): string {
  if (!block.todo) return block.text;
  return `${block.todo} ${block.text}`;
}

/**
 * Return the embed handle when `tokens` is a **single** `!((blk-X))`
 * embed token surrounded only by whitespace `plain` tokens; `null`
 * otherwise. A block that is *only* an embed triggers the read-only
 * subtree expansion (`<EmbeddedSubtree />`); mixed prose keeps the
 * inline `↳ text` render, so `null` is returned for it.
 *
 * Mirror of `outl-tui`'s `embed_only_handle` (view/outline.rs): there
 * it tokenizes the trimmed text and accepts a lone `Embed`, skipping
 * whitespace-only `Plain` runs; a `blockref` (`((…))` without the `!`),
 * a second embed, or any other token disqualifies the block.
 */
export function embedOnlyHandle(tokens: InlineToken[]): string | null {
  let handle: string | null = null;
  for (const tok of tokens) {
    if (tok.kind === "plain" && tok.value.trim() === "") continue;
    if (tok.kind === "embed" && handle === null) {
      handle = tok.value;
      continue;
    }
    return null;
  }
  return handle;
}

/**
 * Collect every distinct block-ref handle (`blk-XXXXXX`) reachable in
 * an outline — both `((…))` inline refs (`blockref`) and `!((…))`
 * embeds (`embed`) — so a client can batch-resolve them in one
 * `resolveEmbeds` round-trip before rendering.
 *
 * DFS over `children`; within each block it also descends the `inner`
 * span of `bold` / `italic` / `strike` tokens (a ref can live inside
 * `**bold**`). Handles are de-duplicated by first appearance (`Set`
 * insertion order), so the caller can slice / display them stably.
 */
export function collectBlockRefHandles(outline: BlockNode[]): string[] {
  const handles = new Set<string>();
  const walkTokens = (tokens: InlineToken[]) => {
    for (const tok of tokens) {
      switch (tok.kind) {
        case "blockref":
        case "embed":
          if (tok.value) handles.add(tok.value);
          break;
        case "bold":
        case "italic":
        case "strike":
        case "highlight":
          walkTokens(tok.inner);
          break;
      }
    }
  };
  const walkNodes = (nodes: BlockNode[]) => {
    for (const node of nodes) {
      walkTokens(node.tokens);
      walkNodes(node.children);
    }
  };
  walkNodes(outline);
  return [...handles];
}

/** Locate a block by id anywhere in a (possibly nested) outline. */
export function findBlock(blocks: BlockNode[], id: string): BlockNode | null {
  for (const b of blocks) {
    if (b.id === id) return b;
    const child = findBlock(b.children, id);
    if (child) return child;
  }
  return null;
}

/** One hop in a zoom/focus breadcrumb — an ancestor block's id and its
 *  plain text (TODO/DONE prefix already split off). */
export interface FocusCrumb {
  id: string;
  text: string;
}

/** Result of zooming into a block: the subtree to render as the new
 *  root, plus the breadcrumb of ancestors — page-top first, the focused
 *  block's immediate parent last (empty when the block is already
 *  top-level). */
export interface FocusView {
  root: BlockNode;
  breadcrumb: FocusCrumb[];
}

/**
 * Locate `blockId` in the outline and return its subtree + breadcrumb
 * for zoom/focus, or `null` when the id isn't present — a stale zoom
 * target (block deleted or moved to another page). The caller drops the
 * zoom and renders the full page.
 *
 * Pure view logic: zoom is local per device and never round-trips to
 * the backend (the client already holds the whole `outline`). The one
 * owner of the zoom walk, shared by desktop + mobile — one owner, every
 * client wraps.
 */
export function focusSubtree(
  blocks: BlockNode[],
  blockId: string,
): FocusView | null {
  const breadcrumb: FocusCrumb[] = [];
  const find = (nodes: BlockNode[]): BlockNode | null => {
    for (const node of nodes) {
      if (node.id === blockId) return node;
      breadcrumb.push({ id: node.id, text: node.text });
      const hit = find(node.children);
      if (hit) return hit;
      breadcrumb.pop();
    }
    return null;
  };
  const root = find(blocks);
  return root ? { root, breadcrumb } : null;
}

/**
 * Do two crumb trails name the same chain of blocks (by id, in order)?
 *
 * Drives breadcrumb collapse in the backlinks panel: consecutive
 * references inside the same branch share an identical ancestor trail,
 * so a client renders the branch header once and lets the following
 * references sit under it silently instead of repeating it per line.
 * Collapse is all-or-nothing on equality (not a shared-prefix count):
 * showing only the differing tail of a trail — `Riscos` without the
 * `Planejamento Q3 ›` it hangs off — reads as ambiguous in the inline
 * breadcrumb, so any change re-shows the whole trail.
 *
 * Generic over `{ id }`, so it works for both `BacklinkCrumb` and
 * `FocusCrumb`.
 */
export function sameCrumbTrail(
  a: readonly { id: string }[],
  b: readonly { id: string }[],
): boolean {
  if (a.length !== b.length) return false;
  return a.every((crumb, i) => crumb.id === b[i].id);
}

/**
 * Flatten an outline into a DFS-preorder list of **`BlockNode`s**
 * (fold state ignored — every block is included).
 *
 * Complement of [`flattenAll`], which walks the same order but
 * returns **ids**. Reach for this one when the caller needs block
 * content, for that one when it only needs identity.
 */
export function flattenNodes(blocks: BlockNode[]): BlockNode[] {
  const out: BlockNode[] = [];
  for (const b of blocks) {
    out.push(b);
    out.push(...flattenNodes(b.children));
  }
  return out;
}

/** Total number of descendants under a block (recursive count of its
 *  whole subtree, *excluding* the block itself). Used to warn the
 *  user before a destructive delete. */
export function countDescendants(block: BlockNode): number {
  let n = 0;
  for (const child of block.children) {
    n += 1 + countDescendants(child);
  }
  return n;
}

/**
 * IDs of every block visible in the outline, in flat DFS order
 * (parent before children, siblings in array order). Collapsed
 * subtrees are skipped — `j` after a collapsed block lands on its
 * next sibling, not on its hidden child.
 */
export function flattenVisible(blocks: BlockNode[]): string[] {
  const out: string[] = [];
  const walk = (bs: BlockNode[]) => {
    for (const b of bs) {
      out.push(b.id);
      if (!b.collapsed && b.children.length > 0) walk(b.children);
    }
  };
  walk(blocks);
  return out;
}

/**
 * IDs of **every** block on the page, in flat DFS order, including
 * blocks hidden under collapsed parents. Use this when an operation
 * needs to act on the whole outline regardless of the fold state
 * (vim `zR` / `zM`, full-page reindex, "select all", …).
 *
 * Distinct from [`flattenVisible`] precisely because `zR` must
 * expand subtrees that are currently hidden — the visible-only walk
 * would silently no-op on every descendant of a collapsed parent.
 * Id-returning complement of [`flattenNodes`].
 */
export function flattenAll(blocks: BlockNode[]): string[] {
  const out: string[] = [];
  const walk = (bs: BlockNode[]) => {
    for (const b of bs) {
      out.push(b.id);
      if (b.children.length > 0) walk(b.children);
    }
  };
  walk(blocks);
  return out;
}

/**
 * IDs that `zM` (fold-all) should target. Walks the full outline in
 * DFS order **but skips leaves**: folding a leaf is invisible today
 * yet still writes `Op::SetCollapsed(true)` to the log, so when the
 * user later adds children underneath they appear collapsed — real
 * future-surprise. By contrast, `zR` (unfold-all) wants every id
 * (use [`flattenAll`] for that) — descolapsar leaf é no-op futuro.
 *
 * Mirror of `outl-tui`'s `collect_collapse_candidates` so a 200-block
 * page fires the same op count on every client.
 */
export function flattenParents(blocks: BlockNode[]): string[] {
  const out: string[] = [];
  const walk = (bs: BlockNode[]) => {
    for (const b of bs) {
      if (b.children.length > 0) out.push(b.id);
      if (b.children.length > 0) walk(b.children);
    }
  };
  walk(blocks);
  return out;
}

/**
 * Return the next visible id after `current`, or `current` itself
 * when already at the bottom (clamps; no wrap). When `current` is
 * `null` or not in the outline, returns the first visible id.
 */
export function nextVisibleId(
  current: string | null,
  blocks: BlockNode[],
): string | null {
  const ids = flattenVisible(blocks);
  if (ids.length === 0) return null;
  if (!current) return ids[0];
  const idx = ids.indexOf(current);
  if (idx === -1) return ids[0];
  return ids[Math.min(idx + 1, ids.length - 1)];
}

/** Previous visible id; returns `null` at the top (no clamp — see below). */
export function previousVisibleId(
  current: string | null,
  blocks: BlockNode[],
): string | null {
  const ids = flattenVisible(blocks);
  if (ids.length === 0) return null;
  if (!current) return ids[0];
  const idx = ids.indexOf(current);
  if (idx === -1) return ids[0];
  // No previous block when `current` is the first (or only) one — return
  // null, never `current` itself. The old `Math.max(idx - 1, 0)` →
  // `ids[0]` left the cursor on the very block the caller was about to
  // delete: selection then anchored a *trashed* node, and `o` /
  // new-block created the next block under the trash root (a deleted
  // node still satisfies `tree.contains`), corrupting the page. Callers
  // already handle null (delete clears the cursor; the page-change
  // effect re-snaps to the first block).
  return idx > 0 ? ids[idx - 1] : null;
}

/**
 * Resolve a Visual range to the inclusive `[lo, hi]` pair of ids in
 * DFS visible order. Caller passes anchor + cursor; this function
 * orders them so the rest of the codebase doesn't have to care about
 * direction (anchor below cursor or above).
 *
 * Returns `null` when either endpoint isn't visible (e.g. collapsed
 * subtree shifted the outline between Visual entry and the current
 * read). Caller should treat that as "no range".
 */
export function visualRangeIds(
  anchor: string | null,
  cursor: string | null,
  blocks: BlockNode[],
): { lo: string; hi: string } | null {
  if (!anchor || !cursor) return null;
  const ids = flattenVisible(blocks);
  const a = ids.indexOf(anchor);
  const c = ids.indexOf(cursor);
  if (a === -1 || c === -1) return null;
  const lo = Math.min(a, c);
  const hi = Math.max(a, c);
  return { lo: ids[lo], hi: ids[hi] };
}

/**
 * Build the **set of block ids inside the Visual range** in one DFS
 * walk. The parent component (`<OutlineView />`) memoises the result
 * inside a `createMemo`; every `<BlockRow />` then answers membership
 * in O(1) via `Set.has(id)`.
 *
 * Why a Set lives at the parent and not inside each row: the earlier
 * implementation called `isInVisualRange(id, anchor, cursor, outline)`
 * per row, and each call rebuilt `flattenVisible(blocks)` from scratch
 * (a full DFS). With N visible blocks that's O(N²) per Visual extension
 * keystroke — on a 500-block page that's measurable lag. The memo
 * pushes the DFS up to one walk per outline/anchor/cursor change.
 *
 * Returns `null` when anchor or cursor is unset, or when either id
 * isn't in the visible outline (collapsed subtree shifted between
 * entry and read). `null` is the caller's signal for "no range".
 */
export function visualRangeSet(
  anchor: string | null,
  cursor: string | null,
  blocks: BlockNode[],
): Set<string> | null {
  if (!anchor || !cursor) return null;
  const ids = flattenVisible(blocks);
  const a = ids.indexOf(anchor);
  const c = ids.indexOf(cursor);
  if (a === -1 || c === -1) return null;
  const lo = Math.min(a, c);
  const hi = Math.max(a, c);
  return new Set(ids.slice(lo, hi + 1));
}

/**
 * Predicate variant of [`visualRangeIds`]. Kept for the outline test
 * suite and any caller that needs an ad-hoc membership check without
 * paying for the Set allocation. **In render paths, prefer
 * [`visualRangeSet`] hoisted to a parent `createMemo`** — calling
 * this per row is O(N²) by construction.
 */
export function isInVisualRange(
  id: string,
  anchor: string | null,
  cursor: string | null,
  blocks: BlockNode[],
): boolean {
  const set = visualRangeSet(anchor, cursor, blocks);
  return set !== null && set.has(id);
}

/**
 * Keep in sync with `outl_actions::TODO_PREFIX` / `DOING_PREFIX` /
 * `DONE_PREFIX`. The order is the cycle order.
 */
const TASK_PREFIXES = ["TODO ", "DOING ", "DONE "] as const;

/**
 * Every prefix a block's text may carry, mapped to the index it
 * occupies in {@link TASK_PREFIXES}. Mirrors
 * `outl_actions::READ_PREFIXES`.
 *
 * Reading includes the CommonMark checkbox spellings; writing goes
 * through {@link TASK_PREFIXES} alone, so `"[ ] foo"` becomes
 * `"DOING foo"` on its first toggle. Miss a spelling here and the
 * toggle reads the block as unmarked and emits `"TODO [ ] foo"` — two
 * markers, and the backend's `split_todo` reads the wrong one.
 */
const READ_PREFIXES: ReadonlyArray<readonly [string, number]> = [
  ["TODO ", 0],
  ["DOING ", 1],
  ["DONE ", 2],
  ["[ ] ", 0],
  ["[/] ", 1],
  ["[x] ", 2],
  ["[X] ", 2],
];

/**
 * Cycle a block's task prefix: none → `TODO ` → `DOING ` → `DONE `
 * → none.
 *
 * Mirrors `outl_actions::todo::cycle_todo`, quote handling included:
 * both prefixes are peeled and re-emitted in canonical order
 * (`TODO > body`), so cycling a quoted block can't produce
 * `TODO > TODO body`.
 *
 * Exists because a GUI client editing a block holds the text in a
 * textarea draft, and round-tripping the toggle through the backend
 * leaves that draft stale: the block only showed its new state after
 * leaving Insert. The clients splice this into the draft instead, the
 * same way the TUI mutates its `EditBuffer` on `Ctrl+T`.
 */
export function cycleTodo(raw: string): string {
  const quoted = raw.startsWith(QUOTE_PREFIX);
  const afterQuote = quoted ? raw.slice(QUOTE_PREFIX.length) : raw;

  const hit = READ_PREFIXES.find(([p]) => afterQuote.startsWith(p));
  const current = hit ? hit[1] : -1;
  const body = hit ? afterQuote.slice(hit[0].length) : afterQuote;

  // -1 (no marker) steps to index 0; the last one steps off the end
  // back to no marker.
  const next = TASK_PREFIXES[current + 1] ?? "";
  return `${next}${quoted ? QUOTE_PREFIX : ""}${body}`;
}
