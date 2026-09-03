/**
 * Keyboard-toolbar action catalog — the single source of truth for the
 * mobile edit toolbar's buttons, their order, and their presentation.
 *
 * **Raw-value contract.** Each action's string id is what the iOS
 * native bar ships to JS via `window.__outlToolbar(action)`
 * (`OutlToolbar.swift`) and what `Journal.tsx`'s dispatch switches on.
 * The iOS side keeps its own Swift copy (`OutlKit/Toolbar/ToolbarAction.swift`)
 * because the native accessory bar renders before the webview loads;
 * **renaming a case here means renaming it there in the same commit**,
 * or the native bar silently stops working.
 *
 * The Android bar (and, later, iOS once the web bar is device-validated)
 * renders straight from this catalog, so there is no Kotlin copy to
 * drift.
 */

/** Every action the toolbar can emit. The string values are the wire
 *  contract with the iOS native bar + the `Journal.tsx` dispatcher. */
export type ToolbarAction =
  | "newLine"
  | "indent"
  | "outdent"
  | "insertRef"
  | "todo"
  | "bold"
  | "italic"
  | "moveUp"
  | "moveDown"
  | "undo"
  | "redo"
  | "insertHash"
  | "insertBlock"
  | "code"
  | "delete"
  | "done";

/** How a button paints: an icon (`symbol`) or a literal glyph
 *  (`text`, e.g. `[[`). The concrete SVG for a `symbol` lives in the
 *  client chrome (`KeyboardToolbar.tsx`) — this only records the kind
 *  and the semantic name so the catalog stays presentation-agnostic. */
export type ToolbarStyle =
  | { kind: "symbol"; symbol: string; destructive?: boolean }
  | { kind: "text"; glyph: string };

export interface ToolbarActionMeta {
  readonly label: string;
  readonly style: ToolbarStyle;
}

/** Per-action presentation metadata. Mirrors `OutlToolbarView.metadata`
 *  in `OutlToolbar.swift`; the `symbol` names are the SF Symbol ids the
 *  iOS bar uses, reused here as stable semantic keys the web icon map
 *  switches on. */
export const TOOLBAR_META: Readonly<Record<ToolbarAction, ToolbarActionMeta>> = {
  newLine: { label: "New line", style: { kind: "symbol", symbol: "plus" } },
  indent: { label: "Indent", style: { kind: "symbol", symbol: "increase.indent" } },
  outdent: { label: "Outdent", style: { kind: "symbol", symbol: "decrease.indent" } },
  moveUp: { label: "Move up", style: { kind: "symbol", symbol: "arrow.up" } },
  moveDown: { label: "Move down", style: { kind: "symbol", symbol: "arrow.down" } },
  undo: { label: "Undo", style: { kind: "symbol", symbol: "arrow.uturn.left" } },
  redo: { label: "Redo", style: { kind: "symbol", symbol: "arrow.uturn.right" } },
  bold: { label: "Bold", style: { kind: "symbol", symbol: "bold" } },
  italic: { label: "Italic", style: { kind: "symbol", symbol: "italic" } },
  code: { label: "Code", style: { kind: "symbol", symbol: "code" } },
  insertRef: { label: "Insert reference", style: { kind: "text", glyph: "[[" } },
  insertBlock: { label: "Insert block ref", style: { kind: "text", glyph: "((" } },
  insertHash: { label: "Insert hashtag", style: { kind: "text", glyph: "#" } },
  todo: { label: "Toggle TODO", style: { kind: "symbol", symbol: "checkmark.circle" } },
  delete: { label: "Delete block", style: { kind: "symbol", symbol: "trash", destructive: true } },
  done: { label: "Hide keyboard", style: { kind: "symbol", symbol: "keyboard.down" } },
};

/**
 * Cold-start order. Used until MFU has enough taps to take over, and as
 * the deterministic tiebreak for actions with equal counts. Mirrors
 * `ToolbarAction.defaultOrder` in Swift.
 */
export const DEFAULT_ORDER: readonly ToolbarAction[] = [
  "newLine",
  "indent",
  "outdent",
  "insertRef",
  "todo",
  "bold",
  "italic",
  "moveUp",
  "moveDown",
  "undo",
  "redo",
  "insertHash",
  "insertBlock",
  "code",
  "delete",
  "done",
];

/** Always sits at index 0 — creating a new block is the outliner's
 *  primary act, worth one thumb-tap regardless of MFU stats. */
export const PINNED_FIRST: ToolbarAction = "newLine";

/** Always sits last — "hide keyboard" lives where muscle memory
 *  expects "Done". */
export const PINNED_LAST: ToolbarAction = "done";
