import {
  For,
  Show,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
} from "solid-js";
import type { PageMeta } from "@outl/shared/api/types";
import {
  type BlockHit,
  deletePage,
  listPages,
  searchBlocks,
  searchPages,
  togglePin,
} from "@outl/shared/api/commands";
import {
  BLOCK_QUERY_MIN,
  COMMAND_PREFIX,
  paletteResults,
  sectionOf,
  type PaletteItem,
  type PaletteLimits,
  type PaletteSection,
} from "@outl/shared/palette";
import { PageGlyph } from "@outl/shared/icons";
import { createSheetDrag } from "../lib/sheet-drag";
import { MOBILE_ACTIONS, type MobileAction } from "../lib/palette-actions";
import { useKeyboardInset } from "../lib/viewport";

interface PageSwitcherProps {
  open: boolean;
  currentSlug: string | null;
  onClose: () => void;
  onPick: (slug: string, kind: "page" | "journal") => void;
  /** Fired when the user taps a task or block hit. The caller navigates
   *  to `hit.source_slug` — a `BlockHit` carries no `kind`, so the
   *  decision goes through `openRef`, same as any other tapped ref. */
  onJumpToBlock: (hit: BlockHit) => void;
  /** Runs a palette action; the sheet closes first. */
  onAction?: (action: MobileAction) => void;
}

/** Debounced like the desktop's palette (`Picker.tsx`): per call,
 *  `search_blocks` rebuilds the index from disk and `search_pages`
 *  rebuilds every `PageMeta` under the workspace lock. */
const SEARCH_DEBOUNCE_MS = 150;

/** A phone screen shows fewer rows than the desktop palette, so each
 *  section opens short and the rest sits behind its "N more". */
const MOBILE_LIMITS: PaletteLimits = { pages: 3, tasks: 3, blocks: 3, actions: 5 };
/** …except Pages with no query, which browses rather than ranks:
 *  journals newest first, then names A-Z (`list_all_pages`). A 3-row
 *  window on that is three recent days and nothing else, so browsing
 *  opens at the ranked path's own cap. */
const BROWSE_PAGES_LIMIT = 25;
const TASK_GLYPH = { todo: "☐", doing: "◐", done: "☑" } as const;
/** Rows a section grows by each time the user taps its "N more". */
const EXPANDED_STEP = 25;

/**
 * Bottom-sheet search: one box over pages, tasks, blocks and actions.
 * What is listed, in which order and under which limits is decided by
 * `@outl/shared/palette` (the desktop's `Cmd+P` palette reads the same
 * module); this sheet only fetches the candidates, draws a heading
 * where the section changes and a "N more" under any section whose
 * limit hid rows. A leading `>` narrows the list to actions.
 *
 * Tap a row to open it. A block hit zooms into that block (`openBlock`
 * in `Journal.tsx` sets `focusBlockId`), so it becomes the page rather
 * than a highlighted row somewhere in one. `Enter` opens the first row,
 * swipe down on the handle dismisses.
 *
 * Long-press a **page** row (not a journal) to surface a delete
 * confirmation. Journals are excluded: deleting a daily note by
 * accident from the picker would be a hostile surprise, and there's no
 * trash UI to recover from today.
 */
