/**
 * Keystrokes aimed at a block on **another** page: the one a focused
 * backlink row shows.
 *
 * That is a different answer to "what does this keystroke change" than
 * `block-handlers.ts` gives, which is why it is its own file along the
 * axis `docs/desktop-keybindings.md` → "`action-handlers.ts` is eight
 * files" describes. It is **not** a spread group
 * in `action-handlers.ts`, though: a group has to own its `Action` keys
 * outright, and every key here is already owned by `block-handlers.ts`
 * for the outline case. Two groups holding `IndentBlock` would make the
 * spread order precedence, and the assembly's own comment says the order
 * is readability. So this module owns the cross-page half and
 * `block-handlers.ts` consults it first — the shape Task 8 already gave
 * `enterInsertOnBacklink`, for the same reason.
 *
 * Four decisions live here, and none of them is the client's to take
 * twice:
 *
 * - **Which command a gesture goes to.** Never the `block.rs` twin the
 *   outline uses: those act on the page the user is looking at and may
 *   therefore *tolerate* a stale block id — `createBlock` by appending
 *   at the end of the page, the rest by reporting a raw `ActionError`
 *   (a ULID), and `moveBlockUp` / `moveBlockDown` / `indentBlock` by
 *   answering success while doing nothing. A backlink's `block_id` is
 *   routinely transient, so all nine route through
 *   `backlink_row_op` / `create_backlink_sibling`, which refuse in
 *   `outl_actions::refusal`'s words. `editBacklinkBlock` is the tenth
 *   and oldest member of that family.
 * - **Which `(page, block)` pair a row addresses** is
 *   `backlinkEditTargetFor`, shared with mobile; the row itself is found
 *   by `BacklinkRow.key`, never by block id.
 * - **What a cross-page reply may touch** is `backlinkCommitPlan`. The
 *   reply describes the *source* page, so it reaches the screen only when
 *   that page is the one on screen — which a page citing itself makes
 *   reachable.
 * - **When the backlinks are re-asked**: per commit, never per keystroke,
 *   and a burst of commits is one scan in flight plus one queued.
 *   `pageBacklinks` is an `O(blocks-in-workspace)` scan (invariant 15),
 *   and "per commit" is not a bound on a **held** key: every repeat of
 *   `Alt+↑` or `Tab` is a commit, so the count is `trackBacklinks`'s to
 *   keep (`lib/backlinks-fetch.ts`), not this module's to remember.
 *
 * **Nothing here nudges.** A citing block's own siblings are not drawn,
 * so indent / outdent / move on one lands on a page the user is not
 * looking at; on a **descendant** shown under it the refetch redraws the
 * subtree, so the result is visible. Shipping them for both is a recorded
 * decision
 * (`docs/superpowers/specs/2026-11-19-backlink-inline-edit-design.md`),
 * which is why `Capability::BacklinkInlineEdit` reads `Full` here and on
 * mobile. The TUI is `Partial` on that row for a gap this client does
 * not have: nothing in its panel opens a cited block's page, where the
 * group header here is a `<button>` that does. The two messages below
 * are for the cases where there is no block to act on at all.
 */

import {
  type BacklinkRowOp,
  backlinkRowOp,
  createBacklinkSibling,
} from "@outl/shared/api/commands";
import {
  type BacklinkCommitPlan,
  backlinkCommitPlan,
  backlinkCreatedElsewhere,
  type BacklinkEditTarget,
  backlinkEditTargetFor,
} from "@outl/shared/backlink-edit";
import type {
  Backlink,
  BlockNode,
  PageMeta,
  PageView,
} from "@outl/shared/api/types";

import {
  backlinkCursor,
  backlinkRowByKey,
  backlinkRowIds,
  startBacklinkEdit,
} from "./cursor-region";
import { type HandlerCtx } from "./handler-context";
import { trackBacklinks } from "./backlinks-fetch";
import { appState, setAppState } from "./store";

/** The cursor names an id the list no longer holds — the backlinks were
 *  re-asked while it sat here, and the row it pointed at is gone. Named
 *  without a chord in it, because `j` / `k` and the arrows both move
 *  this cursor and only one of the two is a vim user's. */
const ROW_GONE =
  "That reference left the list while the cursor was on it — pick another row and try again";

/** `source_page` is null: the citing block lives outside any page, which
 *  is what legacy and migrated workspaces hold. There is no document to
 *  send a mutation to, and guessing one writes to the wrong page. */
