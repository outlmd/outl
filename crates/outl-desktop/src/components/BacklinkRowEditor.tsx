import { createSignal } from "solid-js";

import { editBacklinkBlock } from "@outl/shared/api/commands";
import {
  type BacklinkEditTarget,
  backlinkCommitPlan,
} from "@outl/shared/backlink-edit";
import { rawTextWithTodo } from "@outl/shared/outline";
import type { Backlink } from "@outl/shared/api/types";

import { stopBacklinkEdit } from "../lib/cursor-region";
import { applyPageState } from "../lib/page-view";
import { trackBacklinks } from "../lib/backlinks-fetch";
import { appState, setAppState } from "../lib/store";
import { BlockEditor } from "./BlockEditor";
import type { EditorCallbacks } from "./block-callbacks";

/**
 * Edit mode for one **backlink** row: the block it shows lives on
 * another page, and the mutation is addressed there.
 *
 * Its own file rather than a branch inside `<InlineBacklinks />`, which
 * sits at 364 lines against the 600-line nudge — the axis
 * `<BlockRow />`'s five-file split already uses, and the same answer
 * mobile's `BacklinkRowEditor.tsx` gives.
 *
 * It reuses `<BlockEditor />` instead of growing a second textarea: that
 * component owns the chords, the four inline autocomplete popups and the
 * caret contract, and a parallel copy of any of those is a divergence
 * the user finds before a test does.
 *
 * **The draft is seeded on mount, once per edit session.** This is
 * deliberately *not* the shape `<BlockRow />` has, where the signal's
 * lifetime is the row and only `onStartEdit` re-seeds it — a known
 * defect there, since `setOutline` reconciles on id, so a block a peer
 * rewrote keeps its row and its stale draft. Here the component is
 * mounted only while `appState.editingBacklink` names this row, so
 * every entry point (a click, `i`, `Enter`) seeds it by construction
 * and none can bypass the seeding.
 */
