/**
 * Does an arrow key step out of the block being edited, and where does
 * the caret land in the neighbour?
 *
 * The outline reads as one continuous document: a user holding `←`
 * walks back through the page, and `↑` at the top of a block lands
 * inside the one above without ever leaving Insert. This module is the
 * **single owner** of both halves of that — whether a keystroke leaves
 * the block ({@link caretExit}) and which block it hands the caret to
 * ({@link resolveCaretExit}) — so the desktop and mobile cannot
 * disagree about it. It mirrors the TUI's `cross_block_nav_eligible` /
 * `cross_block_step` + `EditBuffer::move_up`, so neither can disagree
 * with the terminal either. A client is left with only what it alone
 * knows: which blocks are on screen, and how it enters Insert.
 *
 * Four cases, and they are the whole surface:
 *
 * | Key | Fires when | Caret lands |
 * |---|---|---|
 * | `←` | caret at offset 0 | end of the previous block |
 * | `→` | caret at the end | start of the next block |
 * | `↑` | caret on the block's first line | previous block, same column |
 * | `↓` | caret on the block's last line | next block, same column |
 *
 * **Offsets here are codepoints, not UTF-16 code units.** A DOM
 * `selectionStart` is the latter, so a caller converts with
 * `utf16OffsetToCharOffset` (`@outl/shared/paste`) on the way in and
 * `charOffsetToUtf16Offset` on the way out. Every offset this module
 * returns is a codepoint count, which is also what the Rust backend
 * speaks.
 *
 * **Known limitation, shared with the TUI: a line here is a logical
 * line (`\n`), never a soft-wrapped visual one.** A block whose single
 * long line wraps across three rows of the textarea is one line to this
 * module, so `↑` from the middle of it steps to the previous block
 * rather than to the row above. Resolving that needs the browser's
 * layout (a textarea exposes no per-visual-line caret API), and the
 * two ways to get at it are both worse: measuring through a mirror
 * element means copying a dozen font properties that silently rot, and
 * reading the caret back after the browser's own default means the
 * decision is asynchronous and still ambiguous at column 0 of a wrapped
 * row. A rule three clients agree on beats a rule that is right more
 * often on one of them.
 */

import type { BlockNode } from "../api/types";
import { findBlock, flattenVisible, rawTextWithTodo } from "../outline";

/** The four keys that can carry the caret out of a block. */
export type CaretNavKey = "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight";

/**
 * Where the caret lands in the block it steps into.
 *
 * `"end"` / `"start"` are the neighbour's far edges (the `←` / `→`
 * cases, which read as one long line of text). A `column` is the
 * vim-style preferred column the `↑` / `↓` cases carry over, aimed at
 * whichever of the neighbour's lines faces the block being left.
 */
export type CaretLanding = "end" | "start" | { column: number };

/** One step out of the block being edited. */
export interface CaretExit {
  /** Which neighbour in the flat visible outline the caret steps into. */
  to: "previous" | "next";
  landing: CaretLanding;
}

/**
 * Is this keystroke one that can carry the caret out of a block, and
 * which one?
 *
 * The **only** reader of the raw event, which is why it is here and not
 * in a client: both the desktop's `handleKeydown` and mobile's
 * `onKeyDown` had this spelled out, so a modifier guard added to one
 * and not the other is two clients disagreeing about what `Alt+↑`
 * means.
 *
 * A modified arrow is a different chord and answers `null`: `Alt+↑` /
 * `Alt+↓` move the block, `Cmd+↑` / `Cmd+↓` are the OS's own
 * document-edge motion, and `Shift+↑` extends a selection. Takes the
 * four flags structurally rather than a `KeyboardEvent`, so it is
 * testable without a DOM.
 */
export function caretNavKey(event: {
  key: string;
  shiftKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
}): CaretNavKey | null {
  if (event.shiftKey || event.metaKey || event.ctrlKey || event.altKey) {
    return null;
  }
  switch (event.key) {
    case "ArrowUp":
    case "ArrowDown":
    case "ArrowLeft":
    case "ArrowRight":
      return event.key;
    default:
      return null;
  }
}

/** Zero-based logical line of `caret`, and its column within that line. */
function lineColumnAt(chars: string[], caret: number): {
  line: number;
  column: number;
} {
  let line = 0;
  let lineStart = 0;
  for (let i = 0; i < caret; i += 1) {
    if (chars[i] === "\n") {
      line += 1;
      lineStart = i + 1;
    }
  }
  return { line, column: caret - lineStart };
}

/**
 * Should this keystroke leave the block, and where should the caret go?
 * `null` means "the textarea keeps it" — the caller lets the browser's
 * own default move the caret inside the block.
 *
 * A non-empty selection always answers `null`: an arrow key collapses a
 * selection in place on every platform, and hijacking that would make
 * the first arrow after a drag jump to another block.
 */
