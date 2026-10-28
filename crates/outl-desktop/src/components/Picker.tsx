import {
  For,
  Show,
  createEffect,
  createSignal,
  onCleanup,
  onMount,
} from "solid-js";

import {
  openPageBySlug,
  openRef,
  searchBlocks,
  searchPages,
} from "@outl/shared/api/commands";
import type { PageView } from "@outl/shared/api/types";
import { findBlock } from "@outl/shared/outline";
import { PageGlyph, SearchIcon } from "@outl/shared/icons";

import { listShortcutBindings, type Action, type Chord } from "../lib/api";
import { formatSequence } from "../lib/chord-format";
import {
  BLOCK_QUERY_MIN,
  COMMAND_PREFIX,
  paletteActions,
  paletteResults,
  sectionOf,
  type PaletteItem,
} from "../lib/palette-results";
import type { ActionHandlers } from "../lib/shortcuts";
import { appState, setAppState } from "../lib/store";

/**
 * The palette: `Cmd/Ctrl+P`, and `Cmd/Ctrl+K` or `:` in Normal mode,
 * which open it on `>` (actions only). Pages, blocks and actions in
 * one list; see `lib/palette-results.ts` for what shows and in which
 * order.
 *
 * Page search:
 * A centered modal with a single text input and a fuzzy-ranked
 * results list. The backend already implements the ranking
 * (`search_pages` from `outl-mobile`'s original surface, now shared
 * via `@outl/shared/api/commands`) — three tiers: exact / prefix /
 * substring against `title` and `slug`.
 *
 * Enter → opens the highlighted result through `open_ref` (same
 * decision tree the inline `[[ref]]` click handler uses, so typing
 * a non-existent name creates the page in one round-trip).
 */
export function Picker(props: {
  onPicked: (view: PageView) => void;
  /** The dispatcher's handlers; an action row runs the same function its chord does. */
  handlers: ActionHandlers | null;
}) {
  const [query, setQuery] = createSignal("");
  const [results, setResults] = createSignal<PaletteItem<Chord>[]>([]);
  const [highlight, setHighlight] = createSignal(0);

  let inputRef: HTMLInputElement | undefined;

  function close() {
    setAppState("pickerOpen", false);
    setAppState("pickerSeed", null);
    setQuery("");
    setHighlight(0);
  }

  // `search_blocks` rebuilds the workspace index on every call, so it
  // runs debounced, after pages and actions have already painted, the
  // same 150 ms the `((` suggester uses.
  const BLOCK_DEBOUNCE_MS = 150;
  let blockTimer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(blockTimer));

  async function refresh(q: string) {
    clearTimeout(blockTimer);
    try {
      const commandsOnly = q.trim().startsWith(COMMAND_PREFIX);
      const [pages, bindings] = await Promise.all([
        commandsOnly ? [] : searchPages(q),
        listShortcutBindings(),
      ]);
      // A slower, older query must not overwrite a newer one.
      if (q !== query()) return;
      const actions = paletteActions(bindings, (k) => Boolean(props.handlers?.[k]));
      setResults(paletteResults<Chord>({ query: q, pages, blocks: [], actions }).items);
      setHighlight(0);
      if (commandsOnly || q.trim().length < BLOCK_QUERY_MIN) return;
      blockTimer = setTimeout(() => {
        searchBlocks(q.trim())
          .then((blocks) => {
            if (q !== query()) return;
            setResults(paletteResults<Chord>({ query: q, pages, blocks, actions }).items);
          })
          .catch((e) => setAppState("lastError", e instanceof Error ? e.message : String(e)));
      }, BLOCK_DEBOUNCE_MS);
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    }
  }

  // Refresh results whenever the query changes — including the
  // first mount with empty query (top 25 pages).
  createEffect(() => {
    void refresh(query());
  });

  async function pick(target: string) {
    try {
      const view = await openRef(target);
      props.onPicked(view);
      close();
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    }
  }

  async function choose(item: PaletteItem<Chord>) {
    if (item.kind === "page") return pick(item.page.slug);
    if (item.kind === "action") {
      close();
      await props.handlers?.[item.action as Action["kind"]]?.();
      return;
    }
    try {
      const view = await openPageBySlug(item.hit.source_slug);
      props.onPicked(view);
      setAppState("selectedBlockId", item.hit.id);
      // A hit with children opens zoomed into that block, so the
      // match is the whole view and not one row on a long page. A
      // leaf stays in page context with the cursor on it.
      const node = findBlock(appState.outline, item.hit.id);
      if (node && node.children.length > 0) setAppState("focusBlockId", item.hit.id);
      close();
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    }
  }

  function handleKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setHighlight((h) => Math.min(h + 1, Math.max(results().length - 1, 0)));
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      setHighlight((h) => Math.max(h - 1, 0));
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      const r = results();
      if (r.length > 0) void choose(r[highlight()]);
      else if (query().trim() && !query().trim().startsWith(COMMAND_PREFIX)) {
        void pick(query().trim()); // create new
      }
      return;
    }
  }

  /*
   * Focus the search input every time the picker is opened — not
   * only on the component's initial mount. The previous version
   * ran `inputRef?.focus()` inside `onMount`, which fires when the
   * Picker component is added to the tree (boot time, when
   * `pickerOpen === false` and `inputRef` is undefined because
   * `<Show>` hasn't rendered the input yet). Result: user pressed
   * Cmd+P, the input appeared but received no focus — they had to
   * click before typing. UX 101.
   *
   * `createEffect` re-runs whenever `pickerOpen` flips; the
   * microtask gives Solid a tick to commit the `<Show>` mount so
   * `inputRef` is populated by the time we call focus().
   */
  createEffect(() => {
    if (appState.pickerOpen) {
      // Pre-fill from `pickerSeed` if a handler stashed one (e.g. `*`
      // / `#` in vim Normal mode). Cleared on close so the next manual
      // `Cmd+P` opens blank.
      const seed = appState.pickerSeed;
      if (seed && query() === "") {
        setQuery(seed);
      }
      queueMicrotask(() => {
        inputRef?.focus();
        // Select all so the user can either accept the seed or type
        // a fresh query without an extra Backspace. Except the `>`
        // seed, which is a mode, not a query: typing must keep it.
        if (query().startsWith(COMMAND_PREFIX)) {
          inputRef?.setSelectionRange(query().length, query().length);
        } else {
          inputRef?.select();
        }
      });
    }
  });

  onMount(() => {
    // Click outside closes — handled by overlay div's onClick.
    const escListener = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        close();
      }
    };
    window.addEventListener("keydown", escListener, { capture: false });
    onCleanup(() => window.removeEventListener("keydown", escListener));
  });

  return (
    <Show when={appState.pickerOpen}>
      <div
        class="fixed inset-0 z-50 flex items-start justify-center bg-black/50 backdrop-blur-sm"
        onClick={(e) => {
          if (e.target === e.currentTarget) close();
        }}
      >
        <div class="mt-24 w-[520px] max-w-[90vw] overflow-hidden rounded-pop border border-(--color-outl-border)/60 bg-(--color-outl-bg-elev) shadow-pop">
          <div class="flex items-center gap-2 border-b border-(--color-outl-border)/60 px-4">
            <SearchIcon class="text-(--color-outl-fg-dimmer)" />
            <input
              ref={inputRef}
              type="text"
              value={query()}
              onInput={(e) => setQuery(e.currentTarget.value)}
              onKeyDown={handleKey}
              placeholder="Search pages, tasks and blocks, > for actions"
              class="w-full bg-transparent py-3 text-[15px] outline-none"
            />
          </div>
          <div class="max-h-[60vh] overflow-y-auto p-1">
            <For each={results()}>
              {(item, idx) => (
                <>
                  <Show when={idx() === 0 || sectionOf(results()[idx() - 1]) !== sectionOf(item)}>
                    <div class="px-3 pt-2 pb-1 text-[11px] font-medium text-(--color-outl-fg-dimmer)">
                      {sectionOf(item)}
                    </div>
                  </Show>
                  <button
                    type="button"
                    onClick={() => void choose(item)}
                    onMouseMove={() => setHighlight(idx())}
                    class={`flex w-full items-center gap-2 rounded-md px-3 py-1.5 text-left text-[13px] ${
                      idx() === highlight() ? "bg-(--color-outl-accent)/15" : ""
                    }`}
                  >
                    <PaletteRow item={item} />
                  </button>
                </>
              )}
            </For>
            <Show
              when={
                results().length === 0 &&
                query().trim().length > 0 &&
                !query().trim().startsWith(COMMAND_PREFIX)
              }
            >
              <button
                type="button"
                onClick={() => void pick(query().trim())}
                class="block w-full rounded-md px-3 py-1.5 text-left text-[13px] hover:bg-(--color-outl-fg)/5"
              >
                <span class="opacity-60">Create new page </span>
                <span class="font-medium">"{query().trim()}"</span>
              </button>
            </Show>
          </div>
          <div class="border-t border-(--color-outl-fg)/10 px-4 py-1.5 text-[10px] opacity-50">
            ↑/↓ navigate · ⏎ open · &gt; actions · Esc close
          </div>
        </div>
      </div>
    </Show>
  );
}