export function BacklinkRowEditor(props: {
  link: Backlink;
  target: BacklinkEditTarget;
}) {
  const wireText = () => rawTextWithTodo(props.link.source_block);
  const [draft, setDraft] = createSignal(wireText());

  /**
   * Write the draft to the block on its own page, then refresh what the
   * mutation can have changed.
   *
   * `editBacklinkBlock`, never `editBlock`. The two differ only in how
   * they recover from a block id the tree no longer holds, and a
   * backlink's really can be one: when a page's sidecar is absent,
   * short or misaligned, the index mints a transient `NodeId` by
   * documented design. `editBlock` keeps the text by appending it at the
   * end of the page, which is a kindness on the page you are reading and
   * a silent move to a page you are not — so this one refuses, and the
   * refusal reaches the user rather than a `tracing::warn!`.
   *
   * **A refusal keeps the editor up, on the user's draft.** Nothing was
   * written, so closing it would destroy the one thing in this whole
   * feature that cannot be reconstructed: the text they just typed. The
   * op log still holds the pre-edit block, the `.md` still holds it, a
   * peer still holds it — the paragraph exists nowhere but in this
   * textarea.
   *
   * That is reachable in the ordinary two-device case, not an edge: the
   * user types into a row drawn from the from-disk index while a peer
   * trashes the citing block. It is also the rule mobile already ships
   * (`BacklinksSection.row-ops.ts` → `if (written) setEditing(null)`),
   * so the two clients now answer a refusal the same way.
   *
   * It was the other way round, under the argument that "holding the
   * user in an editor that can never commit would trap them". Two
   * things make that the wrong trade. The commit cannot succeed, but
   * **leaving can**: the editor closes on any navigation, on
   * `Cmd+Shift+B`, and on a click into another row — the gestures a
   * user reaches for anyway, each of which gives them time to select
   * and copy first. And `isEditing()` (`lib/mode.ts`) reads
   * `editingBacklink`, so holding it also stops the five background
   * refreshes from remounting the row and re-seeding the draft from
   * disk underneath them.
   *
   * Esc therefore re-attempts rather than leaving, which is harmless —
   * nothing was written either time — and the message is what tells
   * them so.
   */
  async function commit(text: string): Promise<void> {
    const openPageId = appState.page?.id;
    const plan = backlinkCommitPlan({
      sourcePageId: props.target.pageId,
      openPageId,
    });
    try {
      const view = await editBacklinkBlock(
        props.target.pageId,
        props.target.blockId,
        text,
      );
      // The reply describes the **source** page. It reaches the open
      // page's state only when the two are the same page, which a page
      // citing itself makes reachable.
      if (plan.applyView) applyPageState(view);
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
      // No `stopBacklinkEdit()`: the draft lives in this component, so
      // unmounting it is what destroyed the user's text. See the doc
      // above for why leaving is still reachable.
      return;
    }
    stopBacklinkEdit();
    // Per commit, never per keystroke: the scan is
    // `O(blocks-in-workspace)` (invariant 15), and the draft above
    // already held what the user was looking at. Not awaited — the
    // outline keeps the caret and the keystrokes while it runs
    // (`UX.md` → Never block a keystroke), and the section keeps its
    // rows across the refetch (`lateSectionVisible`).
    //
    // Names no page: `trackBacklinks` reads the open one, so a
    // navigation during the `await` above cannot leave the page the
    // user left behind being asked about — which is what passing a slug
    // captured before it used to do.
    if (plan.refetchBacklinks) void trackBacklinks();
  }

  /**
   * What the textarea asks of this host, and what each answer costs.
   *
   * Three groups, and the middle one is where a reader will want to
   * change something:
   *
   * - **Text.** `onCommit` is the whole feature.
   * - **Structural chords.** A backlink row renders one flat block: its
   *   siblings and children are not on screen, so indenting or splitting
   *   from inside the textarea has no visible result here. They are
   *   reachable from the *focused row* instead, where the catalog's
   *   `IndentBlock` / `NewBlockBelow` act on the cited block. So each one
   *   finishes the edit rather than doing nothing — `<BlockEditor />`
   *   calls `preventDefault` before it calls out, so a no-op would eat
   *   the keystroke, which is the silence invariant 12 is about.
   * - **Paste.** Declined, by omission. The two paste commands live in
   *   `commands/block.rs`, where every body may *tolerate* a stale block
   *   id by re-targeting — the exact recovery `edit_backlink_block`
   *   exists to refuse. Omitting them routes a paste to the webview's own
   *   literal splice (the early return a code fence already takes) and
   *   the pasted text commits through the one audited command above.
   */
  const cb: EditorCallbacks = {
    onCommit: (_id, text) => commit(text),
    onEnter: (_id, text) => commit(text),
    onCreateBefore: (_id, text) => commit(text),
    // The draft is empty at this point; Backspace here clears the
    // block's text. Removing the block itself is `DeleteBlock` on the
    // focused row.
    //
    // A block whose wire text was *already* empty gets no op at all: the
    // same `raw !== wire` guard `<BlockEditor />`'s own `commit()`
    // applies on the normal path, which this branch bypasses. An op that
    // writes what is already there still lands in every peer's log.
    onDeleteEmpty: async () => {
      if (wireText() === "") return stopBacklinkEdit();
      await commit("");
    },
    // `<BlockEditor />` committed before calling these, so the edit has
    // already landed and the row is on its way back to render mode. The
    // caret intent it captured for the remount has no editor to reach,
    // and left behind it would hijack the next block the user clicks.
    onIndent: async () => setAppState("caretIntent", null),
    onOutdent: async () => setAppState("caretIntent", null),
    // One row, no neighbour rendered — and walking an arrow out of here
    // would start editing a third page from a keystroke aimed at this
    // one, which is the line the TUI's `cross_block_step` already holds.
    resolveCaretExit: () => null,
    // Unreachable while `resolveCaretExit` answers `null`; it is the
    // second half of that one decision, not a second decision.
    openCaretLanding: () => {},
    onRunPluginCommand: async () => {
      setAppState(
        "lastError",
        "slash commands run on the open page, not through a backlink",
      );
    },
  };

  return (
    <BlockEditor
      blockId={props.target.blockId}
      text={draft}
      setText={setDraft}
      wireText={wireText}
      onLeave={stopBacklinkEdit}
      cb={cb}
    />
  );
}
