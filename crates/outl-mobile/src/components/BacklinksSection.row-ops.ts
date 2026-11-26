/**
 * What happens to **one backlink row** — every mutation the card can
 * perform, and the one rule they all share.
 *
 * A sibling module rather than more closures inside
 * `BacklinksSection.tsx`, for the reason `Journal.block-ops.ts` is one:
 * the component keeps the chrome (the grouping, the breadcrumb, the
 * row, the sheet) and this keeps the question *"which block on which
 * page, and may it run right now"*. It also holds the section's whole
 * state, the way `Journal.remind.ts` and `Journal.embeds.ts` own theirs,
 * so the component carries no signals of its own.
 *
 * **Every write goes through `runOnSource`.** A backlink row shows a
 * block that lives on another page, so the mutation is addressed there
 * and the reply describes *that* page. Which `(page, block)` pair a
 * gesture targets is `@outl/shared/backlink-edit`'s
 * (`backlinkEditTarget`), and what the reply may touch is the host's
 * (`backlinkCommitPlan`, in `Journal.backlinks.ts`) — the only place
 * that holds both page ids. Nothing here decides either, which is what
 * keeps this client and the desktop from disagreeing about them.
 *
 * **Every write is a cross-page command, never the `block.rs` twin the
 * outline uses.** Those act on the page the user is looking at and may
 * therefore *tolerate* a stale block id — `createBlock` by appending at
 * the end of the page, `toggleTodo` / `deleteBlock` by reporting a raw
 * `ActionError` (a ULID and an implementation noun), and `indentBlock` /
 * `moveBlockUp` / `moveBlockDown` by answering success while doing
 * nothing or, on a block a peer trashed, by re-parenting it *inside the
 * trash* where the user can no longer restore it (`moves.rs` uses
 * `ensure_in_tree`, because `trash::restore` is itself a move). A
 * backlink's `block_id` is routinely transient, so the ten gestures
 * route through `backlinkRowOp` / `createBacklinkSibling` /
 * `editBacklinkBlock`, which take their liveness verdict from one
 * `stale_row_block` and refuse in `outl_actions::refusal`'s words.
 *
 * **The card takes one write at a time.** Each op is a cross-page
 * mutation followed by an `O(blocks-in-workspace)` re-ask, so the
 * window in which one is in flight is long enough for a second gesture
 * to land in it — a tap back into a blurred-but-mounted editor, a
 * double tap on a checkbox, a long press that was still reachable. None
 * of these ops is idempotent from the user's seat: a second toggle
 * flips the task back, a second indent moves the block again. The guard
 * is `pending`, and it is deliberately **card-wide** rather than
 * per-row, because the re-ask replaces every row — two writes in flight
 * race on the rows they are both about to be judged by.
 *
 * **A timeout is not the guard**, and would be worse than none: fire
 * one while the write still lands and the user retries, so the task
 * flips twice.
 */

import type { PageMeta, PageView } from "@outl/shared/api/types";
import {
  type BacklinkRowOp,
  backlinkRowOp,
  createBacklinkSibling,
  editBacklinkBlock,
} from "@outl/shared/api/commands";
import {
  type BacklinkEditTarget,
  type BacklinkRow,
  backlinkEditTargetFor,
} from "@outl/shared/backlink-edit";
import { countDescendants, rawTextWithTodo } from "@outl/shared/outline";
import { createSignal } from "solid-js";

import { haptic } from "../lib/haptics";
import type { PendingBlockDelete } from "./JournalDeleteDialogs";

/** A mutation through a backlink row landed. */
export interface BacklinkCommit {
  /** Page the mutated block lives on — the backlink's source page. */
  sourcePageId: string;
  /**
   * Reply of the mutation, which describes `sourcePageId`'s page and
   * **not** necessarily the one on screen. Whether it may be installed
   * as the open page's view is `backlinkCommitPlan`'s answer.
   */
  view: PageView;
  /**
   * Set when the mutation **created** a block, which is the one case
   * the user has to be told about: the new block cites nothing, so it
   * is a backlink of no page and no row here will ever render it.
   *
   * The sentence is not composed here on purpose. It is only true when
   * the block landed on a page the user is *not* looking at, and the
   * only place holding both page ids is the host — so this reports the
   * page and the side, and `Journal.backlinks.ts` decides whether to
   * say anything.
   *
   * A resolved `PageMeta`, not the `Backlink`: `createBeside` already
   * has to refuse an orphan, so the narrowing happens once at the
   * place that has the fact rather than as a fallback in the sentence.
   */
  created?: { page: PageMeta; side: "below" | "above"; depth: number };
}

export interface BacklinkRowOpsDeps {
  /**
   * Report a landed mutation. Resolving late is the contract, not an
   * accident: the editor stays open until it does, so the row never
   * renders the pre-edit text it still holds while the re-ask runs.
   */
  onEditCommitted: (commit: BacklinkCommit) => void | Promise<void>;
  /**
   * A refusal the user has to read. Nothing was written and retrying
   * the same id cannot succeed, so this is a message, never a Retry.
   */
  onError: (message: string) => void;
}

