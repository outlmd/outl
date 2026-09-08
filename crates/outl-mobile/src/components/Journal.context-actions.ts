/**
 * Long-press context-menu action list for a block.
 *
 * Extracted from `Journal.tsx` (which was 3,212 lines) because none of
 * this is component code: `buildContextActions` is a pure function from
 * (block id, page view, handlers) to a typed row list, and
 * `locateSiblings` is its only helper. Nothing here touches Solid,
 * reactivity, or a Tauri command — the handlers arrive from the
 * caller's scope.
 *
 * `buildContextActions` was already exported so
 * `Journal.buildContextActions.test.ts` could drive it without mounting
 * the whole component; the export is now honest about where it lives.
 */
import { detectFence } from "@outl/shared/highlight";
import { findBlock } from "@outl/shared/outline";
import type { BlockContextAction } from "./BlockContextMenu";

/**
 * Wire the long-press block id into a typed action list for
 * `<BlockContextMenu>`. Each action carries an SVG path, label, and
 * a guard (`enabled`) so we hide "Move up" on the first sibling and
 * "Move down" on the last — gestures iOS users expect to disappear
 * when they don't apply.
 *
 * The handlers are passed in from `Journal()`'s scope so the menu
 * doesn't have to import every Tauri command directly.
 *
 * Exported (only) so `Journal.buildContextActions.test.ts` can drive
 * it directly — mounting the whole `<Journal>` component just to
 * assert which context-menu rows appear would need a full Tauri
 * command mock surface for no extra coverage.
 */
