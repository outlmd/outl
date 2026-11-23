import { Show, createEffect, createSignal } from "solid-js";
import type { PageMeta } from "@outl/shared/api/types";
import { deletePage, openRef } from "@outl/shared/api/commands";
import { createConfirmQueue } from "@outl/shared/confirm";
import {
  mergeConfirmMessage,
  mergeInto,
  mergeSummary,
  nameTakenMessage,
  renameOrMerge,
  renameSummary,
  type RenameMergeResult,
} from "@outl/shared/pages/rename-merge";
import { BlockContextMenu, type BlockContextAction } from "./BlockContextMenu";
import { ConfirmDialog } from "./ConfirmDialog";
import { PageNameSheet } from "./PageNameSheet";

/** SVG paths (24x24) for the page actions menu rows. */
const RENAME_ICON = "M12 20h9M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z";
const MERGE_ICON = "M6 3v6a6 6 0 0 0 6 6h6M15 12l3 3-3 3";
const DELETE_ICON = "M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6";

/**
 * What this client's alert needs to draw. The queue behind it is
 * `@outl/shared/confirm` — the bridge from an `await`ed boolean to a
 * renderable signal is not per client, only the chrome is.
 */
interface ConfirmRequest {
  title: string;
  message: string;
  confirmLabel: string;
}

/** What `PageSwitcher` hands over: its props (read lazily, never destructured) and two list hooks. */
interface PageActionDeps {
  open: boolean;
  onPick: (slug: string, kind: "page" | "journal") => void;
  /** The summary after a rename or merge, lines joined by `\n`. */
  onNotice?: (message: string) => void;
  /** Refetch the page list after a write changed what a row shows. */
  reload: () => void;
  clearQuery: () => void;
}

/**
 * Everything the switcher does to ONE page beyond opening it: the
 * long-press actions menu, rename, merge into another page, delete.
 *
 * Journals never get here: `longPressHandlers` returns nothing for one,
 * and `pageActions` refuses one even if a caller asks. The backend
 * refuses a journal as the source of a rename or merge as well.
 *
 * The steps live in `@outl/shared/pages/rename-merge` (`renameOrMerge`, `mergeInto`)
 * so the desktop runs the same flow; this owns the phone's chrome, a
 * name sheet and a confirm dialog.
 */