export function createBacklinkRowOps(deps: BacklinkRowOpsDeps) {
  /** Row **key** (not block id: a block drawn twice has two rows) in edit mode, `null` when none is. */
  const [editing, setEditing] = createSignal<string | null>(null);
  /** Row key with a write in flight, `null` when idle. */
  const [pending, setPending] = createSignal<string | null>(null);
  /** The direction toggle's re-ask is in flight. It is not a row, so it
   *  has no block id to put in `pending` — and `pending` stays a block id
   *  because the per-row dim compares against it. */
  const [reordering, setReordering] = createSignal(false);
  /**
   * The one card-wide guard. Every door asks this, never `pending()` or
   * `reordering()` alone: either alone is the one-directional guard that
   * let a row write land under a reorder's re-ask, whose stale rows then
   * replaced it.
   */
  const busy = () => pending() !== null || reordering();
  /** Row whose long-press sheet is open, `null` when none is. */
  const [menuRow, setMenuRow] = createSignal<BacklinkRow | null>(null);
  /** A delete awaiting confirmation, and the row it would take. */
  const [deleting, setDeleting] = createSignal<{
    row: BacklinkRow;
    descendants: number;
  } | null>(null);

  /**
   * Mutate a row's citing block on the page it lives on, and report the
   * reply. `false` means nothing was written — the row had no page to
   * address (an orphan), the card was busy, or the backend refused.
   *
   * The `onEditCommitted` await is inside the `try` on purpose: it is
   * the host's re-ask, which already reports its own failure, and if it
   * ever threw, saying so beats swallowing it.
   */
  async function runOnSource(
    row: BacklinkRow,
    run: (target: BacklinkEditTarget) => Promise<PageView>,
    created?: BacklinkCommit["created"],
  ): Promise<boolean> {
    const target = backlinkEditTargetFor(row.link, row.node);
    if (!target || busy()) return false;
    setPending(row.key);
    try {
      const view = await run(target);
      await deps.onEditCommitted({ sourcePageId: target.pageId, view, created });
      return true;
    } catch (e) {
      deps.onError(e instanceof Error ? e.message : String(e));
      return false;
    } finally {
      // A guard that outlives its write is a card that never accepts
      // another gesture.
      setPending(null);
    }
  }

  /** The direction toggle, held under the same guard as a row write. */
  async function runReorder(
    toggle: () => void | Promise<void>,
  ): Promise<void> {
    if (busy()) return;
    setReordering(true);
    try {
      await toggle();
    } finally {
      setReordering(false);
    }
  }

  /** The shape the four structural ops share, so none can forget the
   *  source-page routing or the pending guard.
   *
   *  **The op is a tag, not a command.** All six structural and
   *  task-state gestures go through the single `backlinkRowOp`, which is
   *  what makes the refusal one sentence instead of six: its
   *  `BacklinkRowOp` is an exhaustive `match` in Rust, so a seventh
   *  gesture cannot ship printing a raw `ActionError`. */
  const structural =
    (op: BacklinkRowOp) =>
    async (row: BacklinkRow): Promise<void> => {
      haptic("light");
      await runOnSource(row, (t) => backlinkRowOp(t.pageId, t.blockId, op));
    };

  /**
   * Create an empty sibling of the citing block, on its own page.
   *
   * **Insert is deliberately not entered**, and nothing here arms the
   * keyboard. The new block cites nothing, so it is a backlink of no
   * page and no row on this screen will ever render it — there is no
   * textarea to focus, and arming the keyboard for one that never
   * mounts raises it over nothing. The user is told where the block
   * went instead, which the host words.
   *
   * The reply is a `CreateBlockReply`; `new_id` is dropped because this
   * client has nothing to point at it with. That is the declared limit,
   * not an oversight — the TUI reached the same answer from the other
   * side after its re-entry parked the caret on the *citing* block.
   *
   * `createBacklinkSibling`, never `createBlock`: that one recovers
   * from a stale anchor by appending at the end of the page it is
   * given, so `o` on a row whose block a peer trashed left an empty
   * junk block at the end of a page nothing on screen showed — and then
   * said *"Added an empty block after this reference"*, which is false
   * about the one thing the sentence exists to say. Held down, one per
   * repeat.
   */
  async function createBeside(
    row: BacklinkRow,
    side: "below" | "above",
  ): Promise<void> {
    // The orphan refusal `runOnSource` also makes, taken here because
    // this is where the resolved page is needed: the notice names it,
    // and resolving it inside the sentence would mean a fallback that
    // can never fire.
    const page = row.link.source_page;
    if (!page) return;
    haptic("light");
    await runOnSource(
      row,
      async (t) => {
        const reply = await createBacklinkSibling(t.pageId, t.blockId, side);
        return reply.view;
      },
      // `depth` lets the host stay quiet for a child: its sibling is
      // born inside the subtree drawn on this card.
      { page, side, depth: row.depth },
    );
  }

  async function performDelete(row: BacklinkRow): Promise<void> {
    // The block is about to be gone, so a draft pointed at it could
    // only ever commit into a refusal.
    if (editing() === row.key) setEditing(null);
    await runOnSource(row, (t) =>
      backlinkRowOp(t.pageId, t.blockId, "delete"),
    );
  }

  return {
    editing,
    pending,
    busy,
    runReorder,
    menuRow,

    /** `PendingBlockDelete` is what `<BlockDeleteConfirm />` reads; the
     *  link behind it stays here, since only this module can spend it. */
    deletePrompt(): PendingBlockDelete | null {
      const d = deleting();
      return d && { id: d.row.key, descendants: d.descendants };
    },

    /**
     * Tap a row. An orphan backlink is simply inert: there is no page
     * to edit it on and none to navigate to either, so it opens
     * nothing rather than pretending to jump.
     */
    openEditor(row: BacklinkRow) {
      if (!backlinkEditTargetFor(row.link, row.node) || busy()) return;
      haptic("light");
      setEditing(row.key);
    },

    async commitRow(row: BacklinkRow, text: string) {
      // Nothing typed — leave without writing. The draft is rebuilt
      // from the DTO's split `todo` + `text`, so an unconditional
      // commit would rewrite a block written as `[ ] buy milk` to
      // `TODO buy milk` on a tap in and out, and buy an O(blocks)
      // rescan for it. Same guard `commitEdit` carries in `Journal.tsx`.
      if (text === rawTextWithTodo(row.node)) {
        setEditing(null);
        return;
      }
      const written = await runOnSource(row, (t) =>
        editBacklinkBlock(t.pageId, t.blockId, text),
      );
      // Closed only once the fresh rows are in. Closing on the write's
      // own reply leaves the row rendering the tokens it still holds —
      // the pre-edit text — for the length of the scan, so the user
      // watches their edit revert and then reappear.
      //
      // A refusal keeps the editor up on its draft instead: nothing was
      // written, so closing it would drop text the user can no longer
      // see.
      if (written) setEditing(null);
    },

    async toggleRowTodo(row: BacklinkRow) {
      haptic("light");
      await runOnSource(row, (t) =>
        backlinkRowOp(t.pageId, t.blockId, "toggle-todo"),
      );
    },

    /**
     * The disclosure triangle: fold or unfold the row's block **on its
     * own page**, so the fold syncs and the outline there shows it too.
     * An explicit `"collapse"` / `"expand"` read off the row on screen,
     * never a toggle: a toggle aimed at a stale view flips the wrong way.
     * A leaf has nothing to fold.
     */
    async toggleRowCollapsed(row: BacklinkRow) {
      if (row.node.children.length === 0) return;
      haptic("light");
      const op = row.node.collapsed ? "expand" : "collapse";
      await runOnSource(row, (t) => backlinkRowOp(t.pageId, t.blockId, op));
    },

    /** Long press. Refused on an orphan (nothing to offer) and while a
     *  write is in flight (the same concurrent-write defect through a
     *  different door). */
    openMenu(row: BacklinkRow) {
      if (!backlinkEditTargetFor(row.link, row.node) || busy()) return;
      haptic("medium");
      setMenuRow(row);
    },
    closeMenu() {
      setMenuRow(null);
    },

    indent: structural("indent"),
    outdent: structural("outdent"),
    moveUp: structural("move-up"),
    moveDown: structural("move-down"),

    createBelow: (row: BacklinkRow) => createBeside(row, "below"),
    createAbove: (row: BacklinkRow) => createBeside(row, "above"),

    /**
     * Delete a row's block (a citing block or a child of one).
     * **Always asks when it has children**, counted on the row's own
     * node, and the threshold is the outline's own. The card shows the
     * subtree only to depth 4, so a deeper one is still off screen.
     */
    async requestDelete(row: BacklinkRow) {
      if (!backlinkEditTargetFor(row.link, row.node) || busy()) return;
      const descendants = countDescendants(row.node);
      if (descendants > 0) {
        haptic("warning");
        setDeleting({ row, descendants });
        return;
      }
      haptic("heavy");
      await performDelete(row);
    },

    /**
     * Delete the row the prompt was raised on.
     *
     * The link arrives from the caller rather than being read back out
     * of `deleting`, because `<BlockDeleteConfirm />` closes itself
     * first and then confirms — so by this point the signal is already
     * `null`. Resolving it against the **live** rows instead is also
     * the better answer: a sync that replaced them while the prompt was
     * up leaves this acting on the current row, or on none.
     */
    async deleteRow(row: BacklinkRow) {
      setDeleting(null);
      await performDelete(row);
    },
    cancelDelete() {
      setDeleting(null);
    },
  };
}