export function buildContextActions(
  blockId: string | null,
  pageView: import("@outl/shared/api/types").PageView | null,
  handlers: {
    indent: (id: string) => void;
    outdent: (id: string) => void;
    moveUp: (id: string) => void;
    moveDown: (id: string) => void;
    toggleTodo: (id: string) => void;
    delete: (id: string) => void;
    runCode: (id: string) => void;
    insertTemplate: (id: string) => void;
    properties: (id: string) => void;
    remindMe: (id: string) => void;
    copy: (id: string) => void;
    copyBlock: (id: string) => void;
    pasteBlock: (id: string) => void;
    /** RFC 0254 phase 4b — cut `id`'s subtree into the block
     *  clipboard, deleting it from the source. */
    cutBlock: (id: string) => void;
    /** RFC 0254 phase 4b (issue #18) — copy `id`'s `((blk-XXXXXX))`
     *  ref handle to the OS clipboard. */
    copyBlockRef: (id: string) => void;
    /** RFC 0254 phase 4b — create a new sibling immediately above
     *  `id` and start editing it. */
    newBlockAbove: (id: string) => void;
    attachFile: (id: string) => void;
    /** RFC 0254 phase 3 — start a range selection anchored at `id`. */
    selectBlocks: (id: string) => void;
    /** RFC 0254 phase 3 — reselect the range captured on the last
     *  exit (vim `gv`). Takes no id: the range it restores carries
     *  its own anchor/cursor, independent of which block's menu the
     *  user opened to reach it. */
    reselectSelection: () => void;
  },
  /** Is the block clipboard armed (`blockClipboard() !== null`)? Passed
   *  in rather than read from a `handlers` closure so the caller's own
   *  signal read stays inside its `actions=` prop-getter evaluation —
   *  see the call site's comment for why that's what makes this
   *  reactive. */
  canPasteBlock = false,
  /** Does `lastSelection` still resolve against the live outline? Same
   *  "read it at the call site" reactivity reasoning as
   *  `canPasteBlock`. */
  canReselect = false,
): BlockContextAction[] {
  if (!blockId || !pageView) return [];
  // Resolve sibling position so we can hide move-up/down at the
  // ends. Walking the outline is cheap (the user just long-pressed,
  // there's no per-frame budget here).
  const siblings = locateSiblings(pageView.outline, blockId);
  const index = siblings
    ? siblings.findIndex((b) => b.id === blockId)
    : -1;
  const canMoveUp = index > 0;
  const canMoveDown = siblings ? index < siblings.length - 1 : false;
  // `Run code` only shows up when the long-pressed block is a fenced
  // `` ```lang …``` `` AND the fence language is one we actually ship
  // a runtime for. The backend re-validates via `run_block_at_index`
  // (`UnknownLanguage` error path), so this is a UX guard — a long
  // press on a `swift`/`shell`/`ruby` fence shouldn't offer a "Run"
  // button that then errors out, and the narrower set is also
  // cleaner to defend against App Review 2.5.2 if the reviewer
  // browses the contextual menu.
  // Stays in sync with the `outl-exec` features enabled for the
  // mobile IPA (`crates/outl-mobile/src-tauri/Cargo.toml`).
  const block = findBlock(pageView.outline, blockId);
  const fence = block ? detectFence(block.text) : null;
  const fenceLang = fence?.language.toLowerCase() ?? "";
  const canRun =
    fence &&
    (fenceLang === "lisp" ||
      fenceLang === "js" ||
      fenceLang === "javascript" ||
      fenceLang === "node" ||
      fenceLang === "py" ||
      fenceLang === "python" ||
      fenceLang === "lua");
  return [
    ...(canRun && fence
      ? [
          {
            id: "runCode",
            label: `Run ${fence.language}`,
            // SF-Symbols-equivalent "play.fill" — filled right
            // triangle, matches the desktop's `▶ Run` chip.
            iconPath: "M8 5v14l11-7z",
            onSelect: () => handlers.runCode(blockId),
          } satisfies BlockContextAction,
        ]
      : []),
    {
      id: "toggleTodo",
      label: "Toggle TODO",
      iconPath: "M5 12l4 4 10-10",
      onSelect: () => handlers.toggleTodo(blockId),
    },
    // Touch-native range selection (RFC 0254 phase 3) — the one
    // interaction this phase invents. Long-press already opens this
    // menu for every other single-block action, so "start selecting
    // here" is a row in the same sheet rather than a second gesture
    // competing with long-press-for-menu and swipe-for-delete.
    {
      id: "selectBlocks",
      label: "Select blocks",
      // Checklist glyph — three ticked rows, reads as "act on more
      // than one block".
      iconPath:
        "M9 6h11M9 12h11M9 18h11M4 6l1.5 1.5L8 5M4 12l1.5 1.5L8 10M4 18l1.5 1.5L8 16",
      onSelect: () => handlers.selectBlocks(blockId),
    },
    ...(canReselect
      ? [
          {
            id: "reselectSelection",
            label: "Reselect last selection",
            // Circular-arrow "restore" glyph.
            iconPath:
              "M3 12a9 9 0 1 1 3 6.7 M3 12v5 M3 17h5",
            onSelect: () => handlers.reselectSelection(),
          } satisfies BlockContextAction,
        ]
      : []),
    {
      id: "copy",
      label: "Copy text",
      iconPath:
        "M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2 M9 2h6a1 1 0 0 1 1 1v2a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1z",
      onSelect: () => handlers.copy(blockId),
    },
    // Block clipboard (RFC 0254 phase 2, cut added phase 4b), distinct
    // from "Copy text" above: that one writes to the OS clipboard for
    // pasting outside outl; this trio arms an in-app buffer for
    // duplicating (or, for cut, relocating) the block + subtree
    // elsewhere in this workspace, fresh ids on paste — mirrors the
    // desktop's `Cmd/Ctrl+X` / `Cmd/Ctrl+C` / `Cmd/Ctrl+V` (`CutBlock` /
    // `CopyBlock` / `PasteBlock`).
    {
      id: "cutBlock",
      label: "Cut block",
      // Scissors glyph.
      iconPath:
        "M6 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M20 4L8.5 15.5 M14.5 14.5L20 20 M8.5 8.5L10 10",
      onSelect: () => handlers.cutBlock(blockId),
    },
    {
      id: "copyBlock",
      label: "Copy block",
      // Two overlapping rectangles — the "duplicate" glyph, distinct
      // from "Copy text"'s single-document icon above.
      iconPath:
        "M9 9h10v10H9z M5 15V5a2 2 0 0 1 2-2h10",
      onSelect: () => handlers.copyBlock(blockId),
    },
    ...(canPasteBlock
      ? [
          {
            id: "pasteBlock",
            label: "Paste block",
            // Clipboard glyph.
            iconPath:
              "M9 5h6a1 1 0 0 1 1 1v1H8V6a1 1 0 0 1 1-1z M8 4h8a2 2 0 0 1 2 2v13a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z",
            onSelect: () => handlers.pasteBlock(blockId),
          } satisfies BlockContextAction,
        ]
      : []),
    {
      id: "copyBlockRef",
      label: "Copy block ref",
      // Link/chain glyph — reads as "copy a reference", distinct from
      // both the document (Copy text) and duplicate (Copy block) icons.
      iconPath:
        "M9 12a3 3 0 0 0 4.24 0l3-3a3 3 0 0 0-4.24-4.24l-1 1 M15 12a3 3 0 0 0-4.24 0l-3 3a3 3 0 0 0 4.24 4.24l1-1",
      onSelect: () => handlers.copyBlockRef(blockId),
    },
    {
      id: "newBlockAbove",
      label: "New block above",
      // Plus above a horizontal rule — reads as "insert before".
      iconPath: "M12 4v8 M8 8h8 M4 20h16",
      onSelect: () => handlers.newBlockAbove(blockId),
    },
    {
      id: "remindMe",
      label: "Remind me…",
      // "bell" — the reminders affordance, same glyph family as the
      // header button that opens the list.
      iconPath:
        "M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9 M13.73 21a2 2 0 0 1-3.46 0",
      onSelect: () => handlers.remindMe(blockId),
    },
    {
      id: "properties",
      label: "Properties…",
      // "tag" glyph — a `key:: value` is the block's metadata, and the
      // sheet behind it is the only GUI place to create one.
      iconPath:
        "M20.6 13.4l-7.2 7.2a2 2 0 0 1-2.8 0l-7.2-7.2a2 2 0 0 1-.6-1.4V4a1 1 0 0 1 1-1h8a2 2 0 0 1 1.4.6l7.4 7.4a2 2 0 0 1 0 2.8z M7.5 7.5h.01",
      onSelect: () => handlers.properties(blockId),
    },
    {
      id: "insertTemplate",
      label: "Insert template",
      // "doc.on.doc"-style stacked pages — reads as "stamp a template".
      iconPath:
        "M9 3H5a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h4 M15 7h4a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2h-8a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2z",
      onSelect: () => handlers.insertTemplate(blockId),
    },
    {
      id: "attachFile",
      label: "Attach file",
      // "paperclip" — reads as "attach an uploaded file".
      iconPath:
        "M21 11.5l-9 9a5 5 0 0 1-7-7l9-9a3.5 3.5 0 0 1 5 5l-9 9a2 2 0 0 1-3-3l8-8",
      onSelect: () => handlers.attachFile(blockId),
    },
    {
      id: "indent",
      label: "Indent",
      iconPath: "M3 5h12M3 12h8M3 19h12M15 9l3 3-3 3",
      onSelect: () => handlers.indent(blockId),
    },
    {
      id: "outdent",
      label: "Outdent",
      iconPath: "M3 5h12M3 12h8M3 19h12M21 9l-3 3 3 3",
      onSelect: () => handlers.outdent(blockId),
    },
    {
      id: "moveUp",
      label: "Move up",
      iconPath: "M12 19V5M5 12l7-7 7 7",
      enabled: () => canMoveUp,
      onSelect: () => handlers.moveUp(blockId),
    },
    {
      id: "moveDown",
      label: "Move down",
      iconPath: "M12 5v14M19 12l-7 7-7-7",
      enabled: () => canMoveDown,
      onSelect: () => handlers.moveDown(blockId),
    },
    {
      id: "delete",
      label: "Delete",
      iconPath:
        "M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m-9 0v14a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2V6",
      destructive: true,
      onSelect: () => handlers.delete(blockId),
    },
  ];
}

/** DFS for the sibling list containing `targetId`. Returns the
 *  block array (not the parent) so the caller can use `findIndex`
 *  without an extra walk. */
function locateSiblings(
  forest: import("@outl/shared/api/types").BlockNode[],
  targetId: string,
): import("@outl/shared/api/types").BlockNode[] | null {
  for (const node of forest) {
    if (node.id === targetId) return forest;
    const inner = locateSiblings(node.children ?? [], targetId);
    if (inner) return inner;
  }
  return null;
}