export function createPageActions(deps: PageActionDeps) {
  // The page whose actions menu is open, the one being renamed, and the
  // source of a merge while the list is picking its target.
  const [actionsFor, setActionsFor] = createSignal<PageMeta | null>(null);
  const [renaming, setRenaming] = createSignal<PageMeta | null>(null);
  const [mergeSource, setMergeSource] = createSignal<PageMeta | null>(null);
  const confirms = createConfirmQueue<ConfirmRequest>();
  const confirmReq = confirms.pending;
  // The name sheet for a *new* page: `null` closed, a string (often
  // the search box's text) is what it opens prefilled with.
  const [creating, setCreating] = createSignal<string | null>(null);

  /** Ask in the app's own confirm dialog; resolves `false` on cancel. */
  const ask = (title: string, message: string, confirmLabel: string): Promise<boolean> =>
    confirms.ask({ title, message, confirmLabel });

  const answerConfirm = confirms.answer;

  // Closing the switcher abandons whatever sub-flow was open in it.
  createEffect(() => {
    if (deps.open) return;
    setActionsFor(null);
    setRenaming(null);
    setMergeSource(null);
    setCreating(null);
    answerConfirm(false);
  });

  const fail = (e: unknown) => window.alert(e instanceof Error ? e.message : String(e));

  /** Navigate to the resulting page and show what the rename / merge did. */
  function finish(result: RenameMergeResult) {
    if (result.kind === "cancelled") return;
    const notice =
      result.kind === "renamed"
        ? renameSummary(result.report)
        : mergeSummary(result.report).join("\n");
    deps.onPick(result.view.page.slug, result.view.page.kind);
    deps.onNotice?.(notice);
    deps.reload();
  }

  /** Rename; when the name is taken, merging into its owner is offered, never assumed. */
  async function rename(p: PageMeta, newName: string) {
    setRenaming(null);
    try {
      finish(
        await renameOrMerge(p.slug, newName, (existing) =>
          ask("Name already in use", nameTakenMessage(existing.title || existing.slug), "Merge"),
        ),
      );
    } catch (e) {
      fail(e);
    }
  }

  /** Merge always asks. Cancelling stays in the picker so another target can be chosen. */
  async function confirmAndMerge(target: PageMeta) {
    const source = mergeSource();
    if (!source) return;
    const ok = await ask(
      "Merge pages",
      mergeConfirmMessage(source.title || source.slug, target.title || target.slug),
      "Merge",
    );
    if (!ok) return;
    setMergeSource(null);
    try {
      finish(await mergeInto(source.slug, target.slug));
    } catch (e) {
      fail(e);
    }
  }

  /**
   * Delete a page after confirmation. The shared `deletePage` wrapper
   * calls the backend, which returns today's journal; passing it to
   * `onPick` navigates away from the deleted page, then the list is
   * refetched to drop the row.
   */
  async function remove(p: PageMeta) {
    const label = p.title || p.slug;
    // `window.confirm` maps to a native dialog on iOS WKWebView -
    // a real "Delete / Cancel" sheet, not a web alert.
    const ok = window.confirm(
      `Delete page "${label}"?\n\nThis removes the page and all its blocks. ` +
        `The deletion syncs to paired devices.`,
    );
    if (!ok) return;
    try {
      const view = await deletePage(p.slug);
      deps.onPick(view.page.slug, view.page.kind);
      deps.reload();
    } catch (e) {
      // Surface as a brief alert - the picker is already modal.
      fail(e);
    }
  }

  function pageActions(): BlockContextAction[] {
    const p = actionsFor();
    if (!p || p.kind === "journal") return [];
    return [
      // `armsKeyboard` because this ends in a text field and the menu
      // defers `onSelect`. It worked without it only by coincidence —
      // the switcher's search box is `autofocus`, so the keyboard was
      // already up — and that coincidence is not a mechanism.
      { id: "rename", label: "Rename page...", iconPath: RENAME_ICON, armsKeyboard: true, onSelect: () => setRenaming(p) },
      {
        id: "merge",
        label: "Merge into...",
        iconPath: MERGE_ICON,
        onSelect: () => {
          deps.clearQuery();
          setMergeSource(p);
        },
      },
      {
        id: "delete",
        label: "Delete page",
        iconPath: DELETE_ICON,
        destructive: true,
        onSelect: () => void remove(p),
      },
    ];
  }

  /**
   * Create a page by name and open it.
   *
   * `openRef` is the whole implementation on purpose: it runs the
   * canonical decision tree (a date → that journal, an existing
   * literal / slugified / title match → that page, otherwise a new
   * page titled with what was typed), so a name that already belongs
   * to a page **opens** it rather than minting a second one. Branching
   * on the shape here first is the regex-vs-parser drift that command
   * exists to remove.
   *
   * Before this existed the only way to start a page that is not a
   * daily note was to type `[[` into a block and tap the link
   * ([#573](https://github.com/outlmd/outl/issues/573)), which a
   * phone-only user has no reason to know.
   */
  async function createPage(name: string) {
    setCreating(null);
    try {
      const view = await openRef(name.trim());
      deps.onPick(view.page.slug, view.page.kind);
      deps.reload();
    } catch (e) {
      fail(e);
    }
  }

  /** A tapped page row: open it, or, while picking a merge target, merge into it. */
  function openPage(p: PageMeta) {
    if (mergeSource()) void confirmAndMerge(p);
    else deps.onPick(p.slug, p.kind);
  }

  return {
    mergeSource,
    setMergeSource,
    creating,
    startCreating: setCreating,
    createPage,
    openPage,
    openActions: setActionsFor,
    actionsFor,
    renaming,
    setRenaming,
    rename,
    pageActions,
    confirmReq,
    answerConfirm,
  };
}

export type PageActions = ReturnType<typeof createPageActions>;

