/**
 * Concrete `Action -> handler` map for the desktop client.
 *
 * Built once in `<AppShell />` and handed to `installShortcuts`.
 * Each entry maps an `outl_shortcuts::Action` variant to the Tauri
 * call (or store mutation) that materialises it on the desktop.
 *
 * The map mirrors the TUI's vim bindings 1:1 — `j/k` move the
 * selection, `i/o/O` enter Insert, `Tab/Shift-Tab` indent /
 * outdent, `dd` deletes, `c` folds, `Cmd+T` cycles TODO — so a
 * user moving between clients keeps their muscle memory.
 *
 * The selection cursor lives in `appState.selectedBlockId`;
 * `<BlockRow />` highlights it. Auto-selection of the first block
 * runs from `<OutlineView />` so the dispatcher never has to worry
 * about a `null` cursor.
 */

import { getCurrentWindow } from "@tauri-apps/api/window";

import {
  copyBlockMarkdown,
  copyMarkdown,
  createBlock,
  deleteBlock,
  deletePage,
  editBlock,
  indentBlock,
  moveBlockAfter,
  moveBlockDown,
  moveBlockUp,
  nextDay,
  openJournalFor,
  openRef,
  openTodayJournal,
  outdentBlock,
  pasteBlockAfter,
  pluginSyncHooks,
  previousDay,
  runCodeBlock,
  setBlockCollapsed,
  setBlockRemind,
  snoozeReminder,
  todaySlug,
  toggleTodo as toggleTodoCmd,
} from "@outl/shared/api/commands";
import type { BlockNode, PageView } from "@outl/shared/api/types";
import {
  flattenAll,
  flattenParents,
  flattenVisible,
  focusSubtree,
  nextVisibleId,
  previousVisibleId,
  visualRangeIds,
} from "@outl/shared/outline";

import { redoPage, undoPage } from "./api";
import { playPluginViews } from "./plugin-views";
import { cycleTodoInDraft, insertLink, wrapSelection } from "./markdown-wrap";
import { type ActionHandlers } from "./shortcuts";
import { appState, setAppState } from "./store";

export interface DesktopHandlerDeps {
  applyView: (view: PageView) => void;
  setError: (msg: string) => void;
}

/**
 * Build the desktop's handler map. Closes over the `applyView` /
 * `setError` callbacks from the shell so each action can refresh
 * the rendered page or surface a problem in the status bar.
 */
