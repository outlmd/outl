/**
 * Mutations whose unit is the **block under the cursor** — the one
 * `targetBlockId()` resolves to, focused textarea first, selection
 * cursor second.
 *
 * Entering Insert belongs here rather than with the chrome toggles:
 * `editingBlockId` names a block, and three of the four entries
 * (`S`, `A`, `Cmd+Shift+Enter`) edit its text on the way in.
 *
 * The inline markdown wrappers are here for the same reason — they
 * splice the draft of the block being edited. They own no logic;
 * `markdown-wrap.ts` does.
 *
 * Authoring a `remind::` rule is here too, for the same test: it
 * writes a property on the selected block. Only the panel that
 * *lists* reminders is chrome. The split's boundary is the unit a
 * keystroke acts on, not the feature it belongs to, so a feature can
 * legitimately land in two modules.
 */

import {
  type BacklinkRowOp,
  createBlock,
  deleteBlock,
  editBlock,
  indentBlock,
  markBlockDone,
  moveBlockDown,
  moveBlockUp,
  outdentBlock,
  pluginSyncHooks,
  runCodeBlock,
  setBlockCollapsed,
  setBlockRemind,
  snoozeReminder,
  toggleTodo as toggleTodoCmd,
} from "@outl/shared/api/commands";
import type { PageView } from "@outl/shared/api/types";
import { embedOnlyHandle, previousVisibleId } from "@outl/shared/outline";

import { backlinkRowOps } from "./backlink-handlers";
import { type HandlerCtx, lookupBlock, targetBlockId } from "./handler-context";
import { cycleTodoInDraft, insertLink, wrapSelection } from "./markdown-wrap";
import { revealBlock } from "./keyboard-reach";
import { moveOnce } from "./move-gate";
import { playPluginViews } from "./plugin-views";
import { type ActionHandlers } from "./shortcuts";
import { appState, setAppState } from "./store";

