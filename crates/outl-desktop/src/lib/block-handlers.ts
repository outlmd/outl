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
import { previousVisibleId } from "@outl/shared/outline";

import { type HandlerCtx, lookupBlock, targetBlockId } from "./handler-context";
import { cycleTodoInDraft, insertLink, wrapSelection } from "./markdown-wrap";
import { playPluginViews } from "./plugin-views";
import { type ActionHandlers } from "./shortcuts";
import { appState, setAppState } from "./store";

export function blockHandlers(ctx: HandlerCtx): ActionHandlers {
  // Plain record, not Solid props — see `HandlerCtx`.
  const { deps, runOn, safeCall } = ctx;

  /** Run a `(pageId, blockId)` command on whatever block the
   *  keystroke is aimed at, and show what it returns. The four
   *  structure ops below are this shape and nothing else, so a fifth
   *  cannot forget `targetBlockId`'s focused-textarea-first fallback
   *  and quietly act on the selection instead. */
  const onTarget = async (
    cmd: (pageId: string, id: string) => Promise<PageView>,
  ) => {
    const pageId = appState.page?.id;
    const id = targetBlockId();
    if (!pageId || !id) return;
    await runOn(cmd(pageId, id));
  };

  /** Park edit mode on the selected block. Three catalog entries
   *  share it; see the Insert section below for why. */
  const enterInsert = () => {
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
    IndentBlock: () => onTarget(indentBlock),
    OutdentBlock: () => onTarget(outdentBlock),
    MoveBlockUp: () => onTarget(moveBlockUp),
    MoveBlockDown: () => onTarget(moveBlockDown),
    DeleteBlock: async () => {
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
    // TODO / DOING), and it is what the query-result gesture fires. The
    // chord is shifted so it can't be fat-fingered instead of `g d`
    // (delete the page) — a near-miss between "finish this task" and
    // "delete this whole page".
    MarkDone: async () => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const id = targetBlockId();
      if (!id) {
        deps.setError(
          "Select or click a block first, then g D marks it done",
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