/**
 * Long-press detector for page rows (not journals). iOS context
 * menus fire on a sustained touch (~500ms) without significant
 * movement - the same gesture UIKit uses for interactive menus.
 * Returns Solid `onPointer*` handlers to spread onto the row.
 */
export function longPressHandlers(p: PageMeta, onLongPress: (p: PageMeta) => void) {
  if (p.kind === "journal") return {};
  let timer: ReturnType<typeof setTimeout> | undefined;
  let start: { x: number; y: number } | undefined;
  // Set when the long-press fires so the subsequent `click` event
  // (the browser always emits one when the finger lifts, even
  // after a sustained hold) is suppressed. Without this, both
  // the actions menu and `onPick` fire on the same gesture.
  let suppressedClick = false;
  const LONG_PRESS_MS = 500;
  const MOVE_TOLERANCE = 10;
  const cancel = () => {
    if (timer) clearTimeout(timer);
    timer = undefined;
  };
  return {
    onPointerDown(e: PointerEvent) {
      start = { x: e.clientX, y: e.clientY };
      timer = setTimeout(() => {
        timer = undefined;
        suppressedClick = true;
        onLongPress(p);
      }, LONG_PRESS_MS);
    },
    onPointerMove(e: PointerEvent) {
      if (!start || !timer) return;
      const dx = Math.abs(e.clientX - start.x);
      const dy = Math.abs(e.clientY - start.y);
      if (dx > MOVE_TOLERANCE || dy > MOVE_TOLERANCE) cancel();
    },
    onPointerUp: cancel,
    onPointerCancel: cancel,
    // Solid ignores `onClickCapture`; `on:click` with `capture` is the
    // way to run before the row's delegated `onClick`. Eats the click
    // that follows a long-press so `onPick` doesn't also run.
    "on:click": {
      capture: true,
      handleEvent(e: MouseEvent) {
        if (suppressedClick) {
          e.preventDefault();
          e.stopPropagation();
          suppressedClick = false;
        }
      },
    },
  };
}

/** The banner above the list while it is picking a merge target, with its way out. */
export function MergeBanner(props: { actions: PageActions }) {
  return (
    <Show when={props.actions.mergeSource()}>
      {(source) => (
        <div class="mx-4 mb-1 flex items-center gap-3 rounded-xl bg-(--color-outl-accent)/10 px-3 py-1">
          <p class="min-w-0 flex-1 text-[14px] leading-snug">
            Merge <span class="font-semibold">{source().title || source().slug}</span> into which
            page?
          </p>
          <button
            type="button"
            onClick={() => props.actions.setMergeSource(null)}
            class="min-h-11 shrink-0 px-2 text-[15px] font-medium text-(--color-outl-accent) active:opacity-60"
          >
            Cancel
          </button>
        </div>
      )}
    </Show>
  );
}

/** The menu, the rename sheet and the confirm dialog, all stacked above the switcher. */
export function PageActionOverlays(props: { actions: PageActions }) {
  const a = () => props.actions;
  return (
    <>
      <BlockContextMenu
        open={a().actionsFor() !== null}
        actions={a().pageActions()}
        onClose={() => a().openActions(null)}
      />
      <PageNameSheet
        open={a().creating() !== null}
        title="New page"
        initial={a().creating() ?? ""}
        submitLabel="Create"
        submitUnchanged
        onSubmit={(name) => void a().createPage(name)}
        onCancel={() => a().startCreating(null)}
      />
      <PageNameSheet
        open={a().renaming() !== null}
        title="Rename page"
        initial={a().renaming()?.title || a().renaming()?.slug || ""}
        submitLabel="Rename"
        onSubmit={(name) => {
          const p = a().renaming();
          if (p) void a().rename(p, name);
        }}
        onCancel={() => a().setRenaming(null)}
      />
      <ConfirmDialog
        open={a().confirmReq() !== null}
        title={a().confirmReq()?.title ?? ""}
        message={a().confirmReq()?.message ?? ""}
        confirmLabel={a().confirmReq()?.confirmLabel}
        onConfirm={() => a().answerConfirm(true)}
        onCancel={() => a().answerConfirm(false)}
      />
    </>
  );
}
