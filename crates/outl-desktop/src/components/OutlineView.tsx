import { For, Show, createEffect, createMemo, onCleanup, onMount } from "solid-js";

import { FirstRunTips, isBlankOutline } from "@outl/shared/polish";
import { open } from "@tauri-apps/plugin-dialog";

import {
  attachAsset,
  createBlock,
  deleteBlock,
  editBlock,
  importAssetFile,
  instantiateTemplateAt,
  openAsset,
  openExternalUrl,
  openPageBySlug,
  openRef,
  pasteMarkdown,
  pastePlain,
  pluginRun,
  pluginSyncHooks,
  runAutoRunBlocks,
  runCodeBlock,
  setBlockCollapsed,
  splitBlock,
  toggleTodo,
  setBlockProperty,
} from "@outl/shared/api/commands";

import type { PageView } from "@outl/shared/api/types";

import { PageAheadOfLogBanner, ParseWarningsBanner } from "@outl/shared/warnings";
import { isAssetLink } from "@outl/shared/links";
import { HistoryIcon, PageGlyph, PaperclipIcon } from "@outl/shared/icons";
import {
  findBlock,
  rawTextWithTodo,
  visualRangeSet,
} from "@outl/shared/outline";
import {
  NATIVE_ASSET_PLUGIN_ID,
  NATIVE_TEMPLATE_PLUGIN_ID,
} from "../lib/slash-commands";
import { playPluginViews } from "../lib/plugin-views";
import {
  appendMarkdownToBlock,
  installFileDrop,
  joinAssetMarkdowns,
} from "@outl/shared/drag-drop";
import { setPageProperty } from "../lib/api";
import { spliceTextAtCaret } from "../lib/markdown-wrap";
import {
  appState,
  savedMarks,
  setAppState,
  visibleRoots,
  zoomView,
} from "../lib/store";
import { BlockRow, type BlockCallbacks } from "./BlockRow";
import { installBacklinksEffect } from "../lib/backlinks-effect";
import { installPageChangeEffect, resetForNavigation } from "../lib/page-change";
import { applyPageState } from "../lib/page-view";
import { createPasteOps } from "./outline-paste";
import { createStructureOps } from "./outline-structure";
import { PageSections } from "./PageSections";
import { PageTitle } from "./PageTitle";
import { ZoomTitle } from "./ZoomTitle";
import { PropertyEditor } from "./PropertyEditor";

/**
 * Boot placeholder for the outline body.
 *
 * Rendered only while `appState.page` is still null (the background
 * opener hasn't handed us today's journal yet). It occupies the same
 * column the real rows fill, so the content area reads as "already
 * laid out, filling in" instead of a bare `Loading…` that the outline
 * then displaces. Purely decorative — `aria-hidden` keeps it off the
 * a11y tree.
 */
function OutlineSkeleton() {
  // Row widths (percent) picked to look like natural outline text,
  // not a uniform block. Static list → stable across renders.
  const widths = [86, 68, 92, 54, 78, 64];
  return (
    <div class="animate-pulse space-y-3 pt-1" aria-hidden="true">
      <For each={widths}>
        {(w) => (
          <div class="flex items-center gap-2">
            <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-(--color-outl-fg)/15" />
            <span
              class="h-3.5 rounded bg-(--color-outl-fg)/10"
              style={{ width: `${w}%` }}
            />
          </div>
        )}
      </For>
    </div>
  );
}

/**
 * Center pane — title, breadcrumb, editable outline.
 *
 * Owns the editing state (which block has its textarea up) and
 * funnels every mutation through `outl-actions` via the shared
 * Tauri command wrappers. The optimistic refresh path is uniform:
 * every command returns a fresh `PageView` which we splat into the
 * store in one shot.
 */