export function caretExit(
  key: CaretNavKey,
  text: string,
  selectionStart: number,
  selectionEnd: number,
): CaretExit | null {
  if (selectionStart !== selectionEnd) return null;
  const chars = [...text];
  const len = chars.length;
  // A caret past the text is what a stale read looks like (the draft
  // signal lands a frame after a programmatic `value =`). Clamping
  // keeps `←` at a real offset 0 instead of answering "step out" for
  // any caret at all.
  const caret = Math.max(0, Math.min(selectionStart, len));

  switch (key) {
    case "ArrowLeft":
      return caret === 0 ? { to: "previous", landing: "end" } : null;
    case "ArrowRight":
      return caret === len ? { to: "next", landing: "start" } : null;
    case "ArrowUp": {
      const { line, column } = lineColumnAt(chars, caret);
      return line === 0 ? { to: "previous", landing: { column } } : null;
    }
    case "ArrowDown": {
      const { line, column } = lineColumnAt(chars, caret);
      const lastLine = chars.filter((c) => c === "\n").length;
      return line === lastLine ? { to: "next", landing: { column } } : null;
    }
  }
}

/**
 * Resolve an exit's landing to a codepoint offset inside the
 * neighbour's `text`.
 *
 * Takes the whole {@link CaretExit} rather than a direction of its own,
 * so the "which end of the neighbour faces me" half cannot be passed
 * inverted: stepping to the `previous` block arrives at its **last**
 * line, stepping to the `next` one at its **first**. A column past the
 * end of that line clamps to it, the way vim's preferred column does,
 * instead of overshooting into the line after.
 */
export function entryCaretOffset(text: string, exit: CaretExit): number {
  if (exit.landing === "end") return [...text].length;
  if (exit.landing === "start") return 0;
  // Each line's length in codepoints. `\n` is BMP, so splitting on it
  // cannot land inside a surrogate pair.
  const lines = text.split("\n").map((line) => [...line].length);
  const target = exit.to === "previous" ? lines.length - 1 : 0;
  // The target line's first character: every earlier line plus its `\n`.
  let lineStart = 0;
  for (let i = 0; i < target; i += 1) lineStart += lines[i] + 1;
  return lineStart + Math.min(exit.landing.column, lines[target]);
}

/** The block an arrow key hands Insert mode to, and where the caret
 *  opens inside it. */
export interface CaretLandingTarget {
  id: string;
  /** Codepoint offset into {@link text}. */
  caret: number;
  /**
   * The block's **wire text** — the TODO/DONE prefix reattached, which
   * is the string an editor shows.
   *
   * Carried rather than left to the caller to look up, because `caret`
   * was measured against *this* string and the pair has to stay
   * together: a client that re-fetched the block and seeded its editor
   * from the DTO's split `text` would put the caret a `TODO ` short of
   * where the user's arrow key pointed. A client that already renders
   * from the outline (the desktop reads it off the textarea on mount)
   * can ignore it.
   */
  text: string;
}

/**
 * Which block `exit` hands the caret to, and where inside it, or `null`
 * when the keystroke goes nowhere.
 *
 * `blocks` is what the client has on screen and may walk — a zoom's
 * rows, else the page — and the order is {@link flattenVisible}'s, the
 * same one `SelectionUp` / `SelectionDown` move in, so a collapsed
 * subtree is skipped because the caret may not land on a row nothing
 * drew.
 *
 * Pure and synchronous on purpose: a caller decides whether to
 * `preventDefault` from this answer, and that only counts during the
 * keystroke's own dispatch.
 *
 * Three `null`s, and each one is a case that used to go wrong:
 *
 * - **At the outline's edges.** Walking past them would reach a client's
 *   backlinks section, which is *another page* opened for editing from a
 *   keystroke the user aimed at this one. The TUI stops in Normal mode
 *   there for the same reason.
 * - **When the block being edited is no longer in the walk** — a peer
 *   deleted it, or a fold swallowed it. This is why the walk indexes
 *   `flattenVisible` rather than calling `nextVisibleId` /
 *   `previousVisibleId`: those read a missing id as *the first block*,
 *   so an `←` aimed at the character to the user's left would teleport
 *   the caret to the top of the page.
 * - **When the neighbour's id resolves to no block**, which a view
 *   applied between render and keystroke can produce.
 *
 * One `flattenVisible` walk, where the two id helpers plus an edge
 * check cost three — the same reason `visibleRangeSlice` indexes it
 * directly.
 */
export function resolveCaretExit(
  blocks: BlockNode[],
  id: string,
  exit: CaretExit,
): CaretLandingTarget | null {
  const ids = flattenVisible(blocks);
  const idx = ids.indexOf(id);
  if (idx === -1) return null;
  const targetIdx = exit.to === "previous" ? idx - 1 : idx + 1;
  if (targetIdx < 0 || targetIdx >= ids.length) return null;

  const block = findBlock(blocks, ids[targetIdx]);
  if (!block) return null;
  const text = rawTextWithTodo(block);
  return { id: block.id, caret: entryCaretOffset(text, exit), text };
}
