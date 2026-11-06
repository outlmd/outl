/**
 * The caret paste: hand the clipboard to the backend and get back both
 * the page and the block the user is typing in.
 *
 * Split out of `commands.ts` / `types.ts` under the file-size ratchet,
 * and re-exported from both so `@outl/shared/api/commands` and
 * `@outl/shared/api/types` stay the one import path each. Listed in the
 * wire-mirror files (`outl-tauri-shared/tests/ts_parser`).
 *
 * The routing decision that happens *before* any of this
 * (`choosePasteRoute`) and the editor fold that happens *after*
 * (`pasteEditorUpdate`) live in `@outl/shared/paste` — pure, no invoke.
 */

import { invoke } from "@tauri-apps/api/core";

import type { PageView } from "./types";

/**
 * Reply of {@link pasteMarkdown} / {@link pastePlain}. Mirrors the Rust
 * `outl_tauri_shared::state::PasteReply`, which carries the full
 * reasoning.
 *
 * `view` is the refreshed page, like every other mutation's reply.
 * `host_text` is the new text of the block the caret was in, present
 * exactly when the paste merged something into it, and `host_caret` is
 * a **codepoint** offset into it (run it through
 * `charOffsetToUtf16Offset` before `setSelectionRange`).
 *
 * Apply both. A client that only re-renders the outline leaves its
 * editor draft on the pre-paste string and commits that back over the
 * paste (issue #488); `pasteEditorUpdate` from `@outl/shared/paste` is
 * the shared way to fold them in. Never re-derive them as "what I sent
 * plus where I sent it" — the backend built the string and may have
 * inserted a separating space.
 */
export interface PasteReply {
  view: PageView;
  host_text?: string;
  host_caret?: number;
}

/**
 * Hand off an external-clipboard paste to the backend for conversion
 * into a tree of blocks. The Rust side normalises external syntax
 * (Roam `{{[[TODO]]}}`, GitHub checkboxes, etc.), parses the bullet
 * structure, and grafts it under `blockId` at the caret position.
 *
 * `caret` is a `char` offset into the host block's text — for ASCII
 * content the textarea's `selectionStart` (UTF-16 code units) is
 * equivalent. The frontend should `preventDefault` on the original
 * paste event before calling this so the default browser splice
 * doesn't run alongside the backend conversion.
 */
export function pasteMarkdown(
  pageId: string,
  blockId: string,
  caret: number,
  text: string,
): Promise<PasteReply> {
  return invoke<PasteReply>("paste_markdown_at", {
    pageId,
    blockId,
    caret,
    text,
  });
}

/**
 * Paste clipboard text **without formatting** — the raw string is
 * spliced into the host block at `caret`, with no outline detection,
 * syntax normalization, or paragraph splitting. The plain counterpart of
 * {@link pasteMarkdown} (desktop `Cmd+Shift+V`).
 *
 * The host fields matter more here: a plain splice leaves the user's
 * tail to the right of the caret, so `host_caret` is mid-string rather
 * than at the end of the text.
 */
export function pastePlain(
  pageId: string,
  blockId: string,
  caret: number,
  text: string,
): Promise<PasteReply> {
  return invoke<PasteReply>("paste_plain_at", {
    pageId,
    blockId,
    caret,
    text,
  });
}
