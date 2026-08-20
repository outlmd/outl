import {
  For,
  JSX,
  Show,
  createEffect,
  createMemo,
  createSignal,
  on,
} from "solid-js";

import type { PageMeta, PageView } from "@outl/shared/api/types";
import { searchPages, setBlockProperty } from "@outl/shared/api/commands";
import {
  applySuggestion,
  detectRefContext,
  refReplacement,
} from "@outl/shared/autocomplete";
import { findBlock } from "@outl/shared/outline";

import {
  knownPropertyKeys,
  setPageProperty,
  type PropertyKey,
} from "../lib/api";
import {
  editableProperties,
  normalizeKey,
  suggestedKeys,
} from "../lib/properties";
import { createSheetDrag } from "../lib/sheet-drag";
import { haptic } from "../lib/haptics";
import { SwipeRow } from "./SwipeRow";

/** What the sheet is editing: one block's properties, or the page's. */
export type PropertyScope = "block" | "page";

export interface PropertiesSheetProps {
  /**
   * Block whose properties the sheet opens on, or `null` when it was
   * opened from the page header. A non-null id also enables the
   * Block/Page switch — the page's own metadata is one tap away from
   * any block, which is the only place mobile surfaces it.
   */
  blockId: string | null;
  /** Which side the sheet opens on. `null` keeps it closed. */
  scope: PropertyScope | null;
  /** Current page's node id. Every write needs it. */
  pageId: string | null;
  /** The open page — where the sheet reads the current properties. */
  view: PageView | null;
  onClose: () => void;
  /** Toast channel (a refused key, a stale block, a failed write). */
  onMessage: (text: string) => void;
  /** Refreshed page view after a write. */
  onView: (view: PageView) => void;
}

/** Which screen of the sheet is showing. */
type Mode =
  | { kind: "list" }
  /** Picking the key for a new property (chips, or the keyboard). */
  | { kind: "key" }
  /** Editing one value. `existing` is false for a property being added. */
  | { kind: "value"; key: string; existing: boolean };

/**
 * Bottom sheet for `key:: value` properties, opened from the block
 * long-press menu ("Properties…") and from the page header's chips.
 *
 * Why a sheet and not the inline chip editing the outline already has:
 * a 390px screen with the keyboard covering half of it has no room for
 * a chip row that grows a `+` and two inputs. More importantly, the
 * measured shape of the data says the keyboard is the wrong default —
 * a real graph reuses about a dozen keys (`icon`, `related`, `status`,
 * `oura-date`), so the Add step shows **the workspace's own keys as
 * tappable chips** and typing is the escape hatch, not the path.
 * Two taps, no keyboard, for the common case.
 *
 * Values lean the same way: 87% of them are a `[[page]]` or a `#tag`,
 * so typing `[[` inside the value field opens the same page
 * autocomplete the block editor uses (shared `detectRefContext` /
 * `applySuggestion` / `refReplacement` — the accept rule is not
 * re-implemented here), and a "Link page" button types the `[[` for
 * you.
 *
 * Deleting is an explicit action in two shapes: swipe a row (the
 * gesture iOS already taught) and a Delete button inside the value
 * editor. Both write an empty value, which is how `set_property`
 * spells "remove" — the *user* never has to know that.
 */
