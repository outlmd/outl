import { isToolbarAction, recordToStore } from "@outl/shared/toolbar";

/**
 * What a toolbar button does, supplied by `Journal`.
 *
 * Every handler that needs the block being edited takes it as an
 * argument rather than reading a signal, so this module holds no view
 * state and can be driven straight from a test.
 */
export interface ToolbarHandlers {
  /** The block currently in edit mode, or `null`. */
  editingId: () => string | null;
  indent: (id: string) => void;
  outdent: (id: string) => void;
  moveUp: (id: string) => void;
  moveDown: (id: string) => void;
  undo: () => void;
  redo: () => void;
  toggleTodo: (id: string) => void;
  delete: (id: string) => void;
  createAfter: (id: string) => void;
  appendBlock: () => void;
  wrapSelection: (kind: "bold" | "italic" | "code") => void;
  insertPair: (open: string, close: string) => void;
  insertText: (text: string) => void;
  commitEdit: () => void;
}

/**
 * Single dispatch for a toolbar action, shared by the two surfaces that
 * fire them: the iOS native bar (via `window.__outlToolbar`) and the web
 * `<KeyboardAccessory />` (Android). One switch means the two bars can't
 * drift on what a button does.
 *
 * It is also the single place a tap is **counted**. The counts feed the
 * MFU order *and* the settings sheet's Lock / Reset, and that sheet is
 * web on both platforms — so counting inside the native bar (which is
 * what Swift's `ToolbarMFU.record` used to do, into `UserDefaults`) left
 * the sheet reading an always-empty store: "Lock button order" froze a
 * cold-start row instead of the user's, and "Reset button order" did
 * nothing. Counting here puts it on the path both bars already share.
 * Full reasoning: `docs/mobile-ux.md` → Keyboard accessory bar.
 *
 * `action` arrives as a bare string because the iOS bridge sends one;
 * an id outside the catalog is ignored rather than recorded, or it
 * would sit in the counts store forever.
 */
export function dispatchToolbarAction(
  action: string,
  h: ToolbarHandlers,
): void {
  if (isToolbarAction(action)) recordToStore(action);
  const id = h.editingId();
  switch (action) {
    case "indent":
      if (id) h.indent(id);
      return;
    case "outdent":
      if (id) h.outdent(id);
      return;
    case "moveUp":
      if (id) h.moveUp(id);
      return;
    case "moveDown":
      if (id) h.moveDown(id);
      return;
    case "undo":
      h.undo();
      return;
    case "redo":
      h.redo();
      return;
    case "todo":
      if (id) h.toggleTodo(id);
      return;
    case "delete":
      if (id) h.delete(id);
      return;
    case "newLine":
      if (id) h.createAfter(id);
      else h.appendBlock();
      return;
    case "bold":
    case "italic":
    case "code":
      h.wrapSelection(action);
      return;
    case "insertRef":
      h.insertPair("[[", "]]");
      return;
    case "insertBlock":
      h.insertPair("((", "))");
      return;
    case "insertHash":
      h.insertText("#");
      return;
    case "done":
      if (id) h.commitEdit();
      return;
  }
}
