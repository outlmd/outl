/**
 * `key:: value` properties, as an editable chip row.
 *
 * The shared `<BlockProperties />` renders chips and edits a value in
 * place, which covers exactly half the job: you could change a
 * property that already existed and had no way to make the first one.
 * Creating a property in either GUI client was impossible — the only
 * routes were the TUI's `/prop` or opening the `.md` by hand (issue
 * #13). Deleting was worse: the gesture existed (empty the value) and
 * nothing on screen said so.
 *
 * So this adds the two missing verbs as visible affordances — a `+` at
 * the end of the row and an `×` on each chip — and makes the key field
 * complete from the workspace's own catalogue, because in a real graph
 * keys are few and repeat (`icon`, `related`, `status`, `url`); a blank
 * text field asks the user to retype what the workspace already knows.
 *
 * Presentational: no store, no command. The host passes `onCommit` and
 * decides whether that writes to a block or to the page — the two are
 * the same editor precisely because they are the same data shape.
 *
 * **Desktop-local on purpose, for now.** It belongs in `@outl/shared`
 * next to `BlockProperties`, but mobile's answer to the same issue is
 * a sheet with tappable key chips, not an inline row — promoting this
 * before that lands would freeze a desktop-shaped API into the shared
 * lib. See the note at the bottom of this file.
 */

import { For, Show, createEffect, createSignal, createMemo, on, type JSX } from "solid-js";

import { propertyChips } from "@outl/shared/markdown";
import { applySuggestion, detectRefContext } from "@outl/shared/autocomplete";
import { searchPages } from "@outl/shared/api/commands";
import type { PageMeta } from "@outl/shared/api/types";

import { knownPropertyKeys, type PropertyKey } from "../lib/api";
import { handlePopupNav } from "../lib/popup-nav";

/** How many catalogue entries the key popup shows at once. */
const MAX_SUGGESTIONS = 8;

/**
 * A `[[page]]` suggestion open over a value field.
 *
 * Property *values* are overwhelmingly refs — measured at 87% of them
 * on a real graph (issue #13) — so a plain text field here means the
 * common case is typing a page name from memory with nothing checking
 * it. This reuses the same `detectRefContext` → `searchPages` →
 * `applySuggestion` chain the block editor uses, so the trigger, the
 * accept and the `[[…]]` wrapping cannot drift from it.
 */
interface ValueSuggest {
  /** Which field is open: the inline chip editor, or the add row. */
  field: "edit" | "add";
  hits: PageMeta[];
  index: number;
}

export interface PropertyEditorProps {
  /** `(key, value)` pairs, alpha-sorted by the backend. */
  properties?: ReadonlyArray<readonly [string, string]>;
  /**
   * Write `key`. An empty `value` clears the property — that is the
   * backend contract for both `set_block_property` and
   * `set_page_property`, so `×` and "empty the field" are the same op.
   */
  onCommit: (key: string, value: string) => void | Promise<void>;
  /**
   * Surface a failed write. Omit and a rejection is dropped, which is
   * the failure worth naming: the chip repaints either way, so a
   * silent error reads as a successful edit.
   */
  onError?: (message: string) => void;
  /**
   * Open a blank editor from outside — the `Cmd/Ctrl+Shift+P` chord,
   * which has no chip to click. Rising edge opens it; the component
   * calls {@link onAddOpenChange} with `false` when it closes so the
   * host can clear its signal and the next press re-fires.
   */
  addOpen?: boolean;
  onAddOpenChange?: (open: boolean) => void;
  /**
   * `"hover"` keeps the `+` out of the reading surface until the
   * parent `.group` is hovered (block rows, where a permanently
   * visible button on every line is noise). `"always"` pins it — the
   * page header has one row, and a hidden affordance there is the
   * undiscoverable gesture this component exists to remove.
   */
  addAffordance?: "hover" | "always";
  /** Noun used in titles / labels: `"property"` or `"page property"`. */
  noun?: string;
  chipClass?: string;
  inputClass?: string;
}

