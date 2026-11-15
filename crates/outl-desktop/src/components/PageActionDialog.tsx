import { For, Show, createEffect, createSignal, onMount } from "solid-js";
import { Portal } from "solid-js/web";

import { searchPages } from "@outl/shared/api/commands";
import type { PageMeta, PageView } from "@outl/shared/api/types";
import { PageGlyph } from "@outl/shared/icons";

import { mergePageFlow, renamePageFlow } from "../lib/page-actions";
import { setAppState } from "../lib/store";

export type PageAction = { kind: "rename" | "merge"; page: PageMeta };

/**
 * One dialog for the two page actions that need input: a name prompt
 * (rename, prefilled with the current title) and a page search (merge,
 * the source page excluded). The webview has no `window.prompt`, so the
 * text field is ours; the questions that follow (name taken, merge
 * confirmation) are `window.confirm`, in `lib/page-actions.ts`.
 *
 * Declining a confirm leaves the dialog open so the user can pick again;
 * it closes only once the flow produced a page to show. Escape and a
 * click on the backdrop close it without writing anything.
 */
export function PageActionDialog(props: {
  action: PageAction | null;
  onClose: () => void;
  onDone: (view: PageView) => void;
}) {
  return (
    <Show when={props.action}>
      {(action) => (
        <Portal>
          <DialogBody action={action()} onClose={props.onClose} onDone={props.onDone} />
        </Portal>
      )}
    </Show>
  );
}

function DialogBody(props: {
  action: PageAction;
  onClose: () => void;
  onDone: (view: PageView) => void;
}) {
  const isRename = props.action.kind === "rename";
  const [text, setText] = createSignal(isRename ? props.action.page.title : "");
  const [results, setResults] = createSignal<PageMeta[]>([]);
  const [highlight, setHighlight] = createSignal(0);
  const [busy, setBusy] = createSignal(false);
  let input: HTMLInputElement | undefined;

  onMount(() => {
    input?.focus();
    input?.select();
  });

  if (!isRename) {
    createEffect(() => {
      const q = text();
      searchPages(q)
        .then((pages) => {
          if (q !== text()) return;
          setResults(pages.filter((p) => p.slug !== props.action.page.slug));
          setHighlight(0);
        })
        .catch((e) =>
          setAppState("lastError", e instanceof Error ? e.message : String(e)),
        );
    });
  }

  async function run(flow: () => Promise<PageView | null>) {
    if (busy()) return;
    setBusy(true);
    try {
      const view = await flow();
      if (view) props.onDone(view);
    } finally {
      setBusy(false);
    }
  }

  function submit() {
    if (isRename) {
      const name = text().trim();
      if (!name || name === props.action.page.title) return;
      void run(() => renamePageFlow(props.action.page, name));
      return;
    }
    const target = results()[highlight()];
    if (target) pickTarget(target);
  }

  function pickTarget(target: PageMeta) {
    void run(() => mergePageFlow(props.action.page, target));
  }

  function onKey(e: KeyboardEvent) {
    // The dialog owns the keyboard while it is open: the Normal-mode
    // dispatcher must not read these as chords.
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      props.onClose();
    } else if (e.key === "Enter" && !e.isComposing) {
      e.preventDefault();
      submit();
    } else if (!isRename && e.key === "ArrowDown") {
      e.preventDefault();
      setHighlight((h) => Math.min(h + 1, Math.max(results().length - 1, 0)));
    } else if (!isRename && e.key === "ArrowUp") {
      e.preventDefault();
      setHighlight((h) => Math.max(h - 1, 0));
    }
  }

  const sourceLabel = () => props.action.page.title || props.action.page.slug;

  return (
    <div
      class="fixed inset-0 z-50 flex items-start justify-center bg-black/50 backdrop-blur-sm"
      onClick={(e) => {
        if (e.target === e.currentTarget) props.onClose();
      }}
    >
      <div
        role="dialog"
        aria-label={isRename ? "Rename page" : "Merge page"}
        class="mt-24 w-[520px] max-w-[90vw] overflow-hidden rounded-pop border border-(--color-outl-border)/60 bg-(--color-outl-bg-elev) shadow-pop"
      >
        <div class="px-4 pt-3 text-[11px] font-medium text-(--color-outl-fg-dimmer)">
          {isRename ? `Rename "${sourceLabel()}"` : `Merge "${sourceLabel()}" into`}
        </div>
        <input
          ref={input}
          type="text"
          value={text()}
          disabled={busy()}
          onInput={(e) => setText(e.currentTarget.value)}
          onKeyDown={onKey}
          placeholder={isRename ? "New name" : "Search pages"}
          class="block w-full bg-transparent px-4 py-2 text-[15px] outline-none"
        />
        <Show when={!isRename}>
          <div class="max-h-[50vh] overflow-y-auto border-t border-(--color-outl-border)/60 p-1">
            <For each={results()}>
              {(page, idx) => (
                <button
                  type="button"
                  onClick={() => pickTarget(page)}
                  onMouseMove={() => setHighlight(idx())}
                  class={`flex w-full items-center gap-2 rounded-md px-3 py-1.5 text-left text-[13px] ${
                    idx() === highlight() ? "bg-(--color-outl-accent)/15" : ""
                  }`}
                >
                  <PageGlyph icon={page.icon} kind={page.kind} class="opacity-70" />
                  <span class="truncate">{page.title || page.slug}</span>
                  <span class="ml-auto shrink-0 font-mono text-[11px] text-(--color-outl-fg-dimmer)">
                    {page.slug}
                  </span>
                </button>
              )}
            </For>
          </div>
        </Show>
        <div class="border-t border-(--color-outl-fg)/10 px-4 py-1.5 text-[10px] opacity-50">
          {isRename ? "⏎ rename · Esc cancel" : "↑/↓ navigate · ⏎ merge · Esc cancel"}
        </div>
      </div>
    </div>
  );
}
