import {
  For,
  Show,
  createEffect,
  createMemo,
  createResource,
  createSignal,
  onCleanup,
} from "solid-js";
import type { PageMeta } from "@outl/shared/api/types";
import {
  type BlockHit,
  deletePage,
  listPages,
  searchBlocks,
  togglePin,
} from "@outl/shared/api/commands";
import { createSheetDrag } from "../lib/sheet-drag";
import { useKeyboardInset } from "../lib/viewport";

interface PageSwitcherProps {
  open: boolean;
  currentSlug: string | null;
  onClose: () => void;
  onPick: (slug: string, kind: "page" | "journal") => void;
  /** Fired when the user taps a block-search hit ("Blocks" mode,
   *  issue #19). The caller navigates to `hit.source_slug` — a
   *  `BlockHit` carries no `kind`, so the decision goes through
   *  `openRef`, same as any other tapped ref. */
  onJumpToBlock: (hit: BlockHit) => void;
}

/** Which half of the switcher is active — page navigation (original
 *  behaviour) or full-workspace block search (issue #19, RFC 0254
 *  phase 2). One sheet, one search box, two backends: building a
 *  second full-screen sheet would duplicate the drag-dismiss +
 *  keyboard-inset + Cancel chrome this one already gets right, for a
 *  feature that's otherwise "search box + list of tappable rows"
 *  just like this one. Mirrors `PropertiesSheet`'s Block/Page
 *  segmented switch (same visual pattern, same reason: one sheet,
 *  two scopes). */
type SwitcherMode = "pages" | "blocks";

/** Debounced the same way the desktop's `((…))` block-ref popup is
 *  (`BlockRow.tsx`): `search_blocks` rebuilds the whole
 *  `WorkspaceIndex` from disk per call, so firing it on every
 *  keystroke janks on a large workspace. */
const BLOCK_SEARCH_DEBOUNCE_MS = 150;

/**
 * Bottom-sheet style page switcher. Lists every page in the
 * workspace with fuzzy filtering, or — in "Blocks" mode — searches
 * block content across the whole workspace (issue #19). Tap a row to
 * open its page (a block hit opens the page hosting it, same as the
 * backlinks panel's jump — there is no per-block scroll/highlight
 * anywhere in this client). `Enter` opens the first match in either
 * mode, swipe down on the handle dismisses.
 *
 * Long-press a **page** row (not a journal, and not shown in Blocks
 * mode at all) to surface a delete confirmation. Journals are
 * excluded — deleting a daily note by accident from the picker would
 * be a hostile surprise, and there's no trash UI to recover from
 * today.
 */
