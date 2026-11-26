/**
 * Copy, cut and paste of whole blocks.
 *
 * What these chords share is not a target but a **register** — state
 * that outlives the keystroke and is read by a later one. There are two:
 * the OS clipboard (every copy, and every paste with no cut armed), and
 * `appState.blockClipboard` (a pending cut, moved by id on paste). That
 * is why they are their own module rather than sitting with the other
 * ops on the selected block.
 *
 * The OS clipboard is reached only through `./os-clipboard`, which is
 * also what orders a `yy` and the `p` right after it.
 */

import {
  copyBlockEmbed,
  copyBlockRef,
  copyMarkdown,
  moveBlockAfter,
  pasteBlockAfter,
} from "@outl/shared/api/commands";

import { backlinkCursor, backlinkRowByKey } from "./cursor-region";
import { type HandlerCtx } from "./handler-context";
import { isEditing } from "./mode";
import { readOsClipboard, writeOsClipboard } from "./os-clipboard";
import { type ActionHandlers } from "./shortcuts";
import { appState, setAppState } from "./store";

/** `p` on a backlink row. The row's block lives on another page, and a
 *  paste lands in the document on screen, so there is no "after this
 *  row" to paste to. */
export const PASTE_IN_OUTLINE =
  "Paste works in the outline; move out of Linked from";

