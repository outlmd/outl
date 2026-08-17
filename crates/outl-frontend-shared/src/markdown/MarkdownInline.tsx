import { createResource, For, JSX, Match, Show, Switch } from "solid-js";

import { readAssetDataUrl } from "../api/commands";
import type { InlineToken, ResolvedBlock } from "../api/types";
import {
  assetFileName,
  isAssetLink,
  isImagePath,
  isSafeHttpUrl,
} from "../links";

/** Resolved block content keyed by ref handle (`blk-XXXXXX`). Both
 *  `((…))` inline refs and `!((…))` embeds resolve through the same
 *  map — see {@link ResolvedBlock}. */
export type EmbedMap = Record<string, ResolvedBlock>;

/** Task glyph prefix for a resolved block's `status`
 *  (`"todo"`/`"doing"`/`"done"`), shared by the `blockref` and `embed`
 *  renders. Same glyphs the TUI draws. */
function statusMark(status: ResolvedBlock["status"]): string {
  if (status === "done") return "✓ ";
  if (status === "doing") return "◐ ";
  if (status === "todo") return "☐ ";
  return "";
}

/**
 * Button affordances (role / tabindex / click + Enter/Space handlers) for
 * a `<span>` that behaves as a link when `onLinkClick` is wired. Returns
 * `{}` when it isn't — an inert link must stay a plain span, not a fake
 * button that makes a screen reader announce a dead control and traps a
 * keyboard tab stop. Shared by the `link` case and the asset `FileChip`.
 */
function clickableAttrs(
  href: string,
  onLinkClick?: (href: string) => void,
): JSX.HTMLAttributes<HTMLSpanElement> {
  if (!onLinkClick) return {};
  const fire = () => onLinkClick(href);
  return {
    role: "button",
    tabindex: 0,
    onClick: (e: MouseEvent) => {
      e.stopPropagation();
      fire();
    },
    onKeyDown: (e: KeyboardEvent) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        e.stopPropagation();
        fire();
      }
    },
  };
}

/**
 * A clickable chip standing in for an asset that isn't rendered inline
 * (a PDF, an unknown file type, or an image whose bytes failed to load).
 *
 * Clicking / Enter / Space fires `onLinkClick(href)` — the client routes
 * an `assets/…` href to `openAsset` (OS default viewer), the same path
 * the `link` case uses. When no handler is wired the chip is inert text,
 * not a fake button (screen-reader / keyboard parity with the link case).
 */
function FileChip(props: {
  href: string;
  label: string;
  icon?: string;
  onLinkClick?: (href: string) => void;
}): JSX.Element {
  const a11y = clickableAttrs(props.href, props.onLinkClick);
  return (
    <span
      {...a11y}
      title={props.href}
      class="inline-flex items-center gap-1 rounded-md bg-(--color-ios-divider)/30 px-1.5 py-0.5 text-[14px] text-(--color-ios-text) dark:bg-(--color-iosd-divider)/30 dark:text-(--color-iosd-text)"
      classList={{ "cursor-pointer": !!props.onLinkClick }}
    >
      <span aria-hidden="true">{props.icon ?? "📄"}</span>
      {props.label}
    </span>
  );
}

/**
 * Render a block's pre-tokenized inline markdown.
 *
 * The Rust backend (`outl_md::tokenize_owned`) tokenizes every block
 * before it leaves the workspace, so the renderer receives the
 * tokens directly. No parallel TS tokenizer lives here — adding a
 * new variant in `outl_md::InlineTok` plus its `InlineToken`
 * counterpart and a render case below is the whole flow.
 *
 * Variants currently supported:
 *
 * - `**bold**`, `*italic*` / `_italic_`, `~~strike~~`, `` `code` ``
 * - `[label](url)` links — clickable when the host passes
 *   `onLinkClick` (desktop wires it to the Tauri opener so the URL
 *   opens in the system browser); inert text otherwise.
 * - `[[page name]]` wiki refs and `#tag` (visual style controlled
 *   by `variant`)
 * - `((blk-XXXXXX))` block refs and `!((blk-XXXXXX))` embeds
 * - `![alt](href)` images / assets — an `<img>` for image extensions
 *   (local `assets/…` resolved to a `data:` URL via `readAssetDataUrl`,
 *   remote `http(s)` loaded directly), a clickable file chip for pdf /
 *   other kinds (opens via `onLinkClick` → the client's `openAsset`)
 *
 * ## `variant`
 *
 * - **`"pill"`** (default) — touch-friendly chips with filled
 *   backgrounds. Right for mobile where fingers need a fat target.
 *   References `--color-ios-accent` so mobile's existing palette
 *   keeps working.
 * - **`"inline"`** — TUI-style inline text with underline + color
 *   only, no chip background. Right for desktop where dense
 *   reading is the norm; pills there feel heavy. Uses the
 *   canonical `--color-outl-ref-link-fg` (and `--color-outl-tag-link-fg`)
 *   tokens for separate hues on refs vs tags.
 *
 * The host client supplies the matching CSS custom properties; the
 * renderer is otherwise unaware of the active theme.
 */
