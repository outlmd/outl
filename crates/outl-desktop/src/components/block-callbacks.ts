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
  /** Tab pressed inside the textarea. */
  onIndent: (id: string) => Promise<void>;
  /** Shift-Tab pressed inside the textarea. */
  onOutdent: (id: string) => Promise<void>;
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
   *  but applied to stale backend text). */
  onPasteMarkdown: (
    id: string,
    caret: number,
    text: string,
    hostText: string,
  ) => Promise<void>;
  /** Paste without formatting (Cmd+Shift+V) — raw text spliced at the
   *  caret, no conversion. `hostText` is flushed like `onPasteMarkdown`. */
  onPastePlain: (
    id: string,
    caret: number,
    text: string,
    hostText: string,
  ) => Promise<void>;
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
  /** Bullet marker click → zoom into this block (Roam/Workflowy focus).
   *  The fold chevron stays the collapse gesture; the `•`/`▢`/`▣` marker
   *  is the zoom gesture. Optional so contexts without zoom omit it. */
  onFocusBlock?: (id: string) => void;
}