function messageOf(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** The default sink: the status line, which is where `deps.setError`
 *  writes too. A caller with its own sink passes it. */
function toStatusLine(msg: string) {
  setAppState("lastError", msg);
}

/** Queue `text` on the OS clipboard and report a refusal. The write is
 *  enqueued synchronously, before the first `await`, which is what a
 *  `p` pressed right after relies on.
 *
 *  Every block copy goes through here, so this is also where a copy
 *  disarms a pending cut: the latest gesture is what `Cmd+V` pastes,
 *  and a cut left armed behind a Visual `y` or a `y r` would move the
 *  cut block instead. */
async function writeOrReport(
  text: string | Promise<string | undefined>,
  onError: (msg: string) => void,
): Promise<void> {
  if (appState.blockClipboard) setAppState("blockClipboard", null);
  try {
    await writeOsClipboard(text);
  } catch (e) {
    onError(messageOf(e));
  }
}

/** Serialize a block selection to clean outl markdown (backend) and
 *  write it to the OS clipboard. A failure on either half reaches the
 *  status line: `safeCall` reports the serialization, and the write's
 *  refusal goes to `deps.setError`. */
export function copySelectionToClipboard(
  ctx: HandlerCtx,
  blockIds: string[],
): Promise<void> {
  if (blockIds.length === 0) return Promise.resolve();
  return writeOrReport(
    ctx.safeCall(copyMarkdown(blockIds)),
    ctx.deps.setError,
  );
}

/** Put a block handle — `((blk-x))` or its `!((blk-x))` embed
 *  spelling — on the OS clipboard.
 *
 *  It takes the already-guarded promise rather than the block id so
 *  the caller keeps its own error sink: the backend rejects with
 *  "no ref handle yet — save and retry" for a block the sidecar has
 *  not seen, and that refusal has to reach the user wherever the copy
 *  was asked for. `undefined` therefore means it was already
 *  surfaced and there is nothing to write. A refused write goes to
 *  `onError`, the status line unless the caller says otherwise.
 *
 *  Only the `((…))` spelling is shared, never built here: `outl-md`
 *  owns both tokens, so a client that prefixed a `!` would be a second
 *  owner of the syntax. */
export function copyBlockHandleToClipboard(
  token: Promise<string | undefined>,
  onError: (msg: string) => void = toStatusLine,
): Promise<void> {
  return writeOrReport(token, onError);
}

/** The block a copy chord means: the outline's selection, or the block
 *  a backlink row shows when the cursor is on one. `null` when the
 *  cursor is nowhere, or on a row a refetch just dropped. */
function copyTargetId(): string | null {
  const key = backlinkCursor();
  if (key !== null) return backlinkRowByKey(key)?.node.id ?? null;
  return appState.selectedBlockId;
}

/** Text the user dragged the mouse across, if any. `Cmd+C` over it
 *  means that text, not the block the cursor happens to sit on. */
function draggedText(): string | null {
  return window.getSelection()?.toString() || null;
}

export function clipboardHandlers(ctx: HandlerCtx): ActionHandlers {
  // Plain record, not Solid props — see `HandlerCtx`.
  const { deps, safeCall } = ctx;

  /** Copy the block under the cursor (subtree included) to the OS
   *  clipboard as clean outl markdown, which re-pastes into outl as the
   *  same tree. */
  function copyBlock(): Promise<void> | undefined {
    const id = copyTargetId();
    if (!id) return;
    return copySelectionToClipboard(ctx, [id]);
  }

  /** One paste at a time. A held `p` would otherwise send a paste per
   *  key repeat, each anchored on a selection the previous one has not
   *  moved yet, and stack copies in reverse order. */
  let pasting = false;

  /** Paste the OS clipboard after the selected block, as blocks.
   *
   *  Everything the paste depends on is read **before** the clipboard
   *  read, because that read waits for any yank still in flight and the
   *  user can move in the meantime. A navigation during the read
   *  abandons the paste; one during the IPC keeps the ops (they are in
   *  the log) but leaves the screen alone. The cursor follows the
   *  pasted block only if it did not move on its own. */
  async function pasteClipboard(): Promise<void> {
    if (backlinkCursor() !== null) {
      deps.setError(PASTE_IN_OUTLINE);
      return;
    }
    const pageId = appState.page?.id;
    if (!pageId || pasting) return;
    const selected = appState.selectedBlockId;
    const zoom = appState.focusBlockId;
    // Zoomed with the cursor on the root (or nowhere): the root is not
    // drawn, so "after it" would land outside the subtree on screen.
    const anchor =
      zoom && (selected === zoom || selected === null)
        ? { parentId: zoom }
        : { afterId: selected };
    pasting = true;
    try {
      let text: string;
      try {
        text = await readOsClipboard();
      } catch (e) {
        deps.setError(messageOf(e));
        return;
      }
      if (appState.page?.id !== pageId) return;
      const reply = await safeCall(pasteBlockAfter(pageId, anchor, text));
      if (!reply || appState.page?.id !== pageId) return;
      deps.applyView(reply.view);
      if (reply.new_id && appState.selectedBlockId === selected && !isEditing()) {
        setAppState("selectedBlockId", reply.new_id);
      }
    } finally {
      pasting = false;
    }
  }

  return {
    // `y y` / `Y` — the block (subtree included) to the OS clipboard.
    YankCurrentBlock: copyBlock,
    // `y r` / the `refer` and `refer-embed` commands — the block's
    // ref handle, for pasting a `((blk-x))` somewhere else.
    CopyBlockRef: () => {
      const id = appState.selectedBlockId;
      if (!id) return;
      return copyBlockHandleToClipboard(safeCall(copyBlockRef(id)), deps.setError);
    },
    CopyBlockEmbed: () => {
      const id = appState.selectedBlockId;
      if (!id) return;
      return copyBlockHandleToClipboard(
        safeCall(copyBlockEmbed(id)),
        deps.setError,
      );
    },
    // ── view-mode Cmd+X / Cmd+C / Cmd+V ──────────────────────────
    //
    // These fire only in Normal mode — inside a block editor the same
    // chords aren't in the catalog, so the textarea gets the OS-native
    // text cut / copy / paste instead.
    //
    // Cut marks the selected block to **move by id**: the paste emits
    // an `Op::Move`, so the block keeps its identity (refs / backlinks
    // survive). Copy is `yy`, except over a mouse-dragged selection.
    CutBlock: () => {
      const id = appState.selectedBlockId;
      if (!id) return;
      setAppState("blockClipboard", { kind: "cut", nodeId: id });
    },
    CopyBlock: () => {
      const dragged = draggedText();
      if (dragged === null) return copyBlock();
      return writeOrReport(dragged, deps.setError);
    },
    // `p` — the OS clipboard, as blocks after the selection.
    PasteClipboard: pasteClipboard,
    // `Cmd+V` — a pending cut moves; otherwise it is `p`, the same
    // function, so the two chords cannot paste different things.
    PasteBlock: async () => {
      const clip = appState.blockClipboard;
      if (!clip) return pasteClipboard();
      const pageId = appState.page?.id;
      const after = appState.selectedBlockId;
      if (!pageId || !after) return;
      // Pasting a block right where it already is — nothing to do.
      if (clip.nodeId === after) return;
      const view = await safeCall(moveBlockAfter(pageId, clip.nodeId, after));
      // On failure (e.g. the move would create a cycle) the backend
      // error is already surfaced; keep the cut armed so the user
      // can paste somewhere valid.
      if (!view) return;
      // A cut is consumed by its paste, even when the user navigated
      // away during the IPC: the move is in the log, and an armed cut
      // would move the block a second time on the next `Cmd+V`.
      setAppState("blockClipboard", null);
      if (appState.page?.id !== pageId) return;
      deps.applyView(view);
      setAppState("selectedBlockId", clip.nodeId);
    },
  };
}