export function OutlineView() {
  // `editingBlockId` and `selectedBlockId` live on the store so the
  // `outl-shortcuts` dispatcher can flip them from anywhere (Cmd+T,
  // `o`, `i`, `j/k`, …) without prop-drilling callbacks. Local
  // shorthand here keeps the JSX readable.
  const editingId = () => appState.editingBlockId;
  const setEditingId = (id: string | null) => setAppState("editingBlockId", id);

  // Per-page reset (selection, zoom) fires only on a page id change;
  // see `installPageChangeEffect` for why commits must not trigger it.
  installPageChangeEffect();

  // Backlinks are fetched on navigation, by `installBacklinksEffect`
  // (`lib/backlinks-effect.ts`). Its own module because the question it
  // answers — did the page actually change? — is answered by
  // `@outl/shared`'s key and had been answered wrong here with nothing
  // failing. The peer-reload path refetches explicitly
  // (`AppShell::refreshActivePage`) since it keeps the key.
  installBacklinksEffect();

  /**
   * Fold a reply into the page's state — `applyPageState`
   * (`lib/page-view.ts`) owns which fields that is and why the
   * ahead-of-log notice is sticky — then auto-run the page's query
   * blocks. The transclusion resolve rides the outline effect above,
   * so it is not repeated here.
   */
  function applyView(view: PageView) {
    applyPageState(view);
    void runAutoRunBlocks(view.page.id)
      .then((reply) => {
        if (reply.ran > 0) applyPageState(reply.view);
      })
      .catch(() => {});
  }

  /**
   * Memoised Visual-range membership set. Built once per
   * outline / anchor / cursor / mode change, then read O(1) by every
   * `<BlockRow />` via the `visualRangeSet` prop. The previous shape
   * called `isInVisualRange(id, anchor, cursor, outline)` per row,
   * which rebuilt `flattenVisible(blocks)` from scratch — N rows × N
   * DFS = O(N²) per Visual extension keystroke. On a 500-block page
   * the extension felt laggy by the third `j`.
   *
   * `null` outside vim-visual mode (most renders) so `<BlockRow />`
   * can short-circuit without touching the Set at all.
   */
  const visualSet = createMemo<Set<string> | null>(() => {
    if (appState.mode !== "vim-visual") return null;
    return visualRangeSet(
      appState.visualAnchorId,
      appState.selectedBlockId,
      appState.outline,
    );
  });

  /**
   * Zoom view (Roam/Workflowy focus): the focused block's subtree +
   * ancestor breadcrumb, or `null` when not zoomed — *or* when the
   * focused id left the outline, which the effect below then clears.
   *
   * `zoomView` (`lib/store.ts`) is the one owner of that resolution,
   * shared with `j` / `k` and with `<PageSections zoomed>`; the memo
   * keeps the walk to once per render, not once per reader.
   */
  const focus = createMemo(() => zoomView());

  // Self-heal a stale zoom target: `focusBlockId` still holds an id the
  // outline lost (peer delete / off-page move), so clear it and the full
  // page renders. An effect and not the memo, so the memo stays a pure
  // derivation (a `setAppState` inside one is a reactivity hazard).
  createEffect(() => {
    if (appState.focusBlockId && !focus()) {
      setAppState("focusBlockId", null);
    }
  });

  /** Blocks to render in the outline body — the same ones `j` / `k`
   *  walk (`visibleRoots`, `lib/store.ts`). Memoised because three
   *  readers call it per render and the zoom slice is an O(page) walk. */
  const rootBlocks = createMemo(visibleRoots);

  async function handleError<T>(promise: Promise<T>): Promise<T | undefined> {
    try {
      return await promise;
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
      return undefined;
    }
  }

  /** Paste: flush the draft, run the command, fold the reply back into
   *  the editor. See `./outline-paste`. */
  const pasteOps = createPasteOps({
    pageId: () => appState.page?.id,
    editingId,
    applyView,
    handleError,
  });

  /** Tab / Shift-Tab indent and outdent. See `./outline-structure`. */
  const structureOps = createStructureOps({
    pageId: () => appState.page?.id,
    applyView,
    handleError,
    setSelectedBlockId: (id) => setAppState("selectedBlockId", id),
    setEditingId,
  });

  async function handleRefClick(target: string) {
    const view = await handleError(openRef(target));
    if (!view) return;
    // A navigation, even when the ref points back at the open page —
    // so the zoom goes, the same way `Cmd+J` drops it.
    resetForNavigation();
    applyView(view);
  }

  function handleTagClick(tag: string) {
    void handleRefClick(tag);
  }

  function handleLinkClick(href: string) {
    // A `[label](assets/…)` link opens the uploaded file in the OS
    // default app (`open_asset`); everything else is an external
    // `[label](url)` opened in the system browser (scheme-guarded to
    // http(s)/mailto). Errors land on the status line instead of
    // throwing into the click handler.
    const opening = isAssetLink(href) ? openAsset(href) : openExternalUrl(href);
    void opening.catch((e) => {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    });
  }

  const cb: BlockCallbacks = {
    onStartEdit: (id) => {
      // Sync the selection cursor with whatever the user clicked so
      // `j/k` pick up from there instead of teleporting back to the
      // last vim-driven cursor.
      setAppState("selectedBlockId", id);
      setEditingId(id);
    },
    onRunPluginCommand: async (pluginId, commandId) => {
      // Native `/template <name>`: instantiate the structural template
      // under the block the slash was typed in (or the current
      // selection). Intercepted here because it reuses the slash popup
      // but is a core feature, not a plugin — `commandId` is the
      // template name (see `templateSlashCommands`).
      if (pluginId === NATIVE_TEMPLATE_PLUGIN_ID) {
        const target = appState.selectedBlockId;
        if (!target) {
          setAppState("lastError", "select a block to insert a template");
          return;
        }
        const view = await handleError(
          instantiateTemplateAt(commandId, target),
        );
        if (view) applyView(view);
        return;
      }
      // Native `/upload`: open the OS file picker and attach the chosen
      // file's link — the same flow as the 📎 button, reachable inline.
      if (pluginId === NATIVE_ASSET_PLUGIN_ID) {
        await attachFile();
        return;
      }
      // Same dispatch the `⧉` PluginPalette runs: surface the command's
      // notifications / errors on the status line, re-render from the
      // returned view, and play any `ui-render` overlays.
      const reply = await handleError(
        pluginRun(pluginId, commandId, appState.page?.id ?? null),
      );
      if (!reply) return;
      for (const note of reply.notifications) setAppState("lastError", note);
      for (const err of reply.errors) {
        setAppState("lastError", `plugin: ${err}`);
      }
      if (reply.view) applyView(reply.view);
      playPluginViews(reply.views);
    },
    onCommit: async (id, text) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const view = await handleError(editBlock(pageId, id, text));
      if (view) (applyView(view), savedMarks.mark(id));
      setEditingId(null);
      // Plugin `onOp` hooks run OFF the input path — fire-and-forget, no
      // `await`. `sync_hooks` runs the 2 plugins' JS through Boa (tens of
      // ms even in release) and blocking the commit on it stole that time
      // from the next keystroke. It dispatches EVERY op since the host's
      // last sweep, so catching it a beat later still picks up structural
      // ops (indent / move / delete). The re-render / confetti overlay
      // land whenever the hook resolves. Async-by-default (see the outl
      // async-writes principle): nothing the user waits on runs a plugin.
      void handleError(pluginSyncHooks(pageId)).then((hooked) => {
        if (hooked?.view) applyView(hooked.view);
        for (const err of hooked?.errors ?? []) setAppState("lastError", err);
        if (hooked) playPluginViews(hooked.views);
      });
    },
    onEnter: async (id, text, caretChars) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      // Commit the in-flight draft first so the workspace holds the text
      // we're about to split, then split at the caret (issue #184). The
      // head stays in this block, the tail moves into a new sibling
      // below; the backend returns the sibling's id so we drop straight
      // into edit mode on it. A caret at the end yields an empty sibling
      // (the old "Enter appends a block below" behaviour), so this one
      // path covers both. We used to find the new block by diffing the
      // outline, which mis-fired when the host block had children.
      await handleError(editBlock(pageId, id, text));
      const reply = await handleError(splitBlock(pageId, id, caretChars));
      if (!reply) return;
      applyView(reply.view);
      // The sibling carries the tail — land the caret at its start.
      setAppState("caretIntent", "start");
      setEditingId(reply.new_id);
    },
    onCreateBefore: async (id, text) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      // Commit the in-flight edit first, then create the sibling
      // before it. `beforeId` lets the backend pick the fractional
      // index; we focus the freshly-minted block via its returned id
      // (same pattern as `onEnter`).
      await handleError(editBlock(pageId, id, text));
      const reply = await handleError(
        createBlock(pageId, { beforeId: id, text: "" }),
      );
      if (!reply) return;
      applyView(reply.view);
      setAppState("selectedBlockId", reply.new_id);
      setEditingId(reply.new_id);
    },
    onIndent: structureOps.onIndent,
    onOutdent: structureOps.onOutdent,
    onDeleteEmpty: async (id) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const view = await handleError(deleteBlock(pageId, id));
      if (view) applyView(view);
      setEditingId(null);
    },
    onToggleTodo: async (id) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const view = await handleError(toggleTodo(pageId, id));
      if (view) applyView(view);
    },
    onToggleCollapsed: async (id, collapsed) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const view = await handleError(setBlockCollapsed(pageId, id, collapsed));
      if (view) applyView(view);
    },
    onPasteMarkdown: (id, caret, text, hostText) =>
      pasteOps.run(id, hostText, (page) => pasteMarkdown(page, id, caret, text)),
    onPastePlain: (id, caret, text, hostText) =>
      pasteOps.run(id, hostText, (page) => pastePlain(page, id, caret, text)),
    onRunCodeBlock: async (id) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const reply = await handleError(runCodeBlock(pageId, id));
      if (!reply) return;
      applyView(reply.view);
      if (reply.error) {
        setAppState("lastError", `${reply.language}: ${reply.error}`);
      }
    },
    onSetProperty: (blockId: string, key: string, value: string) => {
      const page = appState.page?.id;
      if (!page) return;
      // Returned, not fire-and-forget: the editor reports a rejection
      // (an empty key, a backend refusal) instead of repainting the
      // chip as if the write landed.
      return setBlockProperty(page, blockId, key, value).then(applyView);
    },
    onRefClick: handleRefClick,
    onTagClick: handleTagClick,
    onLinkClick: handleLinkClick,
    onOpenPage: async (slug) => {
      const view = await handleError(openPageBySlug(slug));
      if (view) applyView(view);
    },
    onFocusBlock: (id) => {
      // Zoom into the clicked block. Also sync the selection cursor so
      // `j/k` pick up inside the focused subtree.
      setAppState("selectedBlockId", id);
      setAppState("focusBlockId", id);
    },
  };

  /** Write a page-level property, refreshing the view on success.
   *  Rejections propagate to the editor, which surfaces them — a page
   *  property that silently failed to save is the same defect the
   *  block chips already fixed. */
  function setPagePropertyOnCurrent(key: string, value: string) {
    const pageId = appState.page?.id;
    if (!pageId) return;
    return setPageProperty(pageId, key, value).then(applyView);
  }

  async function addFirstBlock() {
    const pageId = appState.page?.id;
    if (!pageId) return;
    // When zoomed into a leaf, the empty body is *inside* the focused
    // block, so the first block must be created as its child — not at
    // the page root.
    const parentId = appState.focusBlockId;
    const reply = await handleError(
      createBlock(pageId, { afterId: null, parentId, text: "" }),
    );
    if (reply) applyView(reply.view);
  }

  /**
   * Import a file (PDF, image, …) via the OS file picker and attach its
   * link as a new block after the selection (or at the page end). The
   * backend copies it into `<root>/assets/` and returns the refreshed
   * view; outl never renders the file — the link opens it in the OS
   * default app on click.
   */
  async function attachFile() {
    const pageId = appState.page?.id;
    if (!pageId) return;
    const selected = await open({ multiple: false, directory: false });
    // A cancelled dialog resolves `null`; a single pick is a string.
    if (typeof selected !== "string") return;
    const view = await handleError(
      attachAsset(selected, pageId, appState.selectedBlockId ?? undefined),
    );
    if (view) applyView(view);
  }

  /**
   * Handle an OS file drag-drop onto the outline (drag a PDF / image
   * from Finder and drop it on a line). Each dropped file is imported
   * into `<root>/assets/` via `importAssetFile` (content-addressed,
   * idempotent, size-capped) and its ready-to-insert markdown link is
   * placed in the **target block** — the block under the drop, else the
   * current selection / edit cursor, else a fresh block at the page end.
   *
   * Insertion respects an in-flight edit: dropping onto the block being
   * edited splices the link into its textarea at the caret (so the
   * user's unsaved draft isn't clobbered); any other block is edited
   * through the op log via `editBlock`. Import failures (oversized file,
   * …) surface on the status line and are skipped — a bad file never
   * aborts the drop.
   */
  async function handleFileDrop(paths: string[], targetId: string | null) {
    const pageId = appState.page?.id;
    if (!pageId || paths.length === 0) return;
    // Import every dropped file (best-effort — a rejected file surfaces
    // on the status line and is skipped, the rest still land).
    const markdowns: string[] = [];
    for (const path of paths) {
      const asset = await handleError(importAssetFile(path));
      if (asset) markdowns.push(asset.markdown);
    }
    const combined = joinAssetMarkdowns(markdowns);
    if (!combined) return; // every import failed — error already surfaced

    // Resolve the target block: the drop-on block wins, else the current
    // selection, else the block being edited.
    const blockId =
      targetId ?? appState.selectedBlockId ?? appState.editingBlockId;

    // Dropping onto the block being edited: splice into its live textarea
    // at the caret so the in-flight draft is preserved.
    if (blockId && blockId === appState.editingBlockId) {
      const ta = document.querySelector<HTMLTextAreaElement>(
        `textarea[data-block-id="${CSS.escape(blockId)}"]`,
      );
      if (ta) {
        spliceTextAtCaret(ta, combined);
        return;
      }
    }

    // A resolved, non-editing block: append the link to its text.
    if (blockId) {
      const node = findBlock(appState.outline, blockId);
      if (node) {
        const view = await handleError(
          editBlock(
            pageId,
            blockId,
            appendMarkdownToBlock(rawTextWithTodo(node), combined),
          ),
        );
        if (view) applyView(view);
        return;
      }
    }

    // No target block at all: create a fresh block at the page end.
    const reply = await handleError(createBlock(pageId, { text: combined }));
    if (reply) applyView(reply.view);
  }

  // Wire OS file drag-drop once the outline mounts. `onDragDropEvent`
  // resolves a Promise<UnlistenFn>; we drop the listener on unmount. The
  // `dropTargetBlockId` store field drives the per-row hover highlight.
  onMount(async () => {
    let unlisten: (() => void) | undefined;
    try {
      unlisten = await installFileDrop({
        onEnter: (id) => setAppState("dropTargetBlockId", id),
        onOver: (id) => setAppState("dropTargetBlockId", id),
        onLeave: () => setAppState("dropTargetBlockId", null),
        onDrop: async (paths, id) => {
          setAppState("dropTargetBlockId", null);
          await handleFileDrop(paths, id);
        },
      });
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    }
    onCleanup(() => unlisten?.());
  });

  /** Human label for the page crumb in the zoom path — the journal's
   *  ISO slug or a regular page's title (falling back to its slug). */
  function pageCrumbLabel(): string {
    const page = appState.page;
    if (!page) return "";
    return page.kind === "journal" ? page.slug : page.title || page.slug;
  }

  return (
    // `min-w-0 min-h-0` is what makes the inner `overflow-y-auto`
    // actually constrain to the viewport. Two defaults bite here:
    //
    //   * Grid items default to `min-width: auto`, which means
    //     "as wide as my content's natural width". A row with
    //     `BUSER-DJANGO-KX9` or `((blk-XXXXXX))` then pushes the
    //     whole main column wider than the window and the body's
    //     `overflow: hidden` clips it. `min-w-0` lifts that floor
    //     so the grid cell can shrink to the viewport and the
    //     inline tokens wrap inside their column.
    //   * Flex children default to `min-height: auto`, which makes
    //     the `flex-1 overflow-y-auto` block expand to fit instead
    //     of scrolling. `min-h-0` is the matching unlock.
    //
    // Classic Tailwind/flexbox/grid pitfall on both axes; the two
    // unlocks pair.
    <main class="flex h-full min-h-0 min-w-0 flex-col outline-none" data-outl-region="outline" tabIndex={-1}>
      {/* No rule under the header: a full-width border read as two
       *  stacked panes, not as a title naming the outline below it.
       *  `pt-16` clears the chrome capsule, like the sidebar does. */}
      <header class="px-12 pt-16 pb-3">
        <div class="mx-auto max-w-3xl">
          {/*
           * When zoomed (Roam/Workflowy focus) the header becomes the
           * focused block's own page-like header: a clickable path back
           * to the journal/page + ancestors as the eyebrow, and the
           * block's text as the title. Otherwise the normal page header.
           */}
          <Show
            when={focus()}
            fallback={
              <>
                {/*
                 * Breadcrumb — mirrors the TUI's
                 * `📅 Journal · Thursday, 2026-06-04` header. For pages,
                 * it carries the slug instead.
                 *
                 * The eyebrow slot always reserves its row height
                 * (`min-h-5`), even before the page loads (both inner
                 * `<Show>`s off). Without the reserved height the row
                 * collapses to 0 while `appState.page` is null at boot,
                 * then pops in once the journal arrives and shoves the
                 * `<h1>` title down — a visible layout shift. A stable
                 * slot keeps the title pinned from the first frame.
                 */}
                <div class="mb-2 flex min-h-5 items-baseline gap-1.5 text-[12.5px] text-(--color-outl-fg-dim)">
                  <Show when={appState.page}>
                    <PageGlyph
                      icon={appState.page?.icon}
                      kind={appState.page?.kind}
                      class="self-center"
                    />
                    <Show when={appState.page?.kind === "journal"}>
                      <span>Journal ·</span>
                    </Show>
                    <span class="font-mono">{appState.page?.slug}</span>
                  </Show>
                  {/* Page chrome that is not the title: attach a file,
                   *  and page history (issue #241). A labelled `Attach
                   *  file` button over the first block put UI between
                   *  the title and the page's own text. */}
                  <Show when={appState.page}>
                    <span class="ml-auto flex items-center gap-2.5">
                      <button
                        type="button"
                        class="opacity-50 hover:opacity-100"
                        title="Attach a file (PDF, image) as a new block"
                        aria-label="Attach a file"
                        onClick={attachFile}
                      >
                        <PaperclipIcon />
                      </button>
                      <button
                        type="button"
                        class="opacity-50 hover:opacity-100"
                        title="Page history"
                        aria-label="Page history"
                        onClick={() => setAppState("timelineOpen", true)}
                      >
                        <HistoryIcon />
                      </button>
                    </span>
                  </Show>
                </div>

                <h1 class="text-[24px] font-semibold leading-[1.2] tracking-[-0.011em]">
                  <Show
                    when={appState.page}
                    fallback={<span class="opacity-40">No page open</span>}
                  >
                    {(page) => <PageTitle page={page()} />}
                  </Show>
                </h1>
              </>
            }
          >
            {(fv) => (
              <>
                {/*
                 * Zoom path (eyebrow). The leading crumb is the whole
                 * page — click it to exit the zoom and return to the
                 * journal/page; each ancestor crumb re-focuses that
                 * block. The focused block itself is the title below,
                 * so it isn't a crumb.
                 */}
                <nav
                  aria-label="Zoom path"
                  class="mb-2 flex flex-wrap items-center gap-1 text-[12.5px] text-(--color-outl-fg-dim)"
                >
                  <button
                    type="button"
                    onClick={() => setAppState("focusBlockId", null)}
                    class="flex items-center gap-1.5 opacity-70 hover:opacity-100"
                    title="Back to page"
                  >
                    <PageGlyph icon={appState.page?.icon} kind={appState.page?.kind} />
                    <span
                      class={
                        appState.page?.kind === "journal" ? "font-mono" : ""
                      }
                    >
                      {pageCrumbLabel()}
                    </span>
                  </button>
                  <For each={fv().breadcrumb}>
                    {(crumb) => (
                      <>
                        <span aria-hidden="true" class="opacity-40">
                          ›
                        </span>
                        <button
                          type="button"
                          onClick={() => setAppState("focusBlockId", crumb.id)}
                          class="max-w-[16rem] truncate opacity-70 hover:opacity-100"
                          title={crumb.text}
                        >
                          {crumb.text || "(empty)"}
                        </button>
                      </>
                    )}
                  </For>
                </nav>

                {/* Title = the focused block itself, rendered the way
                 *  every other surface renders a block — see
                 *  `<ZoomTitle />` for why it is not `root.text`. */}
                <h1 class="text-[24px] font-semibold leading-[1.2] tracking-[-0.011em]">
                  <ZoomTitle
                    root={fv().root}
                    onRefClick={handleRefClick}
                    onTagClick={handleTagClick}
                    onLinkClick={handleLinkClick}
                  />
                </h1>
              </>
            )}
          </Show>

          {/*
           * The page's own `key:: value` properties (`icon::`,
           * `type::`, …). Same editor as the block chips, because they
           * are the same data — the desktop showed them nowhere until
           * now, so `icon::` was TUI-or-`.md` only (issue #13). Hidden
           * while zoomed: the header is a *block* then, and page
           * metadata under a block title reads as the block's.
           */}
          <Show when={!focus() && appState.page}>
            <PropertyEditor
              properties={appState.pageProperties}
              noun="page property"
              addAffordance="always"
              onCommit={setPagePropertyOnCurrent}
              onError={(msg) => setAppState("lastError", msg)}
              chipClass="rounded bg-(--color-outl-fg)/8 px-1.5 py-0.5 text-xs opacity-70 hover:opacity-100"
              inputClass="rounded border border-(--color-outl-accent)/50 bg-(--color-outl-bg) px-1.5 py-0.5 text-xs outline-none"
            />
          </Show>
        </div>
      </header>

      <div class="outl-cursor-room min-w-0 flex-1 overflow-y-auto px-12 pt-1 pb-6">
        <div class="outl-reading mx-auto w-full max-w-3xl">
          <PageAheadOfLogBanner info={appState.mdAheadOfLog} client="desktop" />
          <ParseWarningsBanner warnings={appState.parseWarnings} />
          <Show
            when={rootBlocks().length > 0}
            fallback={
              // Loading → a skeleton in the outline's shape. Loaded and empty
              // (or zoomed into a leaf) → first-run tips and add-first-block.
              <Show when={appState.page} fallback={<OutlineSkeleton />}>
                <FirstRunTips client="desktop" onStart={addFirstBlock} />
                <button
                  type="button"
                  onClick={addFirstBlock}
                  class="rounded px-3 py-2 text-sm opacity-60 hover:bg-(--color-outl-fg)/5 hover:opacity-100"
                >
                  Click to add the first block
                </button>
              </Show>
            }
          >
            <For each={rootBlocks()}>
              {(block) => (
                <BlockRow
                  block={block}
                  depth={0}
                  editingId={editingId()}
                  visualSet={visualSet()}
                  cb={cb}
                />
              )}
            </For>
          </Show>

          {/* Backlinks, nested pages, namespace mentions — in that order
              below the outline (TUI parity), and none of them while
              zoomed: the header is a block there, so the page's
              references would read as that block's. */}
          <PageSections blank={isBlankOutline(rootBlocks())} zoomed={focus() !== null}
            onOpenPage={(slug) => void handleRefClick(slug)} />
        </div>
      </div>
      {/* The error surface is the top-right `<ErrorToast />` (mounted in
       *  AppShell). A base banner here sat under the fixed ChromeToggleBar
       *  and got covered — issue moved it to the notification corner. */}
    </main>
  );
}