export function buildHandlers(deps: DesktopHandlerDeps): ActionHandlers {
  /** Ask the selected block's row for a blank property editor.
   *  Refusing loudly matters: a chord that silently does nothing when
   *  no block is selected gets reported as "the shortcut is broken". */
  const openPropertyEditor = () => {
    const block = appState.selectedBlockId;
    if (!block) {
      deps.setError("select a block first");
      return;
    }
    setAppState("addPropertyBlockId", block);
  };

  const safeCall = async <T>(p: Promise<T>): Promise<T | undefined> => {
    try {
      return await p;
    } catch (e) {
      deps.setError(e instanceof Error ? e.message : String(e));
      return undefined;
    }
  };

  /** Resolve the block the user means right now — the focused
   *  textarea's id (Insert) takes priority over the selection
   *  cursor (Normal). Returns `null` when neither is set. */
  function targetBlockId(): string | null {
    const el = document.activeElement;
    if (el instanceof HTMLTextAreaElement && el.dataset.blockId) {
      return el.dataset.blockId;
    }
    return appState.selectedBlockId;
  }

  /** Block ids of each visible backlink, in render order. The
   *  selection cursor traverses this list with `j/k` when the
   *  backlinks section is reachable. */
  function backlinkBlockIds(): string[] {
    if (!appState.backlinksOpen) return [];
    return appState.backlinks.map((b) => b.block_id);
  }

  /** Blocks the selection cursor is allowed to traverse. When zoomed
   *  (`focusBlockId` set and still present), the focused block is the
   *  header title, so the cursor traverses its **children** (the visible
   *  body) — matching what `<OutlineView />` renders. Otherwise the
   *  whole page. A stale focus falls back to the full outline. */
  function navBlocks(): BlockNode[] {
    const id = appState.focusBlockId;
    if (!id) return appState.outline;
    const fv = focusSubtree(appState.outline, id);
    return fv ? fv.root.children : appState.outline;
  }

  /** Find a block in the outline by id; deep search. */
  function lookupBlock(id: string): BlockNode | null {
    const walk = (bs: BlockNode[]): BlockNode | null => {
      for (const b of bs) {
        if (b.id === id) return b;
        if (b.children.length > 0) {
          const hit = walk(b.children);
          if (hit) return hit;
        }
      }
      return null;
    };
    return walk(appState.outline);
  }

  /** Serialize a block selection to clean outl markdown (backend) and
   *  write it to the OS clipboard. Both halves are best-effort: the
   *  webview's `navigator.clipboard` can reject (no focus / permission),
   *  and a copy never blocks the yank register that `p`/`P` reads. */
  async function copySelectionToClipboard(blockIds: string[]): Promise<void> {
    if (blockIds.length === 0) return;
    const md = await safeCall(copyMarkdown(blockIds));
    if (md == null) return;
    void navigator.clipboard?.writeText(md).catch(() => {});
  }

  /** Common pattern: await a Tauri command that returns a
   *  `PageView`, apply it, and leave selection on `nextSelectedId`
   *  (or keep current when `undefined`). */
  async function runOn(cmd: Promise<PageView>, nextSelectedId?: string | null) {
    const view = await safeCall(cmd);
    if (!view) return;
    deps.applyView(view);
    if (nextSelectedId !== undefined) {
      setAppState("selectedBlockId", nextSelectedId);
    }
  }

  /** Walk every block in the current Visual range and fire `op` for
   *  each. Used by `>` / `<` (indent) and `Cmd/Ctrl+Shift+↑/↓` (move)
   *  so the multi-block ops share one body. Range stays selected after
   *  the op (vim convention) so the user can repeat it — the anchor /
   *  cursor are block ids, stable across the re-render.
   *
   *  `reverse` walks bottom-up: a move-**down** has to shift the last
   *  block past the block below the range first, or an ascending walk
   *  would drag each block over its own not-yet-moved neighbour. Indent
   *  / move-up keep the default top-down order. */
  async function applyVisualBlockOp(
    op: (pageId: string, id: string) => Promise<PageView>,
    reverse = false,
  ) {
    const pageId = appState.page?.id;
    if (!pageId) return;
    const range = visualRangeIds(
      appState.visualAnchorId,
      appState.selectedBlockId,
      appState.outline,
    );
    if (!range) return;
    const ids = flattenVisible(appState.outline);
    const loIdx = ids.indexOf(range.lo);
    const hiIdx = ids.indexOf(range.hi);
    const targets = ids.slice(loIdx, hiIdx + 1);
    if (reverse) targets.reverse();
    let lastView: PageView | undefined;
    for (const id of targets) {
      const view = await safeCall(op(pageId, id));
      if (view) lastView = view;
    }
    if (lastView) deps.applyView(lastView);
  }

  /** Enter (from Normal) or extend (already in Visual) a contiguous
   *  block selection by one row, `dir` = +1 down / -1 up. The non-vim
   *  multi-select path: `Shift+↓` / `Shift+↑` anchor at the current
   *  block and grow the range. Stays inside the outline — unlike plain
   *  `j` / `k`, it never crosses into the backlinks section, because a
   *  batch op has nothing to do with a read-only backlink row. */
  function extendVisualRange(dir: 1 | -1) {
    if (appState.mode !== "vim-visual") {
      const id = appState.selectedBlockId;
      if (!id) return;
      setAppState("visualAnchorId", id);
      setAppState("mode", "vim-visual");
    }
    const cur = appState.selectedBlockId;
    if (!cur) return;
    const next =
      dir === 1
        ? nextVisibleId(cur, appState.outline)
        : previousVisibleId(cur, appState.outline);
    if (next) setAppState("selectedBlockId", next);
  }

  /** Capture the current range for `g v`, drop the anchor, and land
   *  back in Normal. Shared by every Visual exit (`Esc`, `y`, `d`, the
   *  toolbar's Done). */
  function exitVisual() {
    const range = visualRangeIds(
      appState.visualAnchorId,
      appState.selectedBlockId,
      appState.outline,
    );
    if (range) setAppState("lastVisualRange", range);
    setAppState("visualAnchorId", null);
    setAppState("mode", "vim-normal");
  }

  /** Walk every block on the current page and set its `collapsed`
   *  flag to `value`. **Never `flattenVisible`** — the whole point of
   *  `zR` is to expand subtrees currently hidden under a collapsed
   *  parent, and the visible-only walk would silently no-op on every
   *  descendant of a folded node.
   *
   *  `zM` (value=true) uses `flattenParents` so leaves are skipped:
   *  folding a leaf is invisible today, but `outl_actions::set_block_collapsed`
   *  always writes `Op::SetCollapsed` to the log (a CRDT contract:
   *  every flip lands so concurrent flips converge via HLC), so when
   *  the user later adds children underneath they appear collapsed —
   *  future-surprise. `zR` (value=false) uses `flattenAll`: descolapsar
   *  leaf não tem efeito futuro e mantém a contagem de ops simétrica.
   *  Mirror exato do `collect_collapse_candidates` no TUI. */
  async function applyCollapsedToAll(value: boolean) {
    const pageId = appState.page?.id;
    if (!pageId) return;
    const ids = value
      ? flattenParents(appState.outline)
      : flattenAll(appState.outline);
    let lastView: PageView | undefined;
    for (const id of ids) {
      const view = await safeCall(setBlockCollapsed(pageId, id, value));
      if (view) lastView = view;
    }
    if (lastView) deps.applyView(lastView);
  }

  /** Pre-fill the picker with the selected block's text, then open
   *  it. Powers `*` / `#` — vim's "search the word under cursor"
   *  collapses to "search for something inside this block" on the
   *  desktop because Normal mode has no character cursor.
   *
   *  We can't poke the picker's input ref from here (it lives inside
   *  `<Picker />`), so we stash the seed in a transient store field
   *  the picker reads on open. */
  function seedPickerWithCurrentBlock() {
    const id = appState.selectedBlockId;
    if (!id) return;
    const block = lookupBlock(id);
    if (!block) return;
    const seed = block.text.trim().split(/\s+/).slice(0, 4).join(" ");
    setAppState("pickerSeed", seed);
    setAppState("pickerOpen", true);
  }

  /** Shared status-line nudge fired by every char-cursor vim op
   *  (`x` `X` `D` `C` `s` `r` `~` `e` `f` `F`). Desktop Normal mode
   *  has only a selected block id, not a character cursor inside the
   *  block, so these ops can't act locally — the user has to enter
   *  Insert (`i`) and edit inside the textarea. One handler shared
   *  by all 10 entries so the message stays in lockstep across the
   *  catalog. */
  function charCursorNudge() {
    deps.setError(
      "char-cursor ops (x/X/D/C/s/r/~/e/f/F) — use `i` and edit inside the textarea on the desktop",
    );
  }

  return {
    // ── chrome ────────────────────────────────────────────────────
    OpenPicker: () => {
      setAppState("pickerOpen", !appState.pickerOpen);
    },
    OpenSettings: () => {
      setAppState("settingsOpen", !appState.settingsOpen);
    },
    ToggleSidebar: () => {
      setAppState("sidebarOpen", !appState.sidebarOpen);
    },
    ToggleBacklinks: () => {
      setAppState("backlinksOpen", !appState.backlinksOpen);
    },
    ToggleHelp: () => {
      setAppState("helpOpen", !appState.helpOpen);
    },

    // ── reminders (`remind::`) ────────────────────────────────────
    //
    // Authoring writes a block property; the schedule is derived from
    // it on every scan in Rust, so editing the rule reschedules for
    // free and there is nothing to invalidate here.
    OpenReminders: () => {
      setAppState("remindersOpen", !appState.remindersOpen);
    },
    InsertRemind: async () => {
      const page = appState.page?.id;
      const block = appState.selectedBlockId;
      if (!page || !block) return;
      // A bare anchor is the smallest rule that does something: one
      // fire, today, at the time the user is about to type over.
      const view = await safeCall(setBlockRemind(page, block, "9am"));
      if (view) deps.applyView(view);
    },
    InsertRemindNag: async () => {
      const page = appState.page?.id;
      const block = appState.selectedBlockId;
      if (!page || !block) return;
      // Same preset as the TUI's `NAG_PRESET`
      // (`outl-tui/src/actions/reminders.rs`).
      const view = await safeCall(
        setBlockRemind(page, block, "now every 1h until DONE"),
      );
      if (view) deps.applyView(view);
    },
    SnoozeReminder: async () => {
      const block = appState.selectedBlockId;
      if (!block) return;
      // The preset id, not a duration — the backend resolves it, so
      // `g s` lands on the same instant as the panel's first button.
      await safeCall(snoozeReminder(block, "1h"));
    },
    // ── properties (`key:: value`) ────────────────────────────────
    //
    // The chord has no chip to click, so it names the block and
    // `<BlockRow />` opens a blank editor on it. The editor clears the
    // signal when it closes, which is what lets a second press
    // re-open it on the same block.
    //
    // `OpenProperties` (the TUI's `g p`) resolves here too: that
    // overlay's job is to *list* a block's properties, and the desktop
    // already renders that list permanently as the chip row. The only
    // half it lacks is the blank pair, so both chords open it.
    AddProperty: openPropertyEditor,
    OpenProperties: openPropertyEditor,
    Quit: async () => {
      // `qq` chord in Normal + `Ctrl+C` Global. Close the active
      // window; Tauri tears the app down once the last window
      // closes.
      try {
        await getCurrentWindow().close();
      } catch (e) {
        deps.setError(e instanceof Error ? e.message : String(e));
      }
    },

    // ── page-level navigation ────────────────────────────────────
    OpenToday: async () => {
      const view = await safeCall(openTodayJournal());
      if (view) deps.applyView(view);
    },
    PrevDay: async () => {
      const anchor =
        appState.page?.kind === "journal"
          ? appState.page.slug
          : await safeCall(todaySlug());
      if (!anchor) return;
      const slug = await safeCall(previousDay(anchor));
      if (!slug) return;
      const view = await safeCall(openJournalFor(slug));
      if (view) deps.applyView(view);
    },
    NextDay: async () => {
      const anchor =
        appState.page?.kind === "journal"
          ? appState.page.slug
          : await safeCall(todaySlug());
      if (!anchor) return;
      const slug = await safeCall(nextDay(anchor));
      if (!slug) return;
      const view = await safeCall(openJournalFor(slug));
      if (view) deps.applyView(view);
    },

    // ── selection (vim j/k/↑/↓) ──────────────────────────────────
    //
    // The cursor spans two regions: the outline blocks (top) and
    // the inline backlinks (bottom). Only one of
    // `selectedBlockId` / `selectedBacklinkBlockId` is non-null at
    // any time so navigation stays a single linear motion.
    //
    // Backlinks are only reachable when the section is visible
    // (`backlinksOpen` + the page has at least one ref); otherwise
    // `j` past the last outline block clamps at the bottom — same
    // behaviour the TUI ships when `show_backlinks == false`.
    SelectionDown: () => {
      const backlinkIds = backlinkBlockIds();
      const curBacklink = appState.selectedBacklinkBlockId;
      if (curBacklink) {
        const idx = backlinkIds.indexOf(curBacklink);
        if (idx >= 0 && idx < backlinkIds.length - 1) {
          setAppState("selectedBacklinkBlockId", backlinkIds[idx + 1]);
        }
        // last backlink → stay (no wrap into outline)
        return;
      }
      const cur = appState.selectedBlockId;
      const nav = navBlocks();
      const list = flattenVisible(nav);
      const next = nextVisibleId(cur, nav);
      const atLast = cur !== null && next === cur;
      if (atLast && backlinkIds.length > 0) {
        // Cross into the backlinks section.
        setAppState("selectedBlockId", null);
        setAppState("selectedBacklinkBlockId", backlinkIds[0]);
        return;
      }
      console.info(
        `[shortcuts] SelectionDown cur=${cur} → ${next} (idx ${list.indexOf(cur ?? "")} → ${list.indexOf(next ?? "")} of ${list.length})`,
      );
      if (next) setAppState("selectedBlockId", next);
    },
    SelectionUp: () => {
      const backlinkIds = backlinkBlockIds();
      const curBacklink = appState.selectedBacklinkBlockId;
      if (curBacklink) {
        const idx = backlinkIds.indexOf(curBacklink);
        if (idx > 0) {
          setAppState("selectedBacklinkBlockId", backlinkIds[idx - 1]);
          return;
        }
        // First backlink → step back into the outline's last
        // visible block.
        const list = flattenVisible(navBlocks());
        if (list.length > 0) {
          setAppState("selectedBacklinkBlockId", null);
          setAppState("selectedBlockId", list[list.length - 1]);
        }
        return;
      }
      const cur = appState.selectedBlockId;
      const nav = navBlocks();
      const list = flattenVisible(nav);
      const prev = previousVisibleId(cur, nav);
      console.info(
        `[shortcuts] SelectionUp cur=${cur} → ${prev} (idx ${list.indexOf(cur ?? "")} → ${list.indexOf(prev ?? "")} of ${list.length})`,
      );
      if (prev) setAppState("selectedBlockId", prev);
    },

    // ── enter Insert (i / Shift+I / a / A / Enter) ───────────────
    EnterInsert: () => {
      const id = appState.selectedBlockId;
      if (!id) return;
      setAppState("editingBlockId", id);
    },
    EnterInsertAtStart: () => {
      const id = appState.selectedBlockId;
      if (!id) return;
      setAppState("editingBlockId", id);
    },
    // `a` — vim append. On the desktop without a char cursor in
    // Normal mode this collapses to `EnterInsert` (the textarea's
    // own click-or-keyboard caret lands inside the buffer). The
    // catalog entry stays so muscle memory matches.
    EnterInsertAfter: () => {
      const id = appState.selectedBlockId;
      if (!id) return;
      setAppState("editingBlockId", id);
    },
    // `A` — Insert with caret jumped to end of block. We can't poke
    // the textarea from here because `<Show>` hasn't necessarily
    // mounted it yet; instead we hand the row a `caretIntent: "end"`
    // signal and the row's own `createEffect` applies it the moment
    // the ref is populated. See `store.ts` → `caretIntent` for the
    // reasoning (racey microtask vs Solid render pipeline).
    EnterInsertAtEnd: () => {
      const id = appState.selectedBlockId;
      if (!id) return;
      setAppState("caretIntent", "end");
      setAppState("editingBlockId", id);
    },
    // `S` — clear the block's text and enter Insert at column 0.
    // Vim's "substitute line" / outline's "rewrite this block".
    SubstituteBlock: async () => {
      const pageId = appState.page?.id;
      const id = targetBlockId();
      if (!pageId || !id) return;
      const view = await safeCall(editBlock(pageId, id, ""));
      if (view) deps.applyView(view);
      setAppState("editingBlockId", id);
    },
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
      void copySelectionToClipboard([id]);
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
    OpenRefUnderCursor: async () => {
      // Normal-mode `Enter`. Two branches in priority order:
      //
      // 1. Selection is on a backlink row → open the source page and
      //    snap the selection to the referencing block on that page
      //    (backlink rows are read-only, so "open" is the only
      //    meaningful gesture there).
      // 2. Otherwise → enter Insert on the selected block.
      //
      // Unlike the TUI, the desktop's Normal mode has no character
      // cursor — only a selected block — so "the ref under the
      // cursor" cannot be resolved. An earlier version approximated
      // it as "the first `[[ref]]` in the block", which made every
      // ref-carrying block impossible to edit via Enter. Following
      // a ref on the desktop is the click on the token
      // (`onRefClick` in OutlineView).
      const curBacklink = appState.selectedBacklinkBlockId;
      if (curBacklink) {
        const link = appState.backlinks.find((b) => b.block_id === curBacklink);
        const target = link?.source_page?.slug;
        if (!target) return;
        const view = await safeCall(openRef(target));
        if (!view) return;
        deps.applyView(view);
        // Reset cursor to the source block on the freshly-opened
        // page so the user lands exactly where the ref lives.
        setAppState("selectedBacklinkBlockId", null);
        setAppState("selectedBlockId", curBacklink);
        return;
      }
      const id = appState.selectedBlockId;
      if (!id) return;
      setAppState("editingBlockId", id);
    },

    // ── create siblings (o / O) ──────────────────────────────────
    NewBlockBelow: async () => {
      const pageId = appState.page?.id;
      const after = appState.selectedBlockId;
      if (!pageId) return;
      const reply = await safeCall(
        createBlock(pageId, {
          afterId: after,
          parentId: null,
          text: "",
        }),
      );
      if (!reply) return;
      deps.applyView(reply.view);
      setAppState("selectedBlockId", reply.new_id);
      setAppState("editingBlockId", reply.new_id);
    },
    NewBlockAbove: async () => {
      const pageId = appState.page?.id;
      const anchor = appState.selectedBlockId;
      if (!pageId) return;
      // `beforeId` lands the new block as the sibling immediately
      // before the selected one (vim `O`); with nothing selected it
      // falls back to appending at the page root.
      const reply = await safeCall(
        createBlock(
          pageId,
          anchor ? { beforeId: anchor, text: "" } : { parentId: null, text: "" },
        ),
      );
      if (!reply) return;
      deps.applyView(reply.view);
      setAppState("selectedBlockId", reply.new_id);
      setAppState("editingBlockId", reply.new_id);
    },

    // ── block structure ops on the selected block ────────────────
    IndentBlock: async () => {
      const pageId = appState.page?.id;
      const id = targetBlockId();
      if (!pageId || !id) return;
      await runOn(indentBlock(pageId, id));
    },
    OutdentBlock: async () => {
      const pageId = appState.page?.id;
      const id = targetBlockId();
      if (!pageId || !id) return;
      await runOn(outdentBlock(pageId, id));
    },
    MoveBlockUp: async () => {
      const pageId = appState.page?.id;
      const id = targetBlockId();
      if (!pageId || !id) return;
      await runOn(moveBlockUp(pageId, id));
    },
    MoveBlockDown: async () => {
      const pageId = appState.page?.id;
      const id = targetBlockId();
      if (!pageId || !id) return;
      await runOn(moveBlockDown(pageId, id));
    },
    DeleteBlock: async () => {
      const pageId = appState.page?.id;
      const id = targetBlockId();
      if (!pageId || !id) return;
      // Move selection to the previous visible block first so the
      // cursor doesn't land on `null` after the delete.
      const prev = previousVisibleId(id, appState.outline);
      await runOn(deleteBlock(pageId, id), prev);
    },
    // Delete the currently-viewed page. Triggered from the sidebar's
    // hover × button (the handler is reused here for keyboard parity
    // if a future binding maps a chord to `DeletePage`). We confirm
    // via the OS dialog before calling the backend; the backend does
    // NOT re-confirm. Returns today's journal so the view navigates
    // away from the deleted page in the same round-trip.
    DeletePage: async () => {
      const slug = appState.page?.slug;
      if (!slug) return;
      const title = appState.page?.title ?? slug;
      const ok = window.confirm(
        `Delete page "${title}"?\n\nThis removes the page and all its blocks. ` +
          `The deletion syncs to paired devices.`,
      );
      if (!ok) return;
      const view = await safeCall(deletePage(slug));
      if (view) deps.applyView(view);
    },
    ToggleCollapsed: async () => {
      const pageId = appState.page?.id;
      const id = targetBlockId();
      if (!pageId || !id) return;
      const block = lookupBlock(id);
      if (!block || block.children.length === 0) return;
      await runOn(setBlockCollapsed(pageId, id, !block.collapsed));
    },

    // ── block TODO toggle (Cmd+T) ────────────────────────────────
    //
    // Targets the focused textarea (Insert) or the selected block
    // (Normal) — same fallback chain as IndentBlock/etc.
    ToggleTodo: async () => {
      // Editing: splice the draft. Going through the backend here
      // updated the outline behind a textarea whose draft is seeded
      // once and never re-syncs, so the new state only appeared after
      // leaving Insert. The commit on blur persists it.
      if (cycleTodoInDraft()) return;
      const pageId = appState.page?.id;
      if (!pageId) return;
      const id = targetBlockId();
      if (!id) {
        deps.setError(
          "Select or click a block first, then ⌘T toggles its TODO",
        );
        return;
      }
      const view = await safeCall(toggleTodoCmd(pageId, id));
      if (view) deps.applyView(view);
      // Plugin `onOp` sweep runs OFF the input path — fire-and-forget, no
      // `await` (see the outl async-writes principle). A TODO toggle is an
      // op; a `ui-render` plugin emits HTML here (e.g. confetti on DONE).
      // Blocking the toggle on the plugin JS just delayed the checkbox.
      void safeCall(pluginSyncHooks(pageId)).then((hooked) => {
        if (hooked?.view) deps.applyView(hooked.view);
        if (hooked) playPluginViews(hooked.views);
      });
    },

    // ── overlays + insert escape ─────────────────────────────────
    //
    // Esc cascades: close the topmost overlay first (Help → Picker
    // → Settings), and if no overlay is up but a textarea is
    // focused, blur it. The blur fires `<BlockRow />`'s `onBlur`
    // handler (which commits + flips `editingBlockId` to null), so
    // the user is back in Normal mode without a second key.
    ExitInsert: () => {
      if (appState.timelineOpen) {
        setAppState("timelineOpen", false);
        return;
      }
      if (appState.remindersOpen) {
        setAppState("remindersOpen", false);
        return;
      }
      if (appState.helpOpen) {
        setAppState("helpOpen", false);
        return;
      }
      if (appState.pickerOpen) {
        setAppState("pickerOpen", false);
        return;
      }
      if (appState.settingsOpen) {
        setAppState("settingsOpen", false);
        return;
      }
      // Leaving Visual via Esc — capture the range for `gv`, drop
      // back to Normal. Must come before the textarea-blur branch
      // (Visual mode has no focused textarea, so the blur is a no-op
      // anyway; the explicit return keeps the mode flip atomic).
      if (appState.mode === "vim-visual") {
        exitVisual();
        return;
      }
      const el = document.activeElement;
      if (el instanceof HTMLTextAreaElement || el instanceof HTMLInputElement) {
        el.blur();
        return;
      }
      // Normal-mode `Esc` with nothing focused: cancel a pending cut
      // so the dimmed block snaps back. A copy stays on the clipboard
      // (it's non-destructive and reusable, like the OS clipboard).
      if (appState.blockClipboard?.kind === "cut") {
        setAppState("blockClipboard", null);
      }
    },

    // ── Vim Visual mode ──────────────────────────────────────────
    //
    // The desktop's Visual mode covers a contiguous range of outline
    // blocks (vim's `V` line-visual semantics). `j` / `k` extend the
    // range, `y` yanks, `d` deletes, `>` / `<` shift indent, `Esc`
    // exits + captures `lastVisualRange` so `gv` can restore.
    EnterVisual: () => {
      const id = appState.selectedBlockId;
      if (!id) return;
      setAppState("visualAnchorId", id);
      setAppState("mode", "vim-visual");
    },
    ReselectLastVisual: () => {
      const range = appState.lastVisualRange;
      if (!range) {
        deps.setError("no previous selection");
        return;
      }
      // Verify both ids still exist in the outline (a peer might
      // have deleted them between sessions). Drop gracefully.
      const ids = flattenVisible(appState.outline);
      if (!ids.includes(range.lo) || !ids.includes(range.hi)) {
        deps.setError("previous selection no longer exists");
        return;
      }
      setAppState("visualAnchorId", range.lo);
      setAppState("selectedBlockId", range.hi);
      setAppState("mode", "vim-visual");
    },
    YankRange: () => {
      const range = visualRangeIds(
        appState.visualAnchorId,
        appState.selectedBlockId,
        appState.outline,
      );
      if (!range) return;
      const ids = flattenVisible(appState.outline);
      const loIdx = ids.indexOf(range.lo);
      const hiIdx = ids.indexOf(range.hi);
      const rangeIds = ids.slice(loIdx, hiIdx + 1);
      const texts: string[] = [];
      for (const id of rangeIds) {
        const block = lookupBlock(id);
        if (block) texts.push(block.text);
      }
      setAppState("yankRegister", texts);
      exitVisual();
      // Copy the whole range as markdown; the backend drops any block
      // whose ancestor is also selected so a parent+child range doesn't
      // duplicate the child.
      void copySelectionToClipboard(rangeIds);
    },
    DeleteRange: async () => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const range = visualRangeIds(
        appState.visualAnchorId,
        appState.selectedBlockId,
        appState.outline,
      );
      if (!range) return;
      // Snapshot the range as ids (NOT indices) up front. NodeIds are
      // stable across the CRDT — `deleteBlock` is `Move(node, TRASH)`,
      // not a re-keying — so we don't have to re-resolve them after
      // each round-trip. We DO have to tolerate individual failures:
      // if the range straddles a parent + descendants, deleting the
      // parent moves the whole subtree to trash, and the follow-up
      // delete on a descendant fails with "block already in trash"
      // (or a peer's concurrent delete won the race). `safeCall`
      // captures the error in the status line; we keep iterating so
      // a single bad id doesn't strand the rest of the range.
      const ids = flattenVisible(appState.outline);
      const loIdx = ids.indexOf(range.lo);
      const hiIdx = ids.indexOf(range.hi);
      const targets: string[] = [];
      // Bottom-up: when the range covers both a parent and its
      // children, the children go first so the parent's move-to-trash
      // doesn't pull a still-targeted descendant from under us.
      for (let i = hiIdx; i >= loIdx; i--) targets.push(ids[i]);
      let lastView: PageView | undefined;
      for (const id of targets) {
        const view = await safeCall(deleteBlock(pageId, id));
        if (view) lastView = view;
      }
      if (lastView) deps.applyView(lastView);
      // Land selection on the block above the deleted range, or the
      // first block if the range was at the top.
      const prev = ids[Math.max(loIdx - 1, 0)];
      exitVisual();
      setAppState("selectedBlockId", prev ?? null);
    },
    IndentVisualRange: async () => {
      await applyVisualBlockOp((pageId, id) => indentBlock(pageId, id));
    },
    OutdentVisualRange: async () => {
      await applyVisualBlockOp((pageId, id) => outdentBlock(pageId, id));
    },
    // `Cmd/Ctrl+Shift+↑` — drag the whole range up among its siblings.
    // Top-down walk: the first block slides up past the block above the
    // range, then each following block moves into the slot it vacated.
    // Selection follows (block ids are stable) so the user can repeat.
    MoveVisualRangeUp: async () => {
      await applyVisualBlockOp((pageId, id) => moveBlockUp(pageId, id));
    },
    // `Cmd/Ctrl+Shift+↓` — same, downward. Bottom-up walk so the last
    // block clears the block below the range before its neighbours move.
    MoveVisualRangeDown: async () => {
      await applyVisualBlockOp((pageId, id) => moveBlockDown(pageId, id), true);
    },
    // `Shift+↓` / `Shift+↑` — start (from Normal) or extend a contiguous
    // selection. The non-vim multi-select entry; both chords keep
    // extending once the client is in Visual.
    SelectRangeDown: () => {
      extendVisualRange(1);
    },
    SelectRangeUp: () => {
      extendVisualRange(-1);
    },

    // ── Fold control over the whole page (zR / zM) ───────────────
    UnfoldAll: async () => {
      await applyCollapsedToAll(false);
    },
    FoldAll: async () => {
      await applyCollapsedToAll(true);
    },

    // ── Zoom / focus (z i / z o, Cmd+Shift+] / Cmd+Shift+[) ───────
    //
    // View-state only: `focusBlockId` slices the outline via
    // `focusSubtree` at render time; no op, no backend round-trip. The
    // breadcrumb click targets in `<OutlineView />` set the same signal.
    ZoomIn: () => {
      const id = appState.selectedBlockId;
      if (!id) return;
      setAppState("focusBlockId", id);
    },
    // The desktop keeps no zoom stack; pop one level by reading the
    // current focus's breadcrumb — its last crumb is the parent to zoom
    // to, and an empty breadcrumb means the focused block is top-level,
    // so zooming out exits to the full page. A stale focus (block gone)
    // also exits.
    ZoomOut: () => {
      const id = appState.focusBlockId;
      if (!id) return;
      const fv = focusSubtree(appState.outline, id);
      if (fv && fv.breadcrumb.length > 0) {
        setAppState("focusBlockId", fv.breadcrumb[fv.breadcrumb.length - 1].id);
      } else {
        setAppState("focusBlockId", null);
      }
    },

    // ── Viewport (zz) ────────────────────────────────────────────
    CenterViewport: () => {
      const id = appState.selectedBlockId;
      if (!id) return;
      const el = document.querySelector<HTMLElement>(`[data-block-id="${id}"]`);
      if (el) el.scrollIntoView({ block: "center", behavior: "smooth" });
    },

    // ── Search the workspace for the selected block's text (* / #) ──
    //
    // Vim's `*` / `#` are "search the word under the cursor". The
    // desktop's Normal mode has no character cursor — only a
    // selected block — so the closest useful gesture is "search for
    // something in this block's text". We pre-fill the picker; the
    // user can refine the query before accepting.
    SearchWordForward: () => {
      seedPickerWithCurrentBlock();
    },
    SearchWordBackward: () => {
      // `#` (backward) collapses to the same gesture on the desktop —
      // the picker is bidirectional (fuzzy match, not directional).
      seedPickerWithCurrentBlock();
    },

    // ── Char-cursor ops (Normal) — TUI-only ──────────────────────
    //
    // These need a character cursor inside the selected block,
    // something the desktop's Normal mode does not have (only a
    // selected block id). The catalog entries stay so vim users
    // see them in the help overlay; firing any of them surfaces
    // the same nudge via `charCursorNudge` (one source of truth).
    DeleteCharUnderCursor: charCursorNudge,
    DeleteCharBeforeCursor: charCursorNudge,
    DeleteToEndOfBlock: charCursorNudge,
    ChangeToEndOfBlock: charCursorNudge,
    SubstituteChar: charCursorNudge,
    ReplaceChar: charCursorNudge,
    FindCharForward: charCursorNudge,
    FindCharBackward: charCursorNudge,
    ToggleCharCase: charCursorNudge,
    CursorWordEnd: charCursorNudge,

    // ── inline markdown wrappers (Insert mode) ───────────────────
    //
    // These act on the active `<textarea>` via DOM — no Solid
    // signal lookup needed. The dispatcher only fires them when
    // mode === "insert", so we know a textarea is focused.
    WrapBold: () => wrapSelection("**"),
    // outl's canonical italic delimiter is `_` (the markdown parser
    // accepts `*…*` too, but `_…_` is the form the workspace ships
    // and the form we want Cmd+I to produce).
    WrapItalic: () => wrapSelection("_"),
    WrapCode: () => wrapSelection("`"),
    WrapStrike: () => wrapSelection("~~"),
    InsertLink: () => insertLink(),

    // ── run code block (Cmd+Shift+X) ─────────────────────────────
    //
    // Targets the selected block (Normal — inside a textarea the
    // chord resolves to the Insert-mode WrapStrike binding instead).
    // Resolves the block, asks the backend to execute the
    // `\`\`\`lang … \`\`\`` fence via `outl-exec`, and surfaces the
    // resulting stdout/stderr through `applyView`. Plain `Cmd+X`
    // deliberately has no binding — it falls through to the
    // webview's native cut (issue #80).
    RunCodeBlock: async () => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const id = targetBlockId();
      if (!id) {
        deps.setError(
          "Select or focus a code block first, then ⌘⇧X runs it",
        );
        return;
      }
      const reply = await safeCall(runCodeBlock(pageId, id));
      if (reply) deps.applyView(reply.view);
    },

    // ── undo / redo (Cmd+Z / Cmd+Shift+Z, Normal mode) ───────────
    //
    // Block-level history: reverts / re-applies the last committed
    // mutation on the current page (the backend snapshots the
    // page's `.md` render around every `finish_in_page` mutation).
    // The chords are Normal-mode only, so a focused textarea keeps
    // its own `Cmd+Z`. An empty stack rejects with "nothing to
    // undo" / "nothing to redo" — surfaced via the status bar.
    // Selection is left as-is; if the restored outline no longer
    // contains the selected block, `<OutlineView />`'s
    // auto-selection recovers.
    Undo: async () => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const view = await safeCall(undoPage(pageId));
      if (view) deps.applyView(view);
    },
    Redo: async () => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const view = await safeCall(redoPage(pageId));
      if (view) deps.applyView(view);
    },

    // ── commit + new sibling ─────────────────────────────────────
    //
    // Fires on `Cmd+Shift+Enter` inside a textarea (Insert mode).
    // Reads the live draft straight off the focused textarea and
    // commits, then asks the backend for a fresh sibling and parks
    // edit mode on the new id so the user keeps typing without a
    // click. Mirrors the old `<BlockRow />` intercept that used to
    // own this chord before it moved to the catalog.
    CommitAndContinue: async () => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const el = document.activeElement;
      if (!(el instanceof HTMLTextAreaElement) || !el.dataset.blockId) {
        return;
      }
      const id = el.dataset.blockId;
      const text = el.value;
      const editedView = await safeCall(editBlock(pageId, id, text));
      if (editedView) deps.applyView(editedView);
      const reply = await safeCall(
        createBlock(pageId, { afterId: id, parentId: null, text: "" }),
      );
      if (!reply) return;
      deps.applyView(reply.view);
      setAppState("editingBlockId", reply.new_id);
      setAppState("selectedBlockId", reply.new_id);
    },

    // ── catalog-only (no JS handler today) ───────────────────────
    //
    // Intentionally absent so the dispatcher falls through (no
    // preventDefault) and the textarea / OS handles the chord:
    //
    //   EditBlock chord buffer (CopyBlockRef), EnterVisual,
    //   YankRange, DeleteRange, OpenCommandPalette.
    //
    // Each shows up as `[shortcuts] … no JS handler` in DevTools so
    // a debugging session can see what's still on the to-do list.
  };
}