const ORPHAN =
  "This reference has no page of its own, so there is nothing to write the change to";

/** The cross-page half of the structural chords. Each method answers
 *  `false` when the cursor is in the outline, which is the caller's cue
 *  to fall through; `true` covers a refusal too, because falling through
 *  from a backlink row acts on `appState.selectedBlockId`, and that is
 *  `null` here by construction. */
export interface BacklinkRowOps {
  /** Park Insert on the row, writing the `(page, block)` pair the editor
   *  commits to. Three chords reach it — `i` and its `Shift+I` / `a`
   *  aliases, and `Enter` — so a second copy of the lookup is how one of
   *  them starts disagreeing about which rows are editable. Synchronous:
   *  `Enter` parks Insert in the tick it was pressed. */
  enterInsert(): boolean;
  /** Run one of the catalog's cross-page ops against the cited block.
   *
   *  **The op is a tag, not the outline's own command.** Every structure
   *  op and both task-state ops go through the single
   *  `backlink_row_op` command, which is what makes the refusal one
   *  sentence instead of nine: its `BacklinkRowOp` is an exhaustive
   *  `match` in Rust, so a tenth gesture cannot ship printing a raw
   *  `ActionError`. Routed through `indentBlock` / `toggleTodo` / … —
   *  the page-the-user-is-looking-at twins — these printed a ULID, and
   *  `MoveBlockUp` / `MoveBlockDown` / `IndentBlock` answered success
   *  while doing nothing. */
  run(op: BacklinkRowOp): Promise<boolean>;
  /** Delete it, and step the cursor off the row that is about to vanish. */
  del(): Promise<boolean>;
  /** Create an empty sibling beside it, on its own page. Its own method
   *  rather than a `run`, because the reply carries `new_id` — and its
   *  own *command*, because `createBlock`'s stale-anchor fallback
   *  appends at the end of the page it is given. */
  createSibling(side: "below" | "above"): Promise<boolean>;
}