export function PropertyEditor(props: PropertyEditorProps): JSX.Element {
  // One editor open at a time: a row of live inputs is noise, and
  // committing on blur would commit the first the moment you opened
  // the second anyway.
  const [editingKey, setEditingKey] = createSignal<string | null>(null);
  const [draft, setDraft] = createSignal("");

  const [adding, setAdding] = createSignal(false);
  const [newKey, setNewKey] = createSignal("");
  const [newValue, setNewValue] = createSignal("");

  const [catalogue, setCatalogue] = createSignal<PropertyKey[]>([]);
  const [suggestIndex, setSuggestIndex] = createSignal(0);
  const [suggestOpen, setSuggestOpen] = createSignal(false);

  // `[[page]]` completion over whichever value field is focused.
  const [valueSuggest, setValueSuggest] = createSignal<ValueSuggest | null>(null);
  let valueSearchToken = 0;

  /** Run the ref-completion chain over a value field's current text.
   *
   *  Nothing opens unless the caret actually sits in a `[[…` — typing
   *  a plain value never summons a popup. `((block))` and `@mention`
   *  are ignored on purpose: a property value naming a block is not a
   *  thing the dialect resolves, and `@` is sugar for a page ref the
   *  `[[` path already covers. */
  async function refreshValueSuggest(field: "edit" | "add", text: string, caret: number) {
    // Bump first: leaving without it lets a search fired for an
    // earlier `[[fo` resolve after the user deleted the brackets and
    // reopen the popup over a field that no longer has a ref context.
    const token = ++valueSearchToken;
    const ctx = detectRefContext(text, caret);
    if (!ctx || ctx.kind !== "page") {
      setValueSuggest(null);
      return;
    }
    try {
      const hits = await searchPages(ctx.query);
      // A newer keystroke already fired: its result is the current one.
      if (token !== valueSearchToken) return;
      setValueSuggest(hits.length > 0 ? { field, hits, index: 0 } : null);
    } catch {
      setValueSuggest(null);
    }
  }

  /** Accept the highlighted page into the field, wrapped as `[[…]]`. */
  function acceptValueSuggest(
    field: "edit" | "add",
    text: string,
    caret: number,
    input: HTMLInputElement | undefined,
  ): boolean {
    const sug = valueSuggest();
    if (!sug || sug.field !== field) return false;
    const ctx = detectRefContext(text, caret);
    if (!ctx || ctx.kind !== "page") return false;
    const hit = sug.hits[sug.index];
    if (!hit) return false;
    const { value, caret: nextCaret } = applySuggestion(text, ctx, hit.title);
    if (field === "edit") setDraft(value);
    else setNewValue(value);
    setValueSuggest(null);
    // Put the caret after the inserted `]]` rather than at the end:
    // a value can hold more than one ref (`related:: [[a]] [[b]]`).
    if (input) {
      queueMicrotask(() => {
        input.setSelectionRange(nextCaret, nextCaret);
        input.focus();
      });
    }
    return true;
  }

  /** The add-editor's value input, so accepting a key suggestion can
   *  hand the caret straight to the value — per instance, never a
   *  module global (two open editors would steal each other's focus). */
  let valueRef: HTMLInputElement | undefined;
  /** The inline chip editor's input, for the same reason. */
  let editValueRef: HTMLInputElement | undefined;

  const chips = createMemo(() => propertyChips(props.properties));
  const noun = () => props.noun ?? "property";

  /** Keys already on this block / page — suggesting one would turn an
   *  "add" into a silent overwrite of a chip sitting right there. */
  const taken = createMemo(
    () => new Set(chips().map((c) => c.key.toLowerCase())),
  );

  const suggestions = createMemo(() => {
    const q = newKey().trim().toLowerCase();
    return catalogue()
      .filter((k) => !taken().has(k.key.toLowerCase()))
      .filter((k) => q === "" || k.key.toLowerCase().includes(q))
      .slice(0, MAX_SUGGESTIONS);
  });

  function openAdd() {
    setNewKey("");
    setNewValue("");
    setSuggestIndex(0);
    setSuggestOpen(true);
    setAdding(true);
    // Asked on open rather than cached: the answer is a map scan, and
    // a cached list is wrong the first time the user adds a key.
    void knownPropertyKeys()
      .then(setCatalogue)
      // A missing catalogue costs autocomplete, not the ability to
      // type a key. Never surface it as an error.
      .catch(() => setCatalogue([]));
  }

  function closeAdd() {
    setAdding(false);
    setSuggestOpen(false);
    props.onAddOpenChange?.(false);
  }

  // Host-driven open (the chord). `on(..., { defer: true })` would
  // still fire for a signal that starts true, so guard on the value.
  createEffect(
    on(
      () => props.addOpen,
      (open) => {
        if (open && !adding()) openAdd();
      },
    ),
  );

  /** Run a write, reporting failure. Shared by edit, delete and add so
   *  none of the three can quietly become the one that swallows. */
  function write(key: string, value: string) {
    // The callback runs *inside* the chain: as an argument it would be
    // evaluated before the promise exists, so a synchronous throw
    // escapes past `.catch` and out of the blur handler.
    void Promise.resolve()
      .then(() => props.onCommit(key, value))
      .catch((e) => props.onError?.(e instanceof Error ? e.message : String(e)));
  }

  function commitEdit(key: string) {
    // Read before clearing: the input is gone by the time an async
    // `onCommit` resolves.
    const value = draft();
    setEditingKey(null);
    write(key, value);
  }

  function commitAdd() {
    const key = newKey().trim();
    closeAdd();
    // An empty key is a cancel, not an error: the user opened the
    // editor and changed their mind, and the backend would reject it
    // with a message that reads like a bug.
    if (!key) return;
    write(key, newValue());
  }

  return (
    <div
      class={`mt-0.5 flex-wrap items-center gap-1 ${
        chips().length > 0 || adding() || props.addAffordance === "always"
          ? "flex"
          : // Nothing to show at rest. `hidden` (not a faded row) because
            // an always-rendered empty row costs ~18px on EVERY block:
            // a 200-block page would scroll an extra screenful for a
            // control nobody is reaching for. The `+` still appears on
            // block hover, and the keyboard path (`Cmd+Shift+P`) never
            // depended on seeing it.
            "hidden group-hover:flex"
      }`}
      onClick={(e) => {
        // The row underneath selects / edits the block. Anything in
        // here means "act on a property", never both.
        e.stopPropagation();
      }}
    >
      <For each={chips()}>
        {(chip) => (
          <Show
            when={editingKey() === chip.key}
            fallback={
              <span class="group/chip inline-flex items-center">
                <button
                  type="button"
                  class={props.chipClass}
                  title={`${chip.key}:: ${chip.value} — click to edit`}
                  onClick={() => {
                    setDraft(chip.value);
                    setEditingKey(chip.key);
                  }}
                >
                  {chip.icon
                    ? `${chip.icon} ${chip.value}`
                    : `${chip.key}: ${chip.value}`}
                </button>
                {/* Explicit delete. Emptying the value does the same
                    thing and always has — but a gesture nobody can see
                    is not an affordance. */}
                <button
                  type="button"
                  class="ml-0.5 rounded px-1 text-xs leading-none opacity-50 hover:opacity-100 hover:bg-(--color-outl-fg)/10 focus:opacity-100"
                  aria-label={`Delete ${noun()} ${chip.key}`}
                  title={`Delete ${chip.key}::`}
                  onClick={() => write(chip.key, "")}
                >
                  ×
                </button>
              </span>
            }
          >
            <input
              class={props.inputClass ?? props.chipClass}
              value={draft()}
              autofocus
              aria-label={`${chip.key} value`}
              placeholder={`${chip.key}…`}
              ref={(el) => (editValueRef = el)}
              onInput={(e) => {
                setDraft(e.currentTarget.value);
                void refreshValueSuggest(
                  "edit",
                  e.currentTarget.value,
                  e.currentTarget.selectionStart ?? e.currentTarget.value.length,
                );
              }}
              onBlur={() => {
                // Let a click on a suggestion land before the commit
                // tears the popup down.
                setTimeout(() => {
                  setValueSuggest(null);
                  commitEdit(chip.key);
                }, 120);
              }}
              onKeyDown={(e) => {
                const sug = valueSuggest();
                const open = sug?.field === "edit";
                if (open && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
                  e.preventDefault();
                  const delta = e.key === "ArrowDown" ? 1 : -1;
                  setValueSuggest({
                    ...sug!,
                    index:
                      (sug!.index + delta + sug!.hits.length) % sug!.hits.length,
                  });
                  e.stopPropagation();
                  return;
                }
                if (open && (e.key === "Enter" || e.key === "Tab")) {
                  const el = e.currentTarget;
                  if (
                    acceptValueSuggest(
                      "edit",
                      el.value,
                      el.selectionStart ?? el.value.length,
                      editValueRef,
                    )
                  ) {
                    e.preventDefault();
                    e.stopPropagation();
                    return;
                  }
                }
                if (e.key === "Enter") {
                  e.preventDefault();
                  // Blur would fire the commit a second time.
                  e.currentTarget.blur();
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  // A popup swallows the first Escape, so the value
                  // stays put and a second press closes the editor.
                  if (open) {
                    setValueSuggest(null);
                  } else {
                    // Drop the draft, keep the stored value.
                    setEditingKey(null);
                  }
                }
                // Everything else reaches the input: the outline's vim
                // bindings must not eat characters being typed.
                e.stopPropagation();
              }}
            />
          </Show>
        )}
      </For>

      <Show
        when={adding()}
        fallback={
          <button
            type="button"
            class={`rounded px-1.5 py-0.5 text-xs leading-none opacity-40 hover:bg-(--color-outl-fg)/10 hover:opacity-100 focus:opacity-100 ${
              props.addAffordance === "always" ? "" : "opacity-40 group-hover:opacity-70"
            }`}
            aria-label={`Add ${noun()}`}
            title={`Add a ${noun()} (Cmd/Ctrl+Shift+P)`}
            onClick={openAdd}
          >
            + prop
          </button>
        }
      >
        <span class="relative inline-flex items-center gap-1">
          <input
            class={props.inputClass ?? props.chipClass}
            value={newKey()}
            autofocus
            aria-label={`New ${noun()} key`}
            placeholder="key"
            onInput={(e) => {
              setNewKey(e.currentTarget.value);
              setSuggestIndex(0);
              setSuggestOpen(true);
            }}
            onKeyDown={(e) => {
              // Arrows / Enter / Tab / Escape belong to the popup while
              // it has items; the same contract the `[[` and `/`
              // popups already use, so completion behaves identically.
              if (
                suggestOpen() &&
                handlePopupNav(e, {
                  items: suggestions(),
                  index: suggestIndex(),
                  setIndex: setSuggestIndex,
                  onAccept: (item) => {
                    setNewKey(item.key);
                    setSuggestOpen(false);
                    valueRef?.focus();
                  },
                  onClose: () => setSuggestOpen(false),
                })
              ) {
                return;
              }
              if (e.key === "Enter") {
                e.preventDefault();
                valueRef?.focus();
              } else if (e.key === "Escape") {
                e.preventDefault();
                closeAdd();
              }
              e.stopPropagation();
            }}
          />
          <span class="text-xs opacity-40">::</span>
          <input
            ref={(el) => (valueRef = el)}
            class={props.inputClass ?? props.chipClass}
            value={newValue()}
            aria-label={`New ${noun()} value`}
            placeholder="value"
            onInput={(e) => {
              setNewValue(e.currentTarget.value);
              void refreshValueSuggest(
                "add",
                e.currentTarget.value,
                e.currentTarget.selectionStart ?? e.currentTarget.value.length,
              );
            }}
            onBlur={() => setTimeout(() => setValueSuggest(null), 120)}
            onKeyDown={(e) => {
              const sug = valueSuggest();
              const open = sug?.field === "add";
              if (open && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
                e.preventDefault();
                const delta = e.key === "ArrowDown" ? 1 : -1;
                setValueSuggest({
                  ...sug!,
                  index:
                    (sug!.index + delta + sug!.hits.length) % sug!.hits.length,
                });
                e.stopPropagation();
                return;
              }
              if (open && (e.key === "Enter" || e.key === "Tab")) {
                const el = e.currentTarget;
                if (
                  acceptValueSuggest(
                    "add",
                    el.value,
                    el.selectionStart ?? el.value.length,
                    valueRef,
                  )
                ) {
                  e.preventDefault();
                  e.stopPropagation();
                  return;
                }
              }
              if (e.key === "Enter") {
                e.preventDefault();
                commitAdd();
              } else if (e.key === "Escape") {
                e.preventDefault();
                // The popup eats the first Escape so a mistaken `[[`
                // doesn't throw away the whole pair.
                if (open) setValueSuggest(null);
                else closeAdd();
              }
              e.stopPropagation();
            }}
          />
          <button
            type="button"
            class="rounded px-1.5 py-0.5 text-xs opacity-60 hover:bg-(--color-outl-fg)/10 hover:opacity-100"
            aria-label={`Save ${noun()}`}
            title="Save"
            onClick={commitAdd}
          >
            ✓
          </button>
          <Show when={suggestOpen() && suggestions().length > 0}>
            <ul
              role="listbox"
              aria-label={`${noun()} key suggestions`}
              class="absolute top-full left-0 z-30 mt-1 max-h-56 min-w-40 overflow-y-auto rounded border border-(--color-outl-border) bg-(--color-outl-bg-elev) py-1 shadow-lg"
            >
              <For each={suggestions()}>
                {(item, i) => (
                  <li>
                    <button
                      type="button"
                      role="option"
                      aria-selected={i() === suggestIndex()}
                      class={`flex w-full items-baseline justify-between gap-3 px-2 py-0.5 text-left text-xs ${
                        i() === suggestIndex()
                          ? "bg-(--color-outl-accent)/20"
                          : "hover:bg-(--color-outl-fg)/8"
                      }`}
                      // `mousedown`, not `click`: the key input blurs
                      // first on a click and the popup would unmount
                      // before the handler ran.
                      onMouseDown={(e) => {
                        e.preventDefault();
                        setNewKey(item.key);
                        setSuggestOpen(false);
                        valueRef?.focus();
                      }}
                    >
                      <span class="font-mono">{item.key}</span>
                      <span class="opacity-40">{item.uses}</span>
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </span>
      </Show>
        {/* `[[page]]` completion over whichever value field is focused.
            Anchored at the editor root, not inside the add row: the
            inline chip editor lives in the `<For>` above, so a popup
            nested in the add row never rendered for it while its
            keydown handler still swallowed Arrow/Enter/Escape. */}
            <Show when={valueSuggest()}>
              {(sug) => (
                <ul
                  role="listbox"
                  aria-label={`${noun()} value page suggestions`}
                  class="absolute top-full left-0 z-30 mt-1 max-h-56 min-w-48 overflow-y-auto rounded border border-(--color-outl-border) bg-(--color-outl-bg-elev) py-1 shadow-lg"
                >
                  <For each={sug().hits}>
                    {(hit, i) => (
                      <li>
                        <button
                          type="button"
                          role="option"
                          aria-selected={i() === sug().index}
                          class={`flex w-full items-baseline gap-2 px-2 py-0.5 text-left text-xs ${
                            i() === sug().index
                              ? "bg-(--color-outl-accent)/20"
                              : "hover:bg-(--color-outl-fg)/8"
                          }`}
                          onMouseDown={(e) => {
                            e.preventDefault();
                            const field = sug().field;
                            const input = field === "add" ? valueRef : editValueRef;
                            const text = field === "add" ? newValue() : draft();
                            setValueSuggest({ ...sug(), index: i() });
                            acceptValueSuggest(
                              field,
                              text,
                              input?.selectionStart ?? text.length,
                              input,
                            );
                          }}
                        >
                          <Show when={hit.icon}>
                            <span>{hit.icon}</span>
                          </Show>
                          <span>{hit.title}</span>
                        </button>
                      </li>
                    )}
                  </For>
                </ul>
              )}
            </Show>
    </div>
  );
}