function PaletteRow(props: { item: PaletteItem<Chord> }) {
  const item = props.item;
  if (item.kind === "page") {
    return (
      <>
        <PageGlyph icon={item.page.icon} kind={item.page.kind} class="opacity-70" />
        <span class="truncate">{item.page.title}</span>
        <span class="ml-auto shrink-0 font-mono text-[11px] text-(--color-outl-fg-dimmer)">
          {item.page.slug}
        </span>
      </>
    );
  }
  if (item.kind === "task") {
    return (
      <>
        <span
          aria-label={item.state}
          class={`w-3.5 shrink-0 text-center ${item.state === "done" ? "text-(--color-outl-accent)" : "text-(--color-outl-fg-dimmer)"}`}
        >
          {item.state === "done" ? "☑" : item.state === "doing" ? "◐" : "☐"}
        </span>
        <span class={`truncate ${item.state === "done" ? "line-through opacity-60" : ""}`}>{item.hit.text}</span>
        <span class="ml-auto shrink-0 font-mono text-[11px] text-(--color-outl-fg-dimmer)">
          {item.hit.source_slug}
        </span>
      </>
    );
  }
  if (item.kind === "block") {
    return (
      <>
        <span aria-hidden="true" class="w-3.5 shrink-0 text-center text-(--color-outl-fg-dimmer)">
          •
        </span>
        <span class="truncate">{item.hit.text}</span>
        <span class="ml-auto shrink-0 font-mono text-[11px] text-(--color-outl-fg-dimmer)">
          {item.hit.source_slug}
        </span>
      </>
    );
  }
  return (
    <>
      <span class="truncate">{item.label}</span>
      <Show when={item.chord}>
        {(chord) => (
          <kbd class="ml-auto shrink-0 rounded border border-(--color-outl-border)/60 px-1.5 font-mono text-[11px] text-(--color-outl-fg-dim)">
            {formatSequence(chord())}
          </kbd>
        )}
      </Show>
    </>
  );
}
