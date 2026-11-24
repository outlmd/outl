import type { CaretExit } from "@outl/shared/caret-nav";
import type { PasteEditorUpdate } from "@outl/shared/paste";

import type { CaretLandingTarget } from "./outline-caret-nav";

/**
 * What a block row asks of whoever renders it.
 *
 * `<OutlineView />` is the only implementor: it owns the Tauri
 * round-trips and the store writes, so every row-level gesture lands
 * here rather than reaching the backend itself. The interface lives in
 * its own file because three different pieces of the row depend on it
 * (`<BlockRow />`, `<BlockBody />`, `<BlockEditor />`) and none of them
 * should have to import a component to get at a type.
 *
 * Re-exported from `./BlockRow` so existing call sites keep importing
 * it from there.
 */
export interface BlockCallbacks {
  /** A textarea was double-clicked → enter edit mode on `id`. */
  onStartEdit: (id: string) => void;
  /** Commit the current edit. Called on Esc / blur / structural ops. */
  onCommit: (id: string, text: string) => Promise<void>;
  /** Enter pressed → commit + create a sibling below + focus it. */
  onEnter: (id: string, text: string, caretChars: number) => Promise<void>;
  /**
   * `Cmd/Ctrl+Shift+Enter` with the caret at column 0 → commit + create
   * a sibling *before* this one + focus it. The textarea mirror of vim `O`.
   */
  onCreateBefore: (id: string, text: string) => Promise<void>;
  /**
   * An arrow key walked the caret off the edge of this block.
   *
   * Two calls, in this order, because a `preventDefault` only counts
   * during the synchronous dispatch and the editor must not commit on a
   * keystroke that goes nowhere:
   *
   * 1. `resolveCaretExit` — pure, answers `null` at the outline's
   *    edges, where the editor leaves the keystroke to the browser.
   * 2. `openCaretLanding` — called after the draft is committed, puts
   *    Insert mode (and the caret) on the neighbour.
   *
   * See `./outline-caret-nav`.
   */
  resolveCaretExit: (id: string, exit: CaretExit) => CaretLandingTarget | null;
  openCaretLanding: (target: CaretLandingTarget) => void;
  /** Tab pressed inside the textarea. */
  onIndent: (id: string) => Promise<void>;
  /** Shift-Tab pressed inside the textarea. */
  onOutdent: (id: string) => Promise<void>;
  /** Move chord pressed inside the textarea (catalog Insert rows). */
  onMoveUp: (id: string) => Promise<void>;
  onMoveDown: (id: string) => Promise<void>;
  /** Backspace on empty text → delete this block, jump cursor to prev. */
  onDeleteEmpty: (id: string) => Promise<void>;
  /** Checkbox click — flip TODO/DONE/none. */
  onToggleTodo: (id: string) => Promise<void>;
  /** Chevron click — fold / unfold. */
  onToggleCollapsed: (id: string, collapsed: boolean) => Promise<void>;
  /** External-clipboard paste with formatting (Cmd+V) — structured
   *  payload is converted to blocks. `hostText` is the in-flight
   *  textarea value so the parent can flush the draft into the
   *  workspace before splicing (else the caret is measured on the draft
   *  but applied to stale backend text).
   *
   *  Resolves to the editor update the paste produced (`null` when it
   *  left the host block alone). The editor **must** apply it: the
   *  paste rewrote the very block the textarea is showing, and a draft
   *  still holding the pre-paste string is committed back over the
   *  paste on the next Esc / blur (issue #488). */
  onPasteMarkdown: (
    id: string,
    caret: number,
    text: string,
    hostText: string,
  ) => Promise<PasteEditorUpdate | null>;
  /** Paste without formatting (Cmd+Shift+V) — raw text spliced at the
   *  caret, no conversion. `hostText` is flushed like `onPasteMarkdown`,
   *  and the resolved update carries the same obligation. */
  onPastePlain: (
    id: string,
    caret: number,
    text: string,
    hostText: string,
  ) => Promise<PasteEditorUpdate | null>;
  /** Run a fenced code block through `outl-exec`. */
  onRunCodeBlock: (id: string) => Promise<void>;
  /** Run a plugin command picked from the inline `/` slash menu. The
   *  parent owns the `pluginRun` round-trip + view/overlay application,
   *  same as `PluginPalette` does for the `⧉` palette. */
  onRunPluginCommand: (pluginId: string, commandId: string) => Promise<void>;
  /** Commit a `key:: value` property edit; an empty value clears it.
   *  Returning the promise lets the editor surface a rejected write —
   *  the chip repaints either way, so a swallowed failure reads as a
   *  successful edit. */
  onSetProperty?: (
    blockId: string,
    key: string,
    value: string,
  ) => void | Promise<void>;
  /** Ref / tag click handlers (forwarded to MarkdownInline). */
  onRefClick: (target: string) => void;
  onTagClick: (tag: string) => void;
  /** Navigate to a page by its exact slug. Used by a `call:<name>` code
   *  fence to jump to the template's page. Unlike `onRefClick` (→
   *  `openRef`, which *creates* a page when the target doesn't resolve),
   *  this is an exact `openPageBySlug` — no side effect on a miss. */
  onOpenPage: (slug: string) => void;
  /** External `[label](url)` link click — opens in the system browser.
   *  Optional: contexts that keep links inert simply omit it (the
   *  renderer then draws a plain, non-interactive span). */
  onLinkClick?: (href: string) => void;
  /** Bullet right-click → copy this block's `((blk-x))` handle, or its
   *  `!((blk-x))` embed spelling, to the OS clipboard — the GUI's half
   *  of the TUI's `/refer` and `/refer-embed`.
   *
   *  Optional, and the menu draws only the rows it was given: a host
   *  that cannot address the backend (a backlink row, whose mutations
   *  belong to another page) offers neither rather than offering a row
   *  that does nothing. */
  onCopyBlockRef?: (id: string) => void;
  onCopyBlockEmbed?: (id: string) => void;
  /** Bullet marker click → zoom into this block (Roam/Workflowy focus).
   *  The fold chevron stays the collapse gesture; the `•`/`▢`/`▣` marker
   *  is the zoom gesture. Optional so contexts without zoom omit it. */
  onFocusBlock?: (id: string) => void;
}