export function blockHandlers(ctx: HandlerCtx): ActionHandlers {
  // Plain record, not Solid props — see `HandlerCtx`.
  const { deps, runOn, safeCall } = ctx;
  /** The cross-page half of every mutation below: the cursor may be on a
   *  backlink row, where the block lives on another page. Each op asks
   *  this first and falls through on `false`; `backlink-handlers.ts` owns
   *  the whole decision, including which replies may reach the screen. */
  const onBacklink = backlinkRowOps(ctx);

  /** Run an op on whatever block the keystroke is aimed at, and show
   *  what it returns. The four structure ops below are this shape and
   *  nothing else, so a fifth cannot forget `targetBlockId`'s
   *  focused-textarea-first fallback and quietly act on the selection
   *  instead — nor the backlinks branch, where `targetBlockId` answers
   *  `null` and the page the command needs is not the page on screen.
   *
   *  **Two names per op, and the pairing is the point.** The cursor can
   *  be in either region, and the two regions take *different commands*
   *  for the same gesture: `cmd` is the outline's, which may tolerate a
   *  stale id because the page is on screen, and `op` names the
   *  cross-page one, which refuses. Collapsing them to one argument is
   *  how a backlink row would get the tolerant recovery back. */
  const onTarget = async (
    op: BacklinkRowOp,
    cmd: (pageId: string, id: string) => Promise<PageView>,
  ) => {
    if (await onBacklink.run(op)) return;
    const pageId = appState.page?.id;
    const id = targetBlockId();
    if (!pageId || !id) return;
    await runOn(cmd(pageId, id));
  };

  /** `onTarget` for the two move ops. The backlink half is untouched;
   *  the outline half goes through `moveOnce` (a repeat reads edges the
   *  previous move has not repainted), refuses the zoom root (zoom is
   *  view state: the block would move while drawn nowhere) and scrolls
   *  the moved row back into view. */
  const onMove = async (
    op: "move-up" | "move-down",
    cmd: (pageId: string, id: string) => Promise<PageView>,
  ) => {
    if (await onBacklink.run(op)) return;
    const pageId = appState.page?.id;
    const id = targetBlockId();
    if (!pageId || !id || id === appState.focusBlockId) return;
    await moveOnce(async () => {
      await runOn(cmd(pageId, id));
      revealBlock(id);
    });
  };

  /** Park edit mode on the block under the cursor. Three catalog
   *  entries share it; see the Insert section below for why.
   *
   *  The cursor may be in the backlinks section, where the block lives
   *  on another page — `onBacklink.enterInsert` is the one owner of that
   *  case, shared with `Enter` (`OpenRefUnderCursor`), so the two chords
   *  cannot disagree about which rows are editable, and a row with
   *  nothing to edit says so on both. */
  const enterInsert = () => {
    if (onBacklink.enterInsert()) return;
    const id = appState.selectedBlockId;
    if (!id) return;
    setAppState("editingBlockId", id);
  };

  return {
    // ── reminders (`remind::`) ────────────────────────────────────
    //
    // Authoring writes a block property, so the unit is the selected
    // block and these belong here, not with the panel that lists them
    // (`OpenReminders`, in `chrome-handlers.ts`). The schedule is
    // derived from the property on every scan in Rust, so editing the
    // rule reschedules for free and there is nothing to invalidate.
    InsertRemind: async () => {
      const page = appState.page?.id;
      const block = appState.selectedBlockId;
      if (!page || !block) return;
      // A bare anchor is the smallest rule that does something: one
      // fire, today, at the time the user is about to type over.
      await runOn(setBlockRemind(page, block, "9am"));
    },
    InsertRemindNag: async () => {
      const page = appState.page?.id;
      const block = appState.selectedBlockId;
      if (!page || !block) return;
      // Same preset as the TUI's `NAG_PRESET`
      // (`outl-tui/src/actions/reminders.rs`).
      await runOn(setBlockRemind(page, block, "now every 1h until DONE"));
    },
    SnoozeReminder: async () => {
      const block = appState.selectedBlockId;
      if (!block) return;
      // The preset id, not a duration — the backend resolves it, so
      // `g s` lands on the same instant as the panel's first button.
      await safeCall(snoozeReminder(block, "1h"));
    },

    // ── enter Insert (i / Shift+I / a / A / Enter) ───────────────
    //
    // `i`, `Shift+I` and `a` are the same body, and deliberately one
    // function rather than three copies of it: Normal mode on the
    // desktop has no character cursor, so "insert here", "insert at
    // column 0" and vim's append all collapse to "edit this block"
    // (the textarea's own click-or-keyboard caret lands inside the
    // buffer). The three catalog entries stay so muscle memory
    // matches. `A` is the one that really differs — it asks the row
    // for a caret, below.
    EnterInsert: enterInsert,
    EnterInsertAtStart: enterInsert,
    EnterInsertAfter: enterInsert,
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
      await runOn(editBlock(pageId, id, ""));
      setAppState("editingBlockId", id);
    },
    // ── create siblings (o / O) ──────────────────────────────────
    NewBlockBelow: async () => {
      // Before the `pageId` read, not after: on a backlink row the
      // selection cursor is null, so `{ afterId: null }` would append
      // the block to the page the user is *reading*.
      if (await onBacklink.createSibling("below")) return;
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
      // Same guard as `NewBlockBelow`, for the same reason: the fallback
      // below appends at the open page's root when nothing is selected.
      if (await onBacklink.createSibling("above")) return;
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
    IndentBlock: () => onTarget("indent", indentBlock),
    OutdentBlock: () => onTarget("outdent", outdentBlock),
    MoveBlockUp: () => onMove("move-up", moveBlockUp),
    MoveBlockDown: () => onMove("move-down", moveBlockDown),
    DeleteBlock: async () => {
      // Its own branch rather than `onTarget`: the row the cursor is on
      // is the one that disappears, so the cursor moves within the
      // backlinks section, not through `previousVisibleId`.
      if (await onBacklink.del()) return;
      const pageId = appState.page?.id;
      const id = targetBlockId();
      if (!pageId || !id) return;
      // Move selection to the previous visible block first so the
      // cursor doesn't land on `null` after the delete.
      const prev = previousVisibleId(id, appState.outline);
      await runOn(deleteBlock(pageId, id), prev);
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
      //
      // The draft branch comes first on a backlink row too: this is the
      // one Global binding here, so it fires inside the backlink editor,
      // and `<BacklinkRowEditor />`'s commit persists the spliced prefix
      // through `edit_backlink_block`.
      if (cycleTodoInDraft()) return;
      // The plugin `onOp` sweep below is deliberately not reached from a
      // backlink row: its reply is a `PageView` of whichever page it was
      // asked about, and painting the source page's view over the open
      // one is what `backlinkCommitPlan` exists to refuse. Confetti on a
      // DONE two pages away is worth less than that.
      if (await onBacklink.run("toggle-todo")) return;
      const pageId = appState.page?.id;
      if (!pageId) return;
      const id = targetBlockId();
      if (!id) {
        deps.setError(
          "Select or click a block first, then ⌘T toggles its TODO",
        );
        return;
      }
      await runOn(toggleTodoCmd(pageId, id));
      // Plugin `onOp` sweep runs OFF the input path — fire-and-forget, no
      // `await` (see the outl async-writes principle). A TODO toggle is an
      // op; a `ui-render` plugin emits HTML here (e.g. confetti on DONE).
      // Blocking the toggle on the plugin JS just delayed the checkbox.
      void safeCall(pluginSyncHooks(pageId)).then((hooked) => {
        if (hooked?.view) deps.applyView(hooked.view);
        for (const err of hooked?.errors ?? []) deps.setError(err);
        if (hooked) playPluginViews(hooked.views);
      });
    },

    // ── block mark-done (`g D`) ──────────────────────────────────
    //
    // Sets DONE outright rather than cycling: unlike `Cmd+T`, `g D` is a
    // completion gesture that lands on DONE from any state (unmarked /
    // TODO / DOING). The chord is shifted so it can't be fat-fingered
    // instead of `g d` (delete the page) — a near-miss between "finish
    // this task" and "delete this whole page".
    //
    // A query-result row (one `!((blk-…))` token) is refused, not
    // prefixed: `markBlockDone` targets the row's own block, so
    // forwarding it would write `DONE !((blk-…))` and destroy the
    // reference — the trap the TUI path resolves by rewriting the
    // *source* block. Desktop cannot do that yet: the embed wire
    // (`ResolvedBlock`) carries text/page_slug but no source page_id or
    // block id, so there is nothing to send `mark_block_done` for. The
    // honest answer is a message pointing at the source until that
    // resolution lands.
    MarkDone: async () => {
      // The embed-row guard below is not owed here: a block whose only
      // token is `!((blk-…))` references no page, so it can never appear
      // as a backlink row in the first place. Same plugin-sweep reasoning
      // as `ToggleTodo`.
      if (await onBacklink.run("mark-done")) return;
      const pageId = appState.page?.id;
      if (!pageId) return;
      const id = targetBlockId();
      if (!id) {
        deps.setError(
          "Select or click a block first, then g D marks it done",
        );
        return;
      }
      const row = lookupBlock(id);
      if (row && embedOnlyHandle(row.tokens) !== null) {
        deps.setError(
          "This row is a query result — open the source block and mark it done there",
        );
        return;
      }
      await runOn(markBlockDone(pageId, id));
      // Same fire-and-forget sweep as ToggleTodo: a `ui-render` plugin
      // (confetti on DONE) must not delay the checkbox.
      void safeCall(pluginSyncHooks(pageId)).then((hooked) => {
        if (hooked?.view) deps.applyView(hooked.view);
        for (const err of hooked?.errors ?? []) deps.setError(err);
        if (hooked) playPluginViews(hooked.views);
      });
    },

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
  };
}