export function PageSwitcher(props: PageSwitcherProps) {
  const [query, setQuery] = createSignal("");
  const [pages, setPages] = createSignal<PageMeta[]>([]);
  const [blockHits, setBlockHits] = createSignal<BlockHit[]>([]);
  // Rows each section has been grown by, one `EXPANDED_STEP` per "N more" tap.
  const [grown, setGrown] = createSignal<ReadonlyMap<PaletteSection, number>>(new Map());
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  // A slower, older search must not overwrite a newer one.
  let searchToken = 0;
  onCleanup(() => clearTimeout(searchTimer));

  // No query lists the whole graph, so pinned pages sort in from anywhere;
  // a query takes the backend's ranked search.
  const loadPages = (q: string, token: number) =>
    (q ? searchPages(q) : listPages())
      .then((found) => {
        if (token === searchToken) setPages(found);
      })
      .catch(() => {
        if (token === searchToken) setPages([]);
      });

  // One query drives both backends on one debounce timer; blocks also
  // wait for the query to be long enough. Re-runs on `open` flips too.
  createEffect(() => {
    const q = query().trim();
    const open = props.open;
    const token = ++searchToken;
    clearTimeout(searchTimer);
    setGrown(new Map());
    setBlockHits([]);
    if (!open) return;
    if (q.startsWith(COMMAND_PREFIX)) {
      setPages([]);
      return;
    }
    // An empty box has no keystrokes behind it to coalesce.
    if (!q) return void loadPages(q, token);
    searchTimer = setTimeout(() => {
      void loadPages(q, token);
      if (q.length < BLOCK_QUERY_MIN) return;
      void searchBlocks(q)
        .then((hits) => {
          if (token === searchToken) setBlockHits(hits);
        })
        .catch(() => {
          if (token === searchToken) setBlockHits([]);
        });
    }, SEARCH_DEBOUNCE_MS);
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

  const results = createMemo(() => {
    const limit = (section: PaletteSection) =>
      (section === "pages" && !query().trim() ? BROWSE_PAGES_LIMIT : MOBILE_LIMITS[section]) +
      (grown().get(section) ?? 0);
    // Pinned pages surface first (RFC 0254 phase 4b, `TogglePin`) —
    // `Array.prototype.sort` is stable (ES2019+), so everything else
    // keeps the backend's ranking.
    const ranked = [...pages()].sort(
      (a, b) => Number(b.pinned ?? false) - Number(a.pinned ?? false),
    );
    return paletteResults({
      query: query(),
      pages: ranked,
      blocks: blockHits(),
      actions: props.onAction ? [...MOBILE_ACTIONS] : [],
      limits: {
        pages: limit("pages"),
        tasks: limit("tasks"),
        blocks: limit("blocks"),
        actions: limit("actions"),
      },
    });
  });

  const sectionKey = (item: PaletteItem): PaletteSection =>
    sectionOf(item).toLowerCase() as PaletteSection;

  function expand(section: PaletteSection) {
    setGrown((prev) => new Map(prev).set(section, (prev.get(section) ?? 0) + EXPANDED_STEP));
  }

  /** Refetch after a pin or delete changed what a page row shows. */
  function reloadPages() {
    if (query().trim().startsWith(COMMAND_PREFIX)) return;
    void loadPages(query().trim(), searchToken);
  }

  function choose(item: PaletteItem) {
    switch (item.kind) {
      case "page":
        props.onPick(item.page.slug, item.page.kind);
        break;
      case "action":
        // The rows come from `MOBILE_ACTIONS`, typed to `MobileAction`.
        runAction(item.action as MobileAction);
        break;
      default:
        props.onJumpToBlock(item.hit);
    }
  }

  function runAction(action: MobileAction) {
    setQuery("");
    props.onClose();
    props.onAction?.(action);
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
      reloadPages();
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
      reloadPages();
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

  /** One row of the sectioned list. `item` is a plain value (the `<For>`
   *  re-creates the row when it changes), so reading its fields here is
   *  not a destructured prop. */
  function renderRow(item: PaletteItem) {
    if (item.kind === "page") {
      const page = item.page;
      return (
        <div class="flex items-center gap-1">
          <button
            type="button"
            onClick={() => props.onPick(page.slug, page.kind)}
            {...longPressHandlers(page)}
            class="flex min-w-0 flex-1 items-center gap-3 rounded-xl px-3 py-2.5 text-left active:bg-(--color-outl-bg-elev)/60"
            classList={{
              "bg-(--color-outl-bg-elev)/40":
                page.slug === props.currentSlug,
            }}
          >
            <span class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-(--color-outl-accent)/12 text-(--color-outl-accent)">
              <PageGlyph icon={page.icon} kind={page.kind} />
            </span>
            <span class="flex min-w-0 flex-col">
              <span class="truncate text-[15px] font-medium">
                {page.title}
              </span>
              <span class="truncate text-[12px] text-(--color-outl-fg-dim)">
                {page.slug}
              </span>
            </span>
          </button>
          {/* Pin toggle (RFC 0254 phase 4b, `TogglePin`) — a
              sibling tap target, not nested inside the row
              button above, so the two gestures can't collide
              (same "two buttons in one flex row" convention the
              desktop's backlink TODO toggle uses). Journals are
              excluded: `toggle_pin` refuses them backend-side. */}
          <Show when={page.kind !== "journal"}>
            <button
              type="button"
              aria-label={page.pinned ? "Unpin page" : "Pin page"}
              onClick={() => void handleTogglePin(page)}
              class="flex h-9 w-9 shrink-0 items-center justify-center rounded-full active:bg-(--color-outl-border)/40"
              classList={{
                "text-(--color-outl-accent)": !!page.pinned,
                "text-(--color-outl-fg-dimmer)": !page.pinned,
              }}
            >
              <svg
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill={page.pinned ? "currentColor" : "none"}
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
      );
    }
    if (item.kind === "action") {
      return (
        <button
          type="button"
          data-action={item.action}
          class="flex w-full items-center gap-3 rounded-xl px-4 py-3 text-left text-[15px] active:bg-(--color-outl-accent)/10"
          onClick={() => choose(item)}
        >
          <span aria-hidden="true" class="text-(--color-outl-accent)">›</span>
          <span class="flex-1 truncate">{item.label}</span>
        </button>
      );
    }
    const hit = item.hit;
    return (
        <button
          type="button"
          onClick={() => choose(item)}
          class="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left active:bg-(--color-outl-bg-elev)/60"
        >
          <span class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-(--color-outl-accent)/12 text-(--color-outl-accent)">
            {item.kind === "task" ? (
              <span aria-label={item.state} class="text-[16px] leading-none">
                {TASK_GLYPH[item.state]}
              </span>
            ) : (
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
            )}
          </span>
          <span class="flex min-w-0 flex-col">
            <span
              class="truncate text-[15px] font-medium"
              classList={{ "line-through opacity-60": item.kind === "task" && item.state === "done" }}
            >
              {hit.text}
            </span>
            <span class="truncate text-[12px] text-(--color-outl-fg-dim)">
              {hit.source_slug}
            </span>
          </span>
        </button>
    );
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
              placeholder="Search pages, tasks and blocks, > for actions"
              value={query()}
              onInput={(e) => setQuery(e.currentTarget.value)}
              onFocus={() => setSearchFocused(true)}
              onBlur={() => setSearchFocused(false)}
              onKeyDown={(e) => {
                // Enter opens the first match — same convention as
                // Spotlight / Raycast / Alfred. Esc dismisses.
                if (e.key === "Enter") {
                  e.preventDefault();
                  const first = results().items[0];
                  if (first) choose(first);
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
          <For each={results().items}>
            {(item, idx) => {
              const first = () => idx() === 0 || sectionOf(results().items[idx() - 1]) !== sectionOf(item);
              const last = () =>
                idx() === results().items.length - 1 ||
                sectionOf(results().items[idx() + 1]) !== sectionOf(item);
              const hiddenCount = () => results().hidden[sectionKey(item)];
              return (
                <>
                  <Show when={first()}>
                    <p class="px-4 pt-3 pb-1 text-[12px] font-medium text-(--color-outl-fg-dim)">
                      {sectionOf(item)}
                    </p>
                  </Show>
                  {renderRow(item)}
                  <Show when={last() && hiddenCount() > 0}>
                    <button
                      type="button"
                      onClick={() => expand(sectionKey(item))}
                      aria-label={`${hiddenCount()} more ${sectionKey(item)}`}
                      class="min-h-11 w-full px-4 text-left text-[13px] font-medium text-(--color-outl-accent) active:opacity-60"
                    >
                      {hiddenCount()} more
                    </button>
                  </Show>
                </>
              );
            }}
          </For>
          <Show when={results().items.length === 0}>
            <p class="px-4 py-8 text-center text-[14px] text-(--color-outl-fg-dim)">
              {query().trim() ? `Nothing matches "${query().trim()}".` : "No pages yet."}
            </p>
          </Show>
        </div>
      </div>
    </Show>
  );
}