export function backlinkRowOps(ctx: HandlerCtx): BacklinkRowOps {
  // Plain record, not Solid props — see `HandlerCtx`.
  const { deps, safeCall } = ctx;

  /** Install what the reply is allowed to touch, and re-ask for the rows
   *  the mutation may have changed. Hands the plan back because creating
   *  a sibling needs to know whether the new block is on screen. */
  function settle(
    sourcePageId: string,
    view: PageView,
  ): { plan: BacklinkCommitPlan; refetched: Promise<void> } {
    const plan = backlinkCommitPlan({
      sourcePageId,
      openPageId: appState.page?.id,
    });
    if (plan.applyView) deps.applyView(view);
    // Not awaited: the outline keeps the caret and the keystrokes while
    // the scan runs, and the section keeps its rows across it
    // (`lateSectionVisible`).
    //
    // Asked unconditionally, and that is safe because the ask is
    // coalesced by its owner rather than counted here: a held `Alt+↑`
    // commits once per key repeat, and `trackBacklinks` turns the burst
    // into one scan in flight plus one queued. Naming no page is the
    // other half — by the time this runs, the page the user is on is
    // the only one that can be asked about.
    const refetched = plan.refetchBacklinks
      ? trackBacklinks()
      : Promise.resolve();
    return { plan, refetched };
  }

  /** Where the cursor goes once the row `key` leaves the list: the row
   *  above, else the one below, else nowhere. Read **before** the
   *  mutation, since the refetch is what drops the row.
   *
   *  Skips every row the delete takes with it: the block's own subtree,
   *  and any *other* drawn copy of a block in it (a descendant that also
   *  cites the page is its own row, so deleting its ancestor drops that
   *  row too). A key is a path of block ids, so a row is gone when any
   *  segment names a deleted block.
   *
   *  A cursor left on a vanished key answers no keystroke —
   *  `SelectionUp` / `SelectionDown` look it up by `indexOf` and stay put
   *  on `-1`, so the section would hold the keyboard with nothing to
   *  move. `null` releases it instead: with neither cursor set, `j`
   *  enters the outline at the top. */
  function neighbourRow(key: string): string | null {
    const ids = backlinkRowIds();
    const idx = ids.indexOf(key);
    const doomed = backlinkRowByKey(key);
    if (idx < 0 || !doomed) return null;
    const gone = new Set<string>();
    const collect = (n: BlockNode) => {
      gone.add(n.id);
      n.children.forEach(collect);
    };
    collect(doomed.node);
    const survives = (k: string) => !k.split(">").some((id) => gone.has(id));
    const above = ids.slice(0, idx).reverse().find(survives);
    const below = ids.slice(idx + 1).find(survives);
    return above ?? below ?? null;
  }

  /** Resolve the focused row, surfacing the refusal on the way. **The one
   *  place that decides whether this module answers a keystroke at all**,
   *  and it is synchronous because one of its callers must be: parking
   *  Insert is a store write with no round-trip, and a microtask in front
   *  of it would leave `Enter` landing a frame after the keystroke. */
  function resolveRow():
    | { kind: "outline" }
    | { kind: "refused" }
    | {
        kind: "row";
        key: string;
        link: Backlink;
        node: BlockNode;
        depth: number;
        page: PageMeta;
        target: BacklinkEditTarget;
      } {
    const key = backlinkCursor();
    if (key === null) return { kind: "outline" };
    const found = backlinkRowByKey(key);
    if (!found) {
      deps.setError(ROW_GONE);
      return { kind: "refused" };
    }
    const { link, node, depth } = found;
    const target = backlinkEditTargetFor(link, node);
    // `target` is the verdict; `page` is the same fact narrowed, so the
    // row can carry a title no caller has to re-check for null. The
    // policy stays `backlinkEditTarget`'s — if it ever stops keying on
    // `source_page`, `!target` still gates.
    const page = link.source_page;
    if (!target || !page) {
      deps.setError(ORPHAN);
      return { kind: "refused" };
    }
    return { kind: "row", key, link, node, depth, page, target };
  }

  /** Hand the focused row to `act`, or report that there is no row here
   *  for the caller to fall through on. */
  async function onRow(
    act: (row: {
      key: string;
      depth: number;
      target: BacklinkEditTarget;
      page: PageMeta;
    }) => Promise<void>,
  ): Promise<boolean> {
    const row = resolveRow();
    if (row.kind === "outline") return false;
    if (row.kind === "row") await act(row);
    return true;
  }

  return {
    enterInsert: () => {
      const row = resolveRow();
      if (row.kind === "outline") return false;
      if (row.kind === "row") startBacklinkEdit(row.target, row.key);
      return true;
    },

    run: (op) =>
      onRow(async ({ target }) => {
        const view = await safeCall(
          backlinkRowOp(target.pageId, target.blockId, op),
        );
        if (view) settle(target.pageId, view);
      }),

    del: () =>
      onRow(async ({ key, target }) => {
        const next = neighbourRow(key);
        const view = await safeCall(
          backlinkRowOp(target.pageId, target.blockId, "delete"),
        );
        if (!view) return;
        setAppState("selectedBacklinkBlockId", next);
        settle(target.pageId, view);
      }),

    createSibling: (side) =>
      onRow(async ({ depth, target, page }) => {
        const reply = await safeCall(
          createBacklinkSibling(target.pageId, target.blockId, side),
        );
        if (!reply) return;
        const { plan, refetched } = settle(target.pageId, reply.view);
        if (plan.applyView) {
          // The source page *is* the page on screen, so the new block is
          // in the outline the user is reading: do what `o` does there,
          // and say nothing — `backlinkCreatedElsewhere` is `null` here too.
          setAppState({
            selectedBacklinkBlockId: null,
            selectedBlockId: reply.new_id,
            editingBlockId: reply.new_id,
          });
          return;
        }
        // A sibling of a **shown descendant** is itself inside the drawn
        // subtree, so once the refetch lands it has a row: park Insert
        // there, as `o` does in the outline. One that is not on screen
        // (a sibling of a citing block, which cites nothing) falls
        // through to the catalog's sentence, which is `null` for a
        // descendant row (`depth`) and owns that decision.
        await refetched;
        const created = backlinkRowIds()
          .map((k) => backlinkRowByKey(k))
          .find((r) => r?.node.id === reply.new_id && r.depth > 0);
        if (created) {
          const t = backlinkEditTargetFor(created.link, created.node);
          if (t) {
            startBacklinkEdit(t, created.key);
            return;
          }
        }
        const notice = backlinkCreatedElsewhere(
          page,
          side,
          appState.page?.id,
          depth,
        );
        if (notice !== null) setAppState("notice", [notice]);
      }),
  };
}
