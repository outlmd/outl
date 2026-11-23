import { For, Show } from "solid-js";

import type { PageView } from "@outl/shared/api/types";
import type { FocusView } from "@outl/shared/outline";
import { HistoryIcon, PageGlyph, PaperclipIcon } from "@outl/shared/icons";

import { pageHeaderKeydown } from "../lib/keyboard-reach";
import { appState, setAppState } from "../lib/store";
import { PageTitleEditor } from "./PageTitleEditor";
import { PropertyEditor } from "./PropertyEditor";
import { ZoomTitle } from "./ZoomTitle";

/**
 * Everything above the outline: the eyebrow, the title, the two icon
 * buttons beside the slug, and the page's own `key:: value` chips.
 *
 * Lifted out of `<OutlineView />` verbatim when the title became the
 * rename door — that file is frozen by the size ratchet
 * (`.github/file-size-baseline.txt`), and new behaviour belongs in a
 * sibling rather than in the file that was already too big.
 *
 * It owns no mutations. Everything that writes is a prop, so the page's
 * single writer stays `applyPageState` through `<OutlineView />`'s
 * `applyView` (`lib/page-view.ts`).
 */
export function PageHeader(props: {
  /** The zoom, resolved: the focused block's subtree + breadcrumb. */
  focus: () => FocusView | null;
  pageCrumbLabel: () => string;
  onAttachFile: () => void;
  onRefClick: (target: string) => void;
  onTagClick: (tag: string) => void;
  onLinkClick: (href: string) => void;
  onSetPageProperty: (key: string, value: string) => void;
  onRenamed: (view: PageView) => void;
}) {
  // The controls below — the title's rename door, Attach file, Page
  // history — are one roving group. They are chrome, not catalog
  // actions, so arrows are their reach and no chord is minted
  // (`lib/keyboard-reach.ts`).
  //
  // No rule under the header: a full-width border read as two stacked
  // panes, not as a title naming the outline below it. `pt-16` clears
  // the chrome capsule, like the sidebar does.
  return (
    <header class="px-12 pt-16 pb-3" onKeyDown={pageHeaderKeydown}>
      <div class="mx-auto max-w-3xl">
        {/*
         * When zoomed (Roam/Workflowy focus) the header becomes the
         * focused block's own page-like header: a clickable path back
         * to the journal/page + ancestors as the eyebrow, and the
         * block's text as the title. Otherwise the normal page header.
         */}
        <Show
          when={props.focus()}
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
                      data-outl-header-stop
                      class="opacity-50 hover:opacity-100"
                      title="Attach a file (PDF, image) as a new block"
                      aria-label="Attach a file"
                      onClick={props.onAttachFile}
                    >
                      <PaperclipIcon />
                    </button>
                    <button
                      type="button"
                      data-outl-header-stop
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
                  {/* The title is the rename door. `applyView` is the
                   *  same single writer (`applyPageState`) every other
                   *  reply in this component goes through; a merge
                   *  lands on another page and
                   *  `installPageChangeEffect` does the navigation
                   *  reset from the id change. */}
                  {(page) => (
                    <PageTitleEditor page={page()} onRenamed={props.onRenamed} />
                  )}
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
                    {props.pageCrumbLabel()}
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
                  onRefClick={props.onRefClick}
                  onTagClick={props.onTagClick}
                  onLinkClick={props.onLinkClick}
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
        <Show when={!props.focus() && appState.page}>
          <PropertyEditor
            properties={appState.pageProperties}
            noun="page property"
            addAffordance="always"
            onCommit={props.onSetPageProperty}
            onError={(msg) => setAppState("lastError", msg)}
            chipClass="rounded bg-(--color-outl-fg)/8 px-1.5 py-0.5 text-xs opacity-70 hover:opacity-100"
            inputClass="rounded border border-(--color-outl-accent)/50 bg-(--color-outl-bg) px-1.5 py-0.5 text-xs outline-none"
          />
        </Show>
      </div>
    </header>
  );
}