export function PropertiesSheet(props: PropertiesSheetProps): JSX.Element {
  const drag = createSheetDrag(() => props.onClose());
  const [scope, setScope] = createSignal<PropertyScope>("page");
  const [mode, setMode] = createSignal<Mode>({ kind: "list" });
  const [catalogue, setCatalogue] = createSignal<PropertyKey[]>([]);
  const [busy, setBusy] = createSignal(false);
  // Set when the user taps "Other…" — the key chips give way to an
  // input. Kept apart from `mode` so backing out of the keyboard
  // returns to the chips instead of closing the Add step.
  const [typingKey, setTypingKey] = createSignal(false);
  const [keyDraft, setKeyDraft] = createSignal("");
  const [valueDraft, setValueDraft] = createSignal("");
  const [suggestions, setSuggestions] = createSignal<PageMeta[]>([]);
  let valueInput: HTMLInputElement | undefined;
  // Guards the async page search: a slow reply for an old query must
  // not overwrite the chips for the one the user is looking at.
  let queryToken = 0;

  const open = () => props.scope !== null;

  // Reset every time the sheet opens, and refresh the key catalogue.
  // Keys are added by any client at any time, so a cached list would
  // suggest a stale set — and it is a map scan, not a tree walk.
  createEffect(
    on(
      () => props.scope,
      (next) => {
        if (next === null) return;
        setScope(next);
        setMode({ kind: "list" });
        setTypingKey(false);
        setKeyDraft("");
        setValueDraft("");
        setSuggestions([]);
        void knownPropertyKeys()
          .then(setCatalogue)
          .catch(() => setCatalogue([])); // chips are a nicety; typing still works
      },
    ),
  );

  const properties = createMemo(() => {
    const view = props.view;
    if (!view) return [];
    if (scope() === "page") return editableProperties(view.page_properties);
    const id = props.blockId;
    const block = id ? findBlock(view.outline, id) : null;
    return editableProperties(block?.properties);
  });

  const keyChips = createMemo(() =>
    suggestedKeys(
      catalogue(),
      properties().map(([key]) => key),
    ),
  );

  /**
   * Write one property and swap in the refreshed view.
   *
   * An empty `value` is a delete — the backend's own spelling for it
   * (`set_property` with `None`), so both affordances funnel here.
   */
  async function write(key: string, value: string) {
    const pid = props.pageId;
    if (!pid || busy()) return;
    const clean = normalizeKey(key);
    if (!clean) {
      props.onMessage("A property needs a key.");
      return;
    }
    setBusy(true);
    try {
      const blockId = props.blockId;
      const view =
        scope() === "page" || !blockId
          ? await setPageProperty(pid, clean, value)
          : await setBlockProperty(pid, blockId, clean, value);
      props.onView(view);
      haptic(value.trim() === "" ? "warning" : "light");
      setMode({ kind: "list" });
    } catch (e) {
      // A refused key (`page-slug`) and a stale block both land here,
      // and both are things the user has to see: the row repaints from
      // the view either way, so a swallowed failure reads as success.
      props.onMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  function startAdd() {
    haptic("light");
    setTypingKey(false);
    setKeyDraft("");
    setValueDraft("");
    setSuggestions([]);
    setMode({ kind: "key" });
  }

  function pickKey(key: string) {
    // Normalise here, not at write time, so the value step shows the
    // key the way it will be stored (`oura-date::` typed → `oura-date`).
    const clean = normalizeKey(key);
    if (!clean) return;
    haptic("light");
    setValueDraft("");
    setSuggestions([]);
    setMode({ kind: "value", key: clean, existing: false });
  }

  function startEdit(key: string, value: string) {
    haptic("light");
    setValueDraft(value);
    setSuggestions([]);
    setMode({ kind: "value", key, existing: true });
  }

  /** Refresh the `[[…]]` page suggestions for the caret position. */
  function refreshSuggestions(el: HTMLInputElement) {
    const ctx = detectRefContext(el.value, el.selectionStart ?? el.value.length);
    if (!ctx || ctx.kind !== "page") {
      queryToken += 1; // invalidate any reply still in flight
      setSuggestions([]);
      return;
    }
    const token = ++queryToken;
    void searchPages(ctx.query)
      .then((items) => {
        if (token === queryToken) setSuggestions(items);
      })
      .catch(() => {
        if (token === queryToken) setSuggestions([]);
      });
  }

  function onValueInput(e: InputEvent & { currentTarget: HTMLInputElement }) {
    setValueDraft(e.currentTarget.value);
    refreshSuggestions(e.currentTarget);
  }

  /** Accept a page chip into the open `[[…]]` at the caret. */
  function acceptSuggestion(page: PageMeta) {
    const el = valueInput;
    if (!el) return;
    const ctx = detectRefContext(el.value, el.selectionStart ?? el.value.length);
    if (!ctx) return;
    // `refReplacement` owns the journals-insert-their-slug rule; the
    // block editor's accept path calls the same function.
    const result = applySuggestion(el.value, ctx, refReplacement(page));
    el.value = result.value;
    setValueDraft(result.value);
    el.setSelectionRange(result.caret, result.caret);
    el.focus();
    setSuggestions([]);
  }

  /** Type the `[[` for the user — 87% of values are a page link. */
  function insertRefTrigger() {
    const el = valueInput;
    if (!el) return;
    const at = el.selectionStart ?? el.value.length;
    el.value = `${el.value.slice(0, at)}[[${el.value.slice(at)}`;
    setValueDraft(el.value);
    el.setSelectionRange(at + 2, at + 2);
    el.focus();
    refreshSuggestions(el);
  }

  const title = () =>
    mode().kind === "key"
      ? "Add property"
      : scope() === "page"
        ? "Page properties"
        : "Block properties";

  return (
    <Show when={open()}>
      <div
        class="outl-fade-in fixed inset-0 z-[55] bg-black/40 backdrop-blur-md"
        onClick={props.onClose}
      />
      <div
        class="outl-sheet-up fixed inset-x-0 bottom-0 z-[55] flex flex-col"
        style={{
          "padding-bottom": "max(env(safe-area-inset-bottom), 16px)",
          transform: `translateY(${drag.translateY()}px)`,
          transition: drag.dragging()
            ? "none"
            : "transform 220ms var(--ease-spring-in)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div class="mx-3 mb-2 overflow-hidden rounded-2xl bg-(--color-ios-card)/95 shadow-[var(--shadow-capsule)] backdrop-blur-2xl dark:bg-(--color-iosd-card)/95 dark:shadow-[var(--shadow-capsule-dark)]">
          <span
            class="block py-2"
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
              class="mx-auto block h-1 w-10 rounded-full bg-(--color-ios-divider) dark:bg-(--color-iosd-divider)"
            />
          </span>

          <div class="px-4 pb-1 pt-1">
            <span class="text-[13px] font-semibold uppercase tracking-wide text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
              {title()}
            </span>
          </div>

          {/* Block ↔ Page switch. Only meaningful when the sheet was
              opened from a block: page properties (`icon::`, `type::`)
              had no GUI surface at all before this, and hanging them
              off the same sheet keeps them one tap from anywhere. */}
          <Show when={props.blockId !== null && mode().kind === "list"}>
            <div class="mx-4 mb-2 mt-1 flex rounded-lg bg-(--color-ios-divider)/30 p-0.5 dark:bg-(--color-iosd-divider)/30">
              <For each={["block", "page"] as PropertyScope[]}>
                {(s) => (
                  <button
                    type="button"
                    aria-pressed={scope() === s}
                    onClick={() => {
                      haptic("light");
                      setScope(s);
                    }}
                    class="flex-1 rounded-[7px] py-1.5 text-[13px] font-medium capitalize"
                    classList={{
                      "bg-(--color-ios-card) text-(--color-ios-text) shadow-sm dark:bg-(--color-iosd-card) dark:text-(--color-iosd-text)":
                        scope() === s,
                      "text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)":
                        scope() !== s,
                    }}
                  >
                    {s}
                  </button>
                )}
              </For>
            </div>
          </Show>

          <div class="max-h-[55vh] overflow-y-auto">
            {/* ── List ──────────────────────────────────────────── */}
            <Show when={mode().kind === "list"}>
              <button
                type="button"
                disabled={busy()}
                aria-label="Add property"
                onClick={startAdd}
                class="flex w-full items-center gap-2 border-t border-(--color-ios-divider)/30 px-4 py-3.5 text-left text-[16px] font-medium text-(--color-ios-accent) active:bg-(--color-ios-divider)/30 disabled:opacity-50 dark:border-(--color-iosd-divider)/30 dark:text-(--color-iosd-accent) dark:active:bg-(--color-iosd-divider)/30"
              >
                <span aria-hidden="true" class="text-[18px] leading-none">
                  +
                </span>
                Add property
              </button>

              <For each={properties()}>
                {([key, value]) => (
                  <SwipeRow
                    leftActionLabel="Delete"
                    onSwipeLeft={() => void write(key, "")}
                  >
                    <button
                      type="button"
                      disabled={busy()}
                      aria-label={`Edit ${key}`}
                      onClick={() => startEdit(key, value)}
                      class="flex w-full items-baseline gap-3 border-t border-(--color-ios-divider)/30 px-4 py-3.5 text-left active:bg-(--color-ios-divider)/30 disabled:opacity-50 dark:border-(--color-iosd-divider)/30 dark:active:bg-(--color-iosd-divider)/30"
                    >
                      <span class="shrink-0 font-mono text-[13px] text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
                        {key}
                      </span>
                      <span class="min-w-0 flex-1 truncate text-right text-[15px] text-(--color-ios-text) dark:text-(--color-iosd-text)">
                        {value}
                      </span>
                    </button>
                  </SwipeRow>
                )}
              </For>

              <Show when={properties().length === 0}>
                <div class="border-t border-(--color-ios-divider)/30 px-4 py-6 text-center text-[14px] text-(--color-ios-text-secondary) dark:border-(--color-iosd-divider)/30 dark:text-(--color-iosd-text-secondary)">
                  No properties yet.
                </div>
              </Show>
            </Show>

            {/* ── Key picker ────────────────────────────────────── */}
            <Show when={mode().kind === "key"}>
              <Show
                when={typingKey()}
                fallback={
                  <div class="border-t border-(--color-ios-divider)/30 px-4 py-3 dark:border-(--color-iosd-divider)/30">
                    <p class="mb-2 text-[13px] text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
                      Keys already used in this workspace
                    </p>
                    <div class="flex flex-wrap gap-2">
                      <For each={keyChips()}>
                        {(key) => (
                          <button
                            type="button"
                            onClick={() => pickKey(key)}
                            class="rounded-full bg-(--color-ios-divider)/40 px-3 py-1.5 font-mono text-[13px] text-(--color-ios-text) active:opacity-60 dark:bg-(--color-iosd-divider)/40 dark:text-(--color-iosd-text)"
                          >
                            {key}
                          </button>
                        )}
                      </For>
                      <button
                        type="button"
                        onClick={() => {
                          haptic("light");
                          setTypingKey(true);
                        }}
                        class="rounded-full border border-(--color-ios-accent)/50 px-3 py-1.5 text-[13px] text-(--color-ios-accent) active:opacity-60 dark:border-(--color-iosd-accent)/50 dark:text-(--color-iosd-accent)"
                      >
                        Other…
                      </button>
                    </div>
                  </div>
                }
              >
                <div class="border-t border-(--color-ios-divider)/30 px-4 py-3 dark:border-(--color-iosd-divider)/30">
                  <input
                    type="text"
                    // eslint-disable-next-line jsx-a11y/no-autofocus
                    autofocus
                    autocapitalize="none"
                    autocomplete="off"
                    spellcheck={false}
                    placeholder="property key"
                    aria-label="Property key"
                    value={keyDraft()}
                    onInput={(e) => setKeyDraft(e.currentTarget.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && normalizeKey(keyDraft())) {
                        e.preventDefault();
                        pickKey(keyDraft());
                      }
                    }}
                    class="w-full rounded-lg border border-(--color-ios-divider) bg-(--color-ios-card) px-3 py-2 font-mono text-[15px] text-(--color-ios-text) outline-none dark:border-(--color-iosd-divider) dark:bg-(--color-iosd-card) dark:text-(--color-iosd-text)"
                  />
                  <div class="mt-3 flex gap-2">
                    <button
                      type="button"
                      onClick={() => setTypingKey(false)}
                      class="flex-1 rounded-lg bg-(--color-ios-divider)/40 py-2 text-[15px] font-medium text-(--color-ios-text) active:opacity-60 dark:bg-(--color-iosd-divider)/40 dark:text-(--color-iosd-text)"
                    >
                      Back
                    </button>
                    <button
                      type="button"
                      disabled={normalizeKey(keyDraft()) === ""}
                      onClick={() => pickKey(keyDraft())}
                      class="flex-1 rounded-lg bg-(--color-ios-accent) py-2 text-[15px] font-semibold text-white active:opacity-70 disabled:opacity-40 dark:bg-(--color-iosd-accent)"
                    >
                      Next
                    </button>
                  </div>
                </div>
              </Show>
            </Show>

            {/* ── Value editor ──────────────────────────────────── */}
            <Show when={mode().kind === "value" ? (mode() as Mode & { kind: "value" }) : null}>
              {(m) => (
                <div class="border-t border-(--color-ios-divider)/30 px-4 py-3 dark:border-(--color-iosd-divider)/30">
                  <p class="mb-2 font-mono text-[13px] text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
                    {m().key}::
                  </p>
                  <input
                    ref={(el) => (valueInput = el)}
                    type="text"
                    // eslint-disable-next-line jsx-a11y/no-autofocus
                    autofocus
                    autocapitalize="none"
                    autocomplete="off"
                    spellcheck={false}
                    placeholder="value"
                    aria-label={`Value for ${m().key}`}
                    value={valueDraft()}
                    onInput={onValueInput}
                    onClick={(e) => refreshSuggestions(e.currentTarget)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        e.preventDefault();
                        void write(m().key, valueDraft());
                      }
                    }}
                    class="w-full rounded-lg border border-(--color-ios-divider) bg-(--color-ios-card) px-3 py-2 text-[15px] text-(--color-ios-text) outline-none dark:border-(--color-iosd-divider) dark:bg-(--color-iosd-card) dark:text-(--color-iosd-text)"
                  />

                  {/* Page chips for the open `[[…]]`. Same detector and
                      same accept rule as the block editor's suggester. */}
                  <Show when={suggestions().length > 0}>
                    <div class="ios-scroll mt-2 flex gap-1.5 overflow-x-auto pb-1">
                      <For each={suggestions()}>
                        {(page) => (
                          <button
                            type="button"
                            onPointerDown={(e) => e.preventDefault()}
                            onClick={() => acceptSuggestion(page)}
                            class="shrink-0 rounded-full bg-(--color-ios-divider)/40 px-3 py-1 text-[13px] text-(--color-ios-text) active:opacity-60 dark:bg-(--color-iosd-divider)/40 dark:text-(--color-iosd-text)"
                          >
                            {page.kind === "journal" ? page.slug : page.title}
                          </button>
                        )}
                      </For>
                    </div>
                  </Show>

                  <button
                    type="button"
                    onPointerDown={(e) => e.preventDefault()}
                    onClick={insertRefTrigger}
                    class="mt-2 text-[13px] text-(--color-ios-accent) active:opacity-60 dark:text-(--color-iosd-accent)"
                  >
                    Link a page…
                  </button>

                  <div class="mt-3 flex gap-2">
                    <button
                      type="button"
                      onClick={() => setMode({ kind: "list" })}
                      class="flex-1 rounded-lg bg-(--color-ios-divider)/40 py-2 text-[15px] font-medium text-(--color-ios-text) active:opacity-60 dark:bg-(--color-iosd-divider)/40 dark:text-(--color-iosd-text)"
                    >
                      Cancel
                    </button>
                    <Show when={m().existing}>
                      <button
                        type="button"
                        disabled={busy()}
                        onClick={() => void write(m().key, "")}
                        class="flex-1 rounded-lg bg-(--color-ios-destructive)/15 py-2 text-[15px] font-semibold text-(--color-ios-destructive) active:opacity-60 disabled:opacity-40 dark:text-(--color-iosd-destructive)"
                      >
                        Delete
                      </button>
                    </Show>
                    <button
                      type="button"
                      disabled={busy()}
                      onClick={() => void write(m().key, valueDraft())}
                      class="flex-1 rounded-lg bg-(--color-ios-accent) py-2 text-[15px] font-semibold text-white active:opacity-70 disabled:opacity-40 dark:bg-(--color-iosd-accent)"
                    >
                      Save
                    </button>
                  </div>
                </div>
              )}
            </Show>
          </div>
        </div>

        <button
          type="button"
          onClick={props.onClose}
          class="mx-3 rounded-2xl bg-(--color-ios-card)/95 py-3.5 text-center text-[16px] font-semibold text-(--color-ios-accent) shadow-[var(--shadow-capsule)] backdrop-blur-2xl active:bg-(--color-ios-divider)/30 dark:bg-(--color-iosd-card)/95 dark:text-(--color-iosd-accent) dark:shadow-[var(--shadow-capsule-dark)] dark:active:bg-(--color-iosd-divider)/30"
        >
          Done
        </button>
      </div>
    </Show>
  );
}
