import { Match, Show, Switch, createResource } from "solid-js";

import type { PluginTransformer } from "@outl/shared/api/types";
import { runTransform } from "@outl/shared/plugins/transformer-registry";

/**
 * `<PluginFence />` — renders a fenced code block whose language a plugin
 * content-transformer claims.
 *
 * When a code fence's language matches a registered transformer (looked up in
 * `BlockRow` via `transformerFor`), the body is handed here. We call
 * the transformer (cached per `(block id, body)` so it runs at most once per
 * distinct body — `runTransform` owns the cache) and render its descriptor:
 *
 * - `kind: "text"` → the `content` is text/markdown; rendered inline as
 *   preformatted text. (The frontend has no raw-markdown-string tokenizer —
 *   tokenization is backend-only in `outl_md` — so we render the text
 *   faithfully rather than inventing a parallel parser.)
 * - `kind: "rich"` → the `content` is HTML; run **inline** in a sandboxed
 *   `<iframe>`, persistent (not the ephemeral overlay `<PluginViewOverlay />`
 *   uses for `ctx.ui.render`).
 *
 * ## Security (load-bearing — never weaken)
 *
 * The `rich` iframe is **`sandbox="allow-scripts"` WITHOUT
 * `allow-same-origin`**, exactly like `<PluginViewOverlay />`. Plugin HTML is
 * untrusted author code; omitting `allow-same-origin` forces an opaque origin
 * so the frame cannot reach the app DOM, cookies, `localStorage`, or the Tauri
 * bridge. Adding `allow-same-origin` alongside `allow-scripts` defeats the
 * sandbox entirely (the frame could strip its own sandbox attribute). Content
 * goes in via `srcdoc` — no network, no plugin-controlled URL.
 *
 * The fallback (transformer declined → `null`, or the resource is still
 * loading) is the caller's plain highlighted code block, passed as
 * `fallback`.
 */

/** Default height for the inline `rich` iframe. */
const RICH_FRAME_HEIGHT = "240px";

export function PluginFence(props: {
  /** Block id — half of the per-block transform cache key. */
  blockId: string;
  /** The matched transformer (`plugin_id` / `lang` / `kind`). */
  transformer: PluginTransformer;
  /** The fence body (without the ``` markers) — the transformer input. */
  body: string;
  /** What to render while loading or when the transformer declines. */
  fallback: () => unknown;
}) {
  // Re-run when the block id, transformer, or body changes. `runTransform`
  // dedupes by `(blockId, body)`, so a re-render with the same inputs reuses
  // the in-flight / resolved promise instead of re-invoking the host.
  const [result] = createResource(
    () => ({
      blockId: props.blockId,
      transformer: props.transformer,
      body: props.body,
    }),
    (k) => runTransform(k.blockId, k.transformer, k.body),
  );

  return (
    <Show when={result()} fallback={props.fallback() as never} keyed>
      {(r) => (
        <Switch fallback={props.fallback() as never}>
          <Match when={r.kind === "text"}>
            <div class="overflow-x-auto whitespace-pre-wrap break-words rounded-md bg-(--color-outl-bg-elev)/60 px-3 py-2 font-mono text-[15px] leading-[1.4]">
              {r.content}
            </div>
          </Match>
          <Match when={r.kind === "rich"}>
            <iframe
              // OMITTING `allow-same-origin` is the whole point — see the
              // module doc. Do not add it alongside `allow-scripts`.
              sandbox="allow-scripts"
              srcdoc={r.content}
              title="plugin content"
              class="w-full rounded-md border-0 bg-transparent"
              style={{ height: RICH_FRAME_HEIGHT }}
            />
          </Match>
        </Switch>
      )}
    </Show>
  );
}