type Variant = "pill" | "inline";

interface MarkdownInlineProps {
  /** Tokens produced by `outl_md::tokenize_owned` on the backend. */
  tokens: InlineToken[];
  /** Render style. Defaults to `"pill"` (mobile-style chips).
   *  Desktop passes `"inline"` for TUI-style underlined text. */
  variant?: Variant;
  /** Invoked when the user taps a `[[ref]]`. The argument is the
   * target name (slug or page title) inside the brackets. Skip the
   * default tap handling (e.g. when this lives inside a textarea). */
  onRefClick?: (target: string) => void;
  /** Invoked when the user taps a `#tag`. */
  onTagClick?: (tag: string) => void;
  /** Invoked when the user taps an external `[label](url)` link. The
   * argument is the raw `href`. When omitted the link renders as inert
   * text (mobile / backlink contexts that don't open URLs yet). */
  onLinkClick?: (href: string) => void;
  /** Resolved embed content keyed by handle. When present, `embed`
   *  tokens render the source block's text instead of a chip. */
  embeds?: EmbedMap;
}

export function MarkdownInline(props: MarkdownInlineProps): JSX.Element {
  const variant = (): Variant => props.variant ?? "pill";

  return (
    <For each={props.tokens}>
      {(tok) => {
        switch (tok.kind) {
          case "plain":
            return <span>{tok.value}</span>;
          // Bold / italic / strike recurse: their `inner` is a
          // fully-tokenized list, so nested refs/tags/block-refs
          // re-enter the renderer and pick up their own styling
          // under the wrapping element. Without this recursion the
          // pattern `**[[avelino]]**` rendered as literal text.
          case "bold":
            return (
              <span class="font-semibold">
                <MarkdownInline
                  tokens={tok.inner}
                  variant={props.variant}
                  onRefClick={props.onRefClick}
                  onTagClick={props.onTagClick}
                  onLinkClick={props.onLinkClick}
                />
              </span>
            );
          case "italic":
            return (
              <span class="italic">
                <MarkdownInline
                  tokens={tok.inner}
                  variant={props.variant}
                  onRefClick={props.onRefClick}
                  onTagClick={props.onTagClick}
                  onLinkClick={props.onLinkClick}
                />
              </span>
            );
          case "strike":
            return (
              <span class="line-through opacity-70">
                <MarkdownInline
                  tokens={tok.inner}
                  variant={props.variant}
                  onRefClick={props.onRefClick}
                  onTagClick={props.onTagClick}
                  onLinkClick={props.onLinkClick}
                />
              </span>
            );
          case "highlight":
            return (
              <mark class="rounded-[0.2em] bg-yellow-200/70 px-[0.15em] text-inherit dark:bg-yellow-400/30">
                <MarkdownInline
                  tokens={tok.inner}
                  variant={props.variant}
                  onRefClick={props.onRefClick}
                  onTagClick={props.onTagClick}
                  onLinkClick={props.onLinkClick}
                />
              </mark>
            );
          case "code":
            return (
              <code class="rounded bg-(--color-ios-divider)/30 px-1 py-0.5 font-mono text-[14px] dark:bg-(--color-iosd-divider)/30">
                {tok.value}
              </code>
            );
          case "link": {
            const a11y = clickableAttrs(tok.href, props.onLinkClick);
            return (
              <Show
                when={variant() === "inline"}
                fallback={
                  <span
                    {...a11y}
                    title={tok.href}
                    class="text-(--color-ios-accent) underline active:opacity-60 dark:text-(--color-iosd-accent)"
                    classList={{ "cursor-pointer": !!props.onLinkClick }}
                  >
                    {tok.value}
                  </span>
                }
              >
                <span
                  {...a11y}
                  title={tok.href}
                  class="text-(--color-outl-md-link-fg) underline decoration-(--color-outl-md-link-fg)/40 underline-offset-2 hover:decoration-(--color-outl-md-link-fg)"
                  classList={{ "cursor-pointer": !!props.onLinkClick }}
                >
                  {tok.value}
                </span>
              </Show>
            );
          }
          case "image": {
            const label = assetFileName(tok.href);
            // The compact inline variant (breadcrumbs, previews) can't
            // hold a block image without breaking the line, so any asset
            // renders as an inline-sized chip there. The block/pill
            // variant renders the real <img> below.
            if (variant() === "inline") {
              return (
                <FileChip
                  href={tok.href}
                  label={tok.alt || label}
                  icon={isImagePath(tok.href) ? "🖼️" : "📄"}
                  onLinkClick={props.onLinkClick}
                />
              );
            }
            // Non-image file (pdf, unknown) → clickable chip that opens
            // in the OS viewer (via the client's onLinkClick asset
            // routing). No inline embed for the MVP.
            // TODO(#203): inline PDF viewer
            if (!isImagePath(tok.href)) {
              return (
                <FileChip
                  href={tok.href}
                  label={tok.alt || label}
                  onLinkClick={props.onLinkClick}
                />
              );
            }
            const imgClass =
              "my-1 block h-auto max-h-96 max-w-full rounded-lg";
            // A remote image loads straight from its URL, but only over
            // http(s): the href comes from synced / imported markdown, so
            // an arbitrary scheme (file:, data:, javascript:) is refused
            // and shown as a chip instead of being hit as an <img src>. A
            // local `assets/…` asset has no web origin the webview can
            // reach, so its bytes come back from the backend as a `data:`
            // URL.
            if (!isAssetLink(tok.href)) {
              if (!isSafeHttpUrl(tok.href)) {
                return (
                  <FileChip
                    href={tok.href}
                    label={tok.alt || label}
                    icon="🖼️"
                    onLinkClick={props.onLinkClick}
                  />
                );
              }
              return (
                <img
                  src={tok.href}
                  alt={tok.alt}
                  loading="lazy"
                  class={imgClass}
                />
              );
            }
            const [dataUrl] = createResource(
              () => tok.href,
              readAssetDataUrl,
            );
            return (
              <Switch
                fallback={
                  <FileChip
                    href={tok.href}
                    label={tok.alt || label}
                    icon="🖼️"
                    onLinkClick={props.onLinkClick}
                  />
                }
              >
                <Match when={dataUrl.loading}>
                  <span
                    class="inline-flex items-center gap-1 rounded-md bg-(--color-ios-divider)/30 px-1.5 py-0.5 text-[14px] text-(--color-ios-text-secondary) dark:bg-(--color-iosd-divider)/30 dark:text-(--color-iosd-text-secondary)"
                    aria-label={tok.alt || label}
                  >
                    <span aria-hidden="true">🖼️</span>
                    {tok.alt || label}…
                  </span>
                </Match>
                <Match when={!dataUrl.error && dataUrl()}>
                  {(src) => (
                    <img
                      src={src()}
                      alt={tok.alt}
                      loading="lazy"
                      class={imgClass}
                    />
                  )}
                </Match>
              </Switch>
            );
          }
          case "ref":
            return (
              <Show
                when={variant() === "inline"}
                fallback={
                  <span
                    role="button"
                    onClick={(e) => {
                      if (!props.onRefClick) return;
                      e.stopPropagation();
                      props.onRefClick(tok.value);
                    }}
                    class="rounded-md bg-(--color-ios-accent)/12 px-1.5 py-0.5 text-[15px] font-medium text-(--color-ios-accent) active:opacity-60 dark:bg-(--color-iosd-accent)/20 dark:text-(--color-iosd-accent)"
                  >
                    {tok.value}
                  </span>
                }
              >
                <span
                  role="button"
                  onClick={(e) => {
                    if (!props.onRefClick) return;
                    e.stopPropagation();
                    props.onRefClick(tok.value);
                  }}
                  class="cursor-pointer text-(--color-outl-ref-link-fg) underline decoration-(--color-outl-ref-link-fg)/40 underline-offset-2 hover:decoration-(--color-outl-ref-link-fg)"
                >
                  {tok.value}
                </span>
              </Show>
            );
          case "tag":
            return (
              <Show
                when={variant() === "inline"}
                fallback={
                  <span
                    role="button"
                    onClick={(e) => {
                      if (!props.onTagClick) return;
                      e.stopPropagation();
                      props.onTagClick(tok.value);
                    }}
                    class="text-(--color-ios-accent) active:opacity-60 dark:text-(--color-iosd-accent)"
                  >
                    {tok.value}
                  </span>
                }
              >
                <span
                  role="button"
                  onClick={(e) => {
                    if (!props.onTagClick) return;
                    e.stopPropagation();
                    props.onTagClick(tok.value);
                  }}
                  class="cursor-pointer text-(--color-outl-tag-link-fg) underline decoration-(--color-outl-tag-link-fg)/40 underline-offset-2 hover:decoration-(--color-outl-tag-link-fg)"
                >
                  {/* The Rust token serialiser stores the leading `#` in
                      `value` already (see `outl_md::inline::InlineToken::from_borrowed`
                      and the `round_trips_every_variant_into_serializable_form`
                      contract test that asserts `tag == "#tag"`). Adding
                      another `#` here paints it twice (`##avelino`) —
                      bug seen on desktop pretty render. */}
                  {tok.value}
                </span>
              </Show>
            );
          case "blockref": {
            // `((blk-XXXXXX))` inline ref. When the handle resolves
            // (host passed `embeds`), render the source block's text
            // Roam-style — an inline reference, not the raw handle.
            // Orphan handles (unresolved) keep the neutral chip so a
            // dangling ref is still visible. Display-only: the issue
            // (#147) is the render layer; no onClick is added here.
            const resolved = props.embeds?.[tok.value];
            if (resolved) {
              const mark = statusMark(resolved.status);
              return (
                <Show
                  when={variant() === "inline"}
                  fallback={
                    <span class="rounded-md bg-(--color-ios-accent)/12 px-1.5 py-0.5 text-[15px] font-medium text-(--color-ios-accent) dark:bg-(--color-iosd-accent)/20 dark:text-(--color-iosd-accent)">
                      {mark}
                      {resolved.text}
                    </span>
                  }
                >
                  <span class="text-(--color-outl-ref-link-fg) underline decoration-(--color-outl-ref-link-fg)/40 underline-offset-2">
                    {mark}
                    {resolved.text}
                  </span>
                </Show>
              );
            }
            return (
              <span class="rounded bg-(--color-ios-divider)/30 px-1 font-mono text-[13px] text-(--color-ios-text-secondary) dark:bg-(--color-iosd-divider)/30 dark:text-(--color-iosd-text-secondary)">
                {tok.value}
              </span>
            );
          }
          case "embed": {
            const resolved = props.embeds?.[tok.value];
            if (resolved) {
              const mark = statusMark(resolved.status);
              return (
                <span class="rounded bg-(--color-ios-accent)/8 px-1 py-0.5 text-[13px] text-(--color-ios-text) dark:bg-(--color-iosd-accent)/10 dark:text-(--color-iosd-text)">
                  ↳ {mark}{resolved.text}
                </span>
              );
            }
            return (
              <span class="rounded bg-(--color-ios-accent)/12 px-1 font-mono text-[13px] text-(--color-ios-accent) dark:bg-(--color-iosd-accent)/20 dark:text-(--color-iosd-accent)">
                !{tok.value}
              </span>
            );
          }
          case "emoji":
            // The backend's catalog gate guarantees `glyph` is set on
            // every Emoji token. Defensive fall-back: if a peer ever
            // ships an unknown shortcode (binary too old to know it),
            // render the literal `:shortcode:` form so the user sees
            // *something* instead of a blank span.
            return (
              <span
                title={`:${tok.shortcode}:`}
                aria-label={tok.shortcode}
                class="mx-0.5"
              >
                {tok.glyph || `:${tok.shortcode}:`}
              </span>
            );
        }
      }}
    </For>
  );
}
