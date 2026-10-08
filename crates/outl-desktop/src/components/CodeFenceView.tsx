import {
  Match,
  Show,
  Switch,
  createMemo,
  createResource,
  createSignal,
  onCleanup,
} from "solid-js";

import type { PluginTransformResult } from "@outl/shared/api/types";
import { listTemplates } from "@outl/shared/api/commands";
import { HighlightedCode } from "@outl/shared/highlight";
import {
  runTransform,
  transformerFor,
} from "@outl/shared/plugins/transformer-registry";

/**
 * Rendered view of a `\`\`\`lang\n…\n\`\`\`` block. Shows the source
 * monospaced, a tiny language chip, and a Run button. Clicking the
 * source body kicks the editor in (so the user can edit the fence
 * just like any other block).
 *
 * When a plugin declares a **content transformer** for `language`, the
 * source is replaced by the transformer's rendered output:
 * - `kind: "text"` — rendered as plain, whitespace-preserving text (no
 *   client-side markdown parse — a transformer wanting rich formatting
 *   emits `kind: "rich"` HTML).
 * - `kind: "rich"` — HTML run in a sandboxed `<iframe>` **inline** in the
 *   block (one per fence, persistent while the block exists). The iframe is
 *   `sandbox="allow-scripts"` **without** `allow-same-origin` — the same
 *   isolation as the `ui-render` overlay; the plugin JS runs in a null
 *   origin with no access to the app DOM/cookies. Clicking the chip's edit
 *   affordance still drops into the raw fence editor.
 *
 * The transform runs the plugin's JS, so it is cached by `(blockId, body)`
 * (`runTransform`, `@outl/shared/plugins/transformer-registry`) and only
 * re-runs when the body changes.
 */
export function CodeFenceView(props: {
  blockId: string;
  language: string;
  body: string;
  onEdit: () => void;
  onRun: () => Promise<void>;
  /** Navigate to a page by slug — wired for `call:<name>` fences so the
   *  language chip links to the template's page. */
  onOpenPage?: (slug: string) => void;
}) {
  const [busy, setBusy] = createSignal(false);

  // A `call:<name>` fence references a template page. Resolve its slug so
  // the language chip can double as a link to that page. `null` for any
  // non-`call:` fence (the resource never runs) or an unknown template
  // name (the chip stays a plain label — no dead link).
  const callName = createMemo(() => {
    const lang = props.language.toLowerCase();
    return lang.startsWith("call:") ? props.language.slice(5).trim() : null;
  });
  const [templateSlug] = createResource(callName, async (name) => {
    const templates = await listTemplates().catch(() => []);
    return templates.find((t) => t.name === name)?.slug ?? null;
  });

  async function run() {
    setBusy(true);
    try {
      await props.onRun();
    } finally {
      setBusy(false);
    }
  }

  // Reactive transformer lookup: a fence that mounts before the registry
  // loads picks the transformer up once `loadTransformers` resolves.
  const match = createMemo(() => transformerFor(props.language));

  // Run (or replay) the transformer when one matches, re-keyed by body so a
  // fence edit re-transforms. `undefined` source ⇒ resource stays unset and
  // the plain source view shows.
  const [transformed] = createResource(
    () => {
      const m = match();
      return m ? { m, body: props.body } : undefined;
    },
    (k) => runTransform(props.blockId, k.m, k.body),
  );

  // Whether to show transformed output: a transformer matched AND it
  // produced a non-null descriptor. A declined transform (null) or an
  // in-flight first run falls back to the source view.
  const result = (): PluginTransformResult | null =>
    transformed.state === "ready" ? (transformed() ?? null) : null;

  return (
    <div class="rounded-md border border-(--color-outl-fg)/10 bg-(--color-outl-bg-elev)/60">
      <div class="flex items-center justify-between border-b border-(--color-outl-fg)/10 px-2 py-1">
        <Show
          when={props.onOpenPage ? templateSlug() : null}
          fallback={
            <span class="font-mono text-[10px] uppercase opacity-60">
              {props.language}
            </span>
          }
        >
          {(slug) => (
            <button
              type="button"
              title={`Open template: ${callName()}`}
              onClick={(e) => {
                e.stopPropagation();
                props.onOpenPage?.(slug());
              }}
              class="cursor-pointer font-mono text-[10px] uppercase opacity-60 underline decoration-dotted underline-offset-2 hover:opacity-100"
            >
              {props.language}
            </button>
          )}
        </Show>
        <Show
          when={match()}
          fallback={
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                void run();
              }}
              disabled={busy()}
              class="rounded bg-(--color-outl-fg)/10 px-2 py-0.5 text-[11px] hover:bg-(--color-outl-fg)/20 disabled:opacity-50"
            >
              {busy() ? "Running…" : "▶ Run"}
            </button>
          }
        >
          {/* Transformed fences aren't "run" — clicking edits the source. */}
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              props.onEdit();
            }}
            class="rounded bg-(--color-outl-fg)/10 px-2 py-0.5 text-[11px] hover:bg-(--color-outl-fg)/20"
          >
            ✎ Edit
          </button>
        </Show>
      </div>
      <Switch
        fallback={
          <div onClick={props.onEdit} class="cursor-text">
            <HighlightedCode
              language={props.language}
              code={props.body || " "}
            />
          </div>
        }
      >
        <Match when={result()?.kind === "text"}>
          {/* `text` output is rendered as plain, whitespace-preserving
              text. We deliberately do NOT run a client-side markdown
              parser here (root CLAUDE.md forbids a parallel
              implementation of `outl_md`); a transformer that wants rich
              formatting emits `kind: "rich"` HTML instead. */}
          <div class="cursor-text whitespace-pre-wrap break-words px-2 py-1 leading-snug">
            {result()?.content ?? ""}
          </div>
        </Match>
        <Match when={result()?.kind === "rich"}>
          <RichFenceFrame html={result()?.content ?? ""} />
        </Match>
      </Switch>
    </div>
  );
}

