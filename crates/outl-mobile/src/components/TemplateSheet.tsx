import { For, JSX, Show, createEffect, createSignal } from "solid-js";

import type { PageView, TemplateDto } from "@outl/shared/api/types";
import {
  instantiateTemplateAt,
  listTemplates,
} from "@outl/shared/api/commands";

import { createSheetDrag } from "../lib/sheet-drag";
import { haptic } from "../lib/haptics";

interface TemplateSheetProps {
  /** Block the template is deep-copied under. When `null` the sheet
   *  stays closed — a template needs a host block. */
  blockId: string | null;
  onClose: () => void;
  /** Toast channel for backend errors (unknown template, stale block). */
  onMessage: (text: string) => void;
  /** Refreshed page view after a successful instantiation. */
  onView: (view: PageView) => void;
}

/**
 * Bottom-sheet picker of the workspace's structural templates, opened
 * from the block long-press menu ("Insert template"). Picking a row
 * deep-copies that template's outline under the long-pressed block via
 * `instantiate_template_at`, then applies the returned page view.
 *
 * Structural templates are a core feature (reachable from TUI/CLI/MCP);
 * this sheet is the mobile GUI surface so they don't need a plugin.
 * Chrome mirrors `BlockContextMenu` / `PluginSheet` (same drag-dismiss
 * hook, blurred card, safe-area padding).
 */
export function TemplateSheet(props: TemplateSheetProps): JSX.Element {
  const drag = createSheetDrag(() => props.onClose());
  const [templates, setTemplates] = createSignal<TemplateDto[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [busy, setBusy] = createSignal(false);

  // Refresh the list every time the sheet opens — templates are edited
  // like any other page, so a cached list would go stale.
  createEffect(() => {
    if (props.blockId === null) return;
    setLoading(true);
    void listTemplates()
      .then((list) => setTemplates(list))
      .catch((e) => {
        props.onMessage(e instanceof Error ? e.message : String(e));
        setTemplates([]);
      })
      .finally(() => setLoading(false));
  });

  async function pick(template: TemplateDto) {
    const blockId = props.blockId;
    if (blockId === null || busy()) return;
    setBusy(true);
    haptic("light");
    try {
      const view = await instantiateTemplateAt(template.name, blockId);
      props.onView(view);
      props.onClose();
    } catch (e) {
      props.onMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Show when={props.blockId !== null}>
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
        <div class="mx-3 mb-2 overflow-hidden rounded-2xl bg-(--color-outl-bg-elev)/95 shadow-[var(--shadow-capsule)] backdrop-blur-2xl">
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
              class="mx-auto block h-1 w-10 rounded-full bg-(--color-outl-border)"
            />
          </span>

          <div class="px-4 pb-1 pt-1">
            <span class="text-[13px] font-semibold uppercase tracking-wide text-(--color-outl-fg-dim)">
              Insert template
            </span>
          </div>

          <div class="max-h-[55vh] overflow-y-auto">
            <For each={templates()}>
              {(template) => (
                <button
                  type="button"
                  disabled={busy()}
                  onClick={() => void pick(template)}
                  class="flex w-full flex-col items-start gap-0.5 border-t border-(--color-outl-border)/30 px-4 py-3.5 text-left active:bg-(--color-outl-border)/30 disabled:opacity-50"
                >
                  <span class="text-[16px] font-medium text-(--color-outl-fg)">
                    {template.name}
                    <Show when={template.duplicate}>
                      <span class="ml-1.5 text-[12px] font-normal text-(--color-outl-fg-dim)">
                        (duplicate name)
                      </span>
                    </Show>
                  </span>
                  <span class="font-mono text-[11px] text-(--color-outl-fg-dim)/70">
                    {template.slug}
                  </span>
                </button>
              )}
            </For>

            <Show when={!loading() && templates().length === 0}>
              <div class="border-t border-(--color-outl-border)/30 px-4 py-6 text-center text-[14px] text-(--color-outl-fg-dim)">
                No templates yet. Create a page with a{" "}
                <span class="font-mono">template::</span> property.
              </div>
            </Show>
          </div>
        </div>

        <button
          type="button"
          onClick={props.onClose}
          class="mx-3 rounded-2xl bg-(--color-outl-bg-elev)/95 py-3.5 text-center text-[16px] font-semibold text-(--color-outl-accent) shadow-[var(--shadow-capsule)] backdrop-blur-2xl active:bg-(--color-outl-border)/30"
        >
          Cancel
        </button>
      </div>
    </Show>
  );
}
