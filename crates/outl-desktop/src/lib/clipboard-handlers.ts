/**
 * The block clipboard and the yank register.
 *
 * What these four chords share is not a target but a **register** —
 * state that outlives the keystroke and is read by a later one
 * (`appState.blockClipboard`, `appState.yankRegister`, the OS
 * clipboard). That is why they are their own module rather than
 * sitting with the other ops on the selected block.
 */

import {
  copyBlockMarkdown,
  copyMarkdown,
  moveBlockAfter,
  pasteBlockAfter,
} from "@outl/shared/api/commands";

import { type HandlerCtx, lookupBlock } from "./handler-context";
import { type ActionHandlers } from "./shortcuts";
import { appState, setAppState } from "./store";

/** Serialize a block selection to clean outl markdown (backend) and
 *  write it to the OS clipboard. Both halves are best-effort: the
 *  webview's `navigator.clipboard` can reject (no focus / permission),
 *  and a copy never blocks the yank register that `p`/`P` reads. */
export async function copySelectionToClipboard(
  ctx: HandlerCtx,
  blockIds: string[],
): Promise<void> {
  if (blockIds.length === 0) return;
  const md = await ctx.safeCall(copyMarkdown(blockIds));
  if (md == null) return;
  void navigator.clipboard?.writeText(md).catch(() => {});
}

export function clipboardHandlers(ctx: HandlerCtx): ActionHandlers {
  // Plain record, not Solid props — see `HandlerCtx`.
  const { deps, safeCall } = ctx;
  return {
    // `Y` — yank the selected block as clean outl markdown to the OS
    // clipboard (subtree included), and keep its text in the in-app
    // register that `p` / `P` will read. Copy-out is the inverse of
    // paste-in: the markdown re-pastes into outl as the same tree.
    YankCurrentBlock: () => {
      const id = appState.selectedBlockId;
      if (!id) return;
      const block = lookupBlock(id);
      if (!block) return;
      setAppState("yankRegister", [block.text]);
      void copySelectionToClipboard(ctx, [id]);
    },
    // ── block clipboard (view-mode Cmd+X / Cmd+C / Cmd+V) ────────
    //
    // These fire only in Normal mode — inside a block editor the same
    // chords aren't in the catalog, so the textarea gets the OS-native
    // text cut / copy / paste instead.
    //
    // Cut marks the selected block to **move by id**: the paste emits
    // an `Op::Move`, so the block keeps its identity (refs / backlinks
    // survive). Copy snapshots the subtree as markdown; its paste
    // duplicates with fresh ids.
    CutBlock: () => {
      const id = appState.selectedBlockId;
      if (!id) return;
      setAppState("blockClipboard", { kind: "cut", nodeId: id });
    },
    CopyBlock: async () => {
      const id = appState.selectedBlockId;
      if (!id) return;
      const markdown = await safeCall(copyBlockMarkdown(id));
      if (markdown === undefined) return;
      setAppState("blockClipboard", { kind: "copy", markdown });
    },
    PasteBlock: async () => {
      const pageId = appState.page?.id;
      const after = appState.selectedBlockId;
      const clip = appState.blockClipboard;
      if (!pageId || !after || !clip) return;
      if (clip.kind === "cut") {
        // Pasting a block right where it already is — nothing to do.
        if (clip.nodeId === after) return;
        const view = await safeCall(moveBlockAfter(pageId, clip.nodeId, after));
        // On failure (e.g. the move would create a cycle) the backend
        // error is already surfaced; keep the cut armed so the user
        // can paste somewhere valid.
        if (!view) return;
        deps.applyView(view);
        // A cut is consumed by its paste; follow the moved block.
        setAppState("blockClipboard", null);
        setAppState("selectedBlockId", clip.nodeId);
      } else {
        const view = await safeCall(
          pasteBlockAfter(pageId, after, clip.markdown),
        );
        // A copy persists so it can be pasted again.
        if (!view) return;
        deps.applyView(view);
      }
    },
  };
}