/**
 * The subset of {@link BlockCallbacks} that `<BlockEditor />` actually
 * asks for — the questions a *textarea* has, as against the questions a
 * rendered row has.
 *
 * It exists because `<BlockEditor />` is mounted in two places now: an
 * outline row, where `<OutlineView />` answers all eighteen, and a
 * backlink row, where the mutation is addressed to another page and only
 * these are answerable. Requiring the full interface there would have
 * meant seven stubs for gestures the editor never invokes — seven
 * statements about behaviour that are not true.
 *
 * `BlockCallbacks` satisfies it, so every existing call site is
 * unchanged.
 *
 * The two paste members are **optional**, and that is the one place a
 * host can decline. `<BlockEditor />`'s paste handlers `preventDefault`
 * before they call out, so a stub returning `null` does not leave the
 * keystroke alone — it swallows the paste. Omitting them instead routes
 * every paste to the webview's own literal splice, the same early return
 * a code fence already takes, and the spliced text still commits through
 * whatever `onCommit` is.
 */
export type EditorCallbacks = Pick<
  BlockCallbacks,
  | "onCommit"
  | "onEnter"
  | "onCreateBefore"
  | "resolveCaretExit"
  | "openCaretLanding"
  | "onIndent"
  | "onOutdent"
  | "onMoveUp"
  | "onMoveDown"
  | "onDeleteEmpty"
  | "onRunPluginCommand"
> &
  Partial<Pick<BlockCallbacks, "onPasteMarkdown" | "onPastePlain">>;