/**
 * Inline sandboxed iframe for a `rich` content-transformer's HTML output.
 *
 * **Security — do not weaken.** `sandbox="allow-scripts"` with **no**
 * `allow-same-origin`: the plugin's JS runs in a null origin, isolated from
 * the app's DOM, cookies, `localStorage`, and credentialed fetch. HTML
 * enters via `srcdoc`, never `innerHTML` on the host document. This is the
 * same isolation as the `ui-render` overlay (`PluginEffectLayer`); the
 * difference is only placement (inline in the block, not a fullscreen
 * overlay) and lifetime (persistent while the block exists, not ephemeral).
 *
 * The iframe is sized to its content via a postMessage handshake the plugin
 * may opt into (`parent.postMessage({ outlHeight: n }, "*")`); absent that,
 * it falls back to a reasonable default height so the content is visible.
 */
function RichFenceFrame(props: { html: string }) {
  // Default height until the plugin reports its content height. Bounded so a
  // misbehaving plugin can't grow the iframe without limit.
  const DEFAULT_H = 240;
  const MAX_H = 2000;
  const [height, setHeight] = createSignal(DEFAULT_H);

  let frame: HTMLIFrameElement | undefined;

  function onMessage(e: MessageEvent) {
    // Only trust height reports from *this* iframe's null-origin document.
    if (frame && e.source === frame.contentWindow) {
      const h = (e.data as { outlHeight?: unknown } | null)?.outlHeight;
      if (typeof h === "number" && h > 0) {
        setHeight(Math.min(Math.ceil(h), MAX_H));
      }
    }
  }

  // Listen for the plugin's height report for this frame's lifetime;
  // removed on cleanup so a re-render (new html) doesn't leak handlers.
  window.addEventListener("message", onMessage);
  onCleanup(() => window.removeEventListener("message", onMessage));

  return (
    <iframe
      ref={frame}
      // SECURITY: allow-scripts WITHOUT allow-same-origin — the plugin JS
      // runs in a null origin, isolated from the app. Never add
      // allow-same-origin here (mirrors PluginEffectLayer / ui-render).
      sandbox="allow-scripts"
      srcdoc={props.html}
      title="content-transformer"
      style={{
        width: "100%",
        height: `${height()}px`,
        border: "0",
        display: "block",
      }}
    />
  );
}