export function PageSwitcher(props: PageSwitcherProps) {
  const [pages, { refetch }] = createResource(() =>
    props.open ? listPages() : Promise.resolve<PageMeta[]>([]),
  );
  const [mode, setMode] = createSignal<SwitcherMode>("pages");
  const [query, setQuery] = createSignal("");
  const [blockHits, setBlockHits] = createSignal<BlockHit[]>([]);
  let blockSearchTimer: ReturnType<typeof setTimeout> | undefined;
  let blockSearchToken = 0;
  onCleanup(() => clearTimeout(blockSearchTimer));
  // Re-runs on every `query()` change AND on `open`/`mode` flips (both
  // read directly below, not through a handler), so reopening the
  // sheet on "Blocks" re-searches instead of showing a stale list.
  createEffect(() => {
    const q = query();
    if (!props.open || mode() !== "blocks") {
      clearTimeout(blockSearchTimer);
      return;
    }
    const token = ++blockSearchToken;
    clearTimeout(blockSearchTimer);
    blockSearchTimer = setTimeout(() => {
      void searchBlocks(q.trim())
        .then((hits) => {
          if (token === blockSearchToken) setBlockHits(hits);
        })
        .catch(() => {
          if (token === blockSearchToken) setBlockHits([]);
        });
    }, BLOCK_SEARCH_DEBOUNCE_MS);
  });
  // `searchFocused` drives the "Cancel" link that slides in to the
  // right of the search input. Mirrors iOS UISearchController: the
  // affordance only appears when the search has focus, and tapping
  // it dismisses the sheet + clears the query.
  const [searchFocused, setSearchFocused] = createSignal(false);
  let searchInput: HTMLInputElement | undefined;
  // Sheet drag-to-dismiss — shared `createSheetDrag` keeps the
  // gesture identical across every bottom sheet (PageSwitcher,
  // Calendar). Wire `drag.onPointer*` onto the grab handle only,
  // never the whole header.
  const drag = createSheetDrag(() => props.onClose());
  // iOS WKWebView ignores `interactive-widget=resizes-content` (it's
  // Chromium-only), so the keyboard OVERLAYS the layout viewport
  // instead of shrinking it. Without compensating, the bottom of the
  // filtered list sits behind the keyboard and can never be scrolled
  // into view. We pad the sheet by the keyboard height so the scroll
  // area ends right above it.
  const keyboardInset = useKeyboardInset();

  const filtered = createMemo(() => {
    const all = pages() ?? [];
    const q = query().trim().toLowerCase();
    const matched = !q
      ? all
      : all.filter(
          (p) =>
            p.slug.toLowerCase().includes(q) ||
            p.title.toLowerCase().includes(q),
        );
    // Pinned pages surface first (RFC 0254 phase 4b, `TogglePin`) —
    // same "canonical entry point" grouping the desktop sidebar's
    // Pinned section gives, without splitting this single-column sheet
    // into two visually disconnected lists. `Array.prototype.sort` is
    // stable (ES2019+), so everything else keeps `listPages`' order.
    return [...matched].sort(
      (a, b) => Number(b.pinned ?? false) - Number(a.pinned ?? false),
    );
  });

  function pickFirst() {
    if (mode() === "blocks") {
      const first = blockHits()[0];
      if (first) props.onJumpToBlock(first);
      return;
    }
    const first = filtered()[0];
    if (!first) return;
    props.onPick(first.slug, first.kind);
  }

  /**
   * Delete a page after confirmation. Journals are rejected at the
   * call site (long-press only fires for `kind === "page"`). The
   * shared `deletePage` wrapper calls the backend, which returns
   * today's journal — we pass it to `onPick` so the caller navigates
   * away from the deleted page, then refetch the list to drop the
   * row from the picker.
   */
  async function handleDelete(p: PageMeta) {
    const label = p.title || p.slug;
    // `window.confirm` maps to a native dialog on iOS WKWebView —
    // a real "Delete / Cancel" sheet, not a web alert.
    const ok = window.confirm(
      `Delete page "${label}"?\n\nThis removes the page and all its blocks. ` +
        `The deletion syncs to paired devices.`,
    );
    if (!ok) return;
    try {
      const view = await deletePage(p.slug);
      props.onPick(view.page.slug, view.page.kind);
      refetch();
    } catch (e) {
      // Surface as a brief alert — the picker is already modal.
      window.alert(e instanceof Error ? e.message : String(e));
    }
  }

  /**
   * Pin / unpin a page (`TogglePin`, RFC 0254 phase 4b). Journals are
   * excluded at the call site — `toggle_pin` refuses them backend-side
   * too (a journal auto-rotates daily and would dilute the pinned
   * group with date-shaped junk). Refetches rather than patching the
   * `pages` resource in place: the list is small and a full re-list
   * keeps this in one code path with `handleDelete` above.
   */
  async function handleTogglePin(p: PageMeta) {
    try {
      await togglePin(p.id);
      refetch();
    } catch (e) {
      window.alert(e instanceof Error ? e.message : String(e));
    }
  }

  /**
   * Long-press detector for page rows (not journals). iOS context
   * menus fire on a sustained touch (~500ms) without significant
   * movement — the same gesture UIKit uses for interactive menus.
   * Returns Solid `onPointer*` handlers to spread onto the row.
   */
  function longPressHandlers(p: PageMeta) {
    if (p.kind === "journal") return {};
    let timer: ReturnType<typeof setTimeout> | undefined;
    let start: { x: number; y: number } | undefined;
    // Set when the long-press fires so the subsequent `click` event
    // (the browser always emits one when the finger lifts, even
    // after a sustained hold) is suppressed. Without this, both
    // `handleDelete` and `props.onPick` fire on the same gesture.
    let suppressedClick = false;
    const LONG_PRESS_MS = 500;
    const MOVE_TOLERANCE = 10;
    return {
      onPointerDown(e: PointerEvent) {
        start = { x: e.clientX, y: e.clientY };
        timer = setTimeout(() => {
          timer = undefined;
          suppressedClick = true;
          void handleDelete(p);
        }, LONG_PRESS_MS);
      },
      onPointerMove(e: PointerEvent) {
        if (!start || !timer) return;
        const dx = Math.abs(e.clientX - start.x);
        const dy = Math.abs(e.clientY - start.y);
        if (dx > MOVE_TOLERANCE || dy > MOVE_TOLERANCE) {
          clearTimeout(timer);
          timer = undefined;
        }
      },
      onPointerUp() {
        if (timer) {
          clearTimeout(timer);
          timer = undefined;
        }
      },
      onPointerCancel() {
        if (timer) {
          clearTimeout(timer);
          timer = undefined;
        }
      },
      // Captured BEFORE the row's `onClick` (React fires pointer
      // capture-phase before click bubble-phase). Eats the click
      // that follows a long-press so `onPick` doesn't also run.
      onClickCapture(e: MouseEvent) {
        if (suppressedClick) {
          e.preventDefault();
          e.stopPropagation();
          suppressedClick = false;
        }
      },
    };
  }

  return (
    <Show when={props.open}>
      <div
        class="outl-fade-in fixed inset-0 z-50 bg-black/40 backdrop-blur-md"
        onClick={props.onClose}
      />
      <div
        class="outl-sheet-up fixed inset-x-0 bottom-0 z-50 flex flex-col overflow-hidden rounded-t-2xl bg-(--color-outl-bg)/85 shadow-2xl backdrop-blur-2xl backdrop-saturate-150"
        style={{
          // Fullscreen sheet, iOS page-sheet style: anchored to the
          // bottom and stretching up to just below the status bar so
          // the rounded corners + grab handle stay visible.
          top: "calc(env(safe-area-inset-top) + 8px)",
          // When the keyboard is up, its inset already covers the
          // home-indicator safe area — take the max instead of
          // stacking both.
          "padding-bottom": `max(env(safe-area-inset-bottom), ${keyboardInset()}px)`,
          transform: `translateY(${drag.translateY()}px)`,
          transition: drag.dragging()
            ? "none"
            : "transform 220ms var(--ease-spring-in)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <header class="flex items-center gap-3 border-b border-(--color-outl-border)/30 px-4 py-3">
          {/* Drag-to-dismiss handle. We attach the drag handlers
              and `touch-action: none` ONLY here, not to the whole
              header — otherwise the user can't scroll the list
              from anywhere near the top of the sheet because the
              header steals the pointer. */}
          <span
            class="mx-auto block h-3 w-16 cursor-grab py-1 active:cursor-grabbing"
            style={{ "touch-action": "none" }}
            onPointerDown={drag.onPointerDown}
            onPointerMove={drag.onPointerMove}
            onPointerUp={drag.onPointerUp}
            onPointerCancel={drag.onPointerCancel}
            aria-label="Drag to close"
            role="button"
          >
            <span
              aria-hidden="true"
              class="block h-1 w-10 mx-auto rounded-full bg-(--color-outl-border)"
            />
          </span>
        </header>
        {/* Pages ↔ Blocks segmented switch (issue #19) — same visual
            pattern as `PropertiesSheet`'s Block/Page switch: one
            sheet, one search box, two scopes. */}
        <div class="mx-4 mt-1 flex rounded-lg bg-(--color-outl-border)/30 p-0.5">
          <For each={["pages", "blocks"] as SwitcherMode[]}>
            {(m) => (
              <button
                type="button"
                aria-pressed={mode() === m}
                onClick={() => setMode(m)}
                class="flex-1 rounded-[7px] py-1.5 text-[13px] font-medium capitalize"
                classList={{
                  "bg-(--color-outl-bg-elev) text-(--color-outl-fg) shadow-sm":
                    mode() === m,
                  "text-(--color-outl-fg-dim)": mode() !== m,
                }}
              >
                {m}
              </button>
            )}
          </For>
        </div>
        <div class="flex items-center gap-2 px-4 py-2">
          <div class="flex flex-1 items-center gap-2 rounded-xl bg-(--color-outl-bg-elev) px-3 py-2">
            <svg
              width="16"
              height="16"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              class="text-(--color-outl-fg-dim)"
              aria-hidden="true"
            >
              <path d="M21 21l-4.3-4.3M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16z" />
            </svg>
            <input
              ref={searchInput}
              type="text"
              autofocus
              placeholder={
                mode() === "blocks" ? "Search blocks…" : "Search pages…"
              }
              value={query()}
              onInput={(e) => setQuery(e.currentTarget.value)}
              onFocus={() => setSearchFocused(true)}
              onBlur={() => setSearchFocused(false)}
              onKeyDown={(e) => {
                // Enter opens the first match — same convention as
                // Spotlight / Raycast / Alfred. Esc dismisses.
                if (e.key === "Enter") {
                  e.preventDefault();
                  pickFirst();
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  props.onClose();
                }
              }}
              class="w-full bg-transparent text-[16px] outline-none"
            />
            <Show when={query().length > 0}>
              <button
                type="button"
                aria-label="Clear"
                onClick={() => {
                  setQuery("");
                  searchInput?.focus();
                }}
                class="text-(--color-outl-fg-dim) active:opacity-60"
              >
                <svg
                  width="16"
                  height="16"
                  viewBox="0 0 24 24"
                  fill="currentColor"
                  aria-hidden="true"
                >
                  <path d="M12 2a10 10 0 1 0 10 10A10 10 0 0 0 12 2zm5 13.59L15.59 17 12 13.41 8.41 17 7 15.59 10.59 12 7 8.41 8.41 7 12 10.59 15.59 7 17 8.41 13.41 12z" />
                </svg>
              </button>
            </Show>
          </div>
          {/* iOS UISearchController convention: a "Cancel" link
              appears flush right of the input *only while it has
              focus*, and a tap clears the query + dismisses. The
              outer flex sizing means the input shrinks to make room
              without reflowing the rest of the sheet. */}
          <Show when={searchFocused() || query().length > 0}>
            <button
              type="button"
              onClick={() => {
                setQuery("");
                searchInput?.blur();
                props.onClose();
              }}
              class="shrink-0 text-[15px] font-medium text-(--color-outl-accent) active:opacity-60"
            >
              Cancel
            </button>
          </Show>
        </div>
        <div class="ios-scroll flex-1 px-2 pb-4">
          <Show when={mode() === "pages"}>
          <Show
            when={filtered().length > 0}
            fallback={
              <p class="px-4 py-8 text-center text-[14px] text-(--color-outl-fg-dim)">
                No pages match "{query()}".
              </p>
            }
          >
            <For each={filtered()}>
              {(p) => (
                <div class="flex items-center gap-1">
                  <button
                    type="button"
                    onClick={() => props.onPick(p.slug, p.kind)}
                    {...longPressHandlers(p)}
                    class="flex min-w-0 flex-1 items-center gap-3 rounded-xl px-3 py-2.5 text-left active:bg-(--color-outl-bg-elev)/60"
                    classList={{
                      "bg-(--color-outl-bg-elev)/40":
                        p.slug === props.currentSlug,
                    }}
                  >
                    <span class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-(--color-outl-accent)/12 text-(--color-outl-accent)">
                      <Show
                        when={p.kind === "journal"}
                        fallback={
                          <svg
                            width="16"
                            height="16"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="2"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            aria-hidden="true"
                          >
                            <path d="M4 4h12a4 4 0 0 1 4 4v12H8a4 4 0 0 1-4-4V4z" />
                            <path d="M8 8h8M8 12h8M8 16h5" />
                          </svg>
                        }
                      >
                        <svg
                          width="16"
                          height="16"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          stroke-width="2"
                          stroke-linecap="round"
                          stroke-linejoin="round"
                          aria-hidden="true"
                        >
                          <rect x="3" y="4" width="18" height="18" rx="3" />
                          <path d="M3 10h18M8 2v4m8-4v4" />
                        </svg>
                      </Show>
                    </span>
                    <span class="flex min-w-0 flex-col">
                      <span class="truncate text-[15px] font-medium">
                        {p.title}
                      </span>
                      <span class="truncate text-[12px] text-(--color-outl-fg-dim)">
                        {p.slug}
                      </span>
                    </span>
                  </button>
                  {/* Pin toggle (RFC 0254 phase 4b, `TogglePin`) — a
                      sibling tap target, not nested inside the row
                      button above, so the two gestures can't collide
                      (same "two buttons in one flex row" convention the
                      desktop's backlink TODO toggle uses). Journals are
                      excluded: `toggle_pin` refuses them backend-side. */}
                  <Show when={p.kind !== "journal"}>
                    <button
                      type="button"
                      aria-label={p.pinned ? "Unpin page" : "Pin page"}
                      onClick={() => void handleTogglePin(p)}
                      class="flex h-9 w-9 shrink-0 items-center justify-center rounded-full active:bg-(--color-outl-border)/40"
                      classList={{
                        "text-(--color-outl-accent)": !!p.pinned,
                        "text-(--color-outl-fg-dimmer)": !p.pinned,
                      }}
                    >
                      <svg
                        width="16"
                        height="16"
                        viewBox="0 0 24 24"
                        fill={p.pinned ? "currentColor" : "none"}
                        stroke="currentColor"
                        stroke-width="2"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        aria-hidden="true"
                      >
                        <path d="M12 2l1.5 5.5L19 9l-4.5 3.5L16 18l-4-3-4 3 1.5-5.5L5 9l5.5-1.5z" />
                      </svg>
                    </button>
                  </Show>
                </div>
              )}
            </For>
          </Show>
          </Show>
          <Show when={mode() === "blocks"}>
            <Show
              when={blockHits().length > 0}
              fallback={
                <p class="px-4 py-8 text-center text-[14px] text-(--color-outl-fg-dim)">
                  No blocks match "{query()}".
                </p>
              }
            >
              <For each={blockHits()}>
                {(hit) => (
                  <button
                    type="button"
                    onClick={() => props.onJumpToBlock(hit)}
                    class="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left active:bg-(--color-outl-bg-elev)/60"
                  >
                    <span class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-(--color-outl-accent)/12 text-(--color-outl-accent)">
                      <svg
                        width="16"
                        height="16"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="2"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        aria-hidden="true"
                      >
                        <path d="M4 6h16M4 12h10M4 18h7" />
                      </svg>
                    </span>
                    <span class="flex min-w-0 flex-col">
                      <span class="truncate text-[15px] font-medium">
                        {hit.text}
                      </span>
                      <span class="truncate text-[12px] text-(--color-outl-fg-dim)">
                        {hit.source_slug}
                      </span>
                    </span>
                  </button>
                )}
              </For>
            </Show>
          </Show>
        </div>
      </div>
      <RefreshHook trigger={() => props.open} refetch={refetch} />
    </Show>
  );
}

// Trick to refetch every time `open` flips true without using effects
// outside the component body.
function RefreshHook(props: { trigger: () => boolean; refetch: () => void }) {
  createMemo(() => {
    if (props.trigger()) props.refetch();
  });
  return null;
}
