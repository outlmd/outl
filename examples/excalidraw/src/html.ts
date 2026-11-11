/**
 * The HTML a `rich` transformer hands back.
 *
 * The host runs this in `<iframe sandbox="allow-scripts">`, with no
 * `allow-same-origin`: the page is a null origin with no access to the app's
 * DOM, its storage or its cookies. So everything is inline — the styles, the
 * one script that reports the drawing's height, and the SVG itself.
 *
 * **`allow-scripts` does not block the network**, and reading it that way is
 * what let a scene's `dataURL` become a real request (see `isInlineImage`).
 * The desktop ships `csp: null`, so the only policy this page is guaranteed
 * is the one it declares itself, below.
 *
 * Two surfaces live here: a drawing, and a message. The message page matters
 * as much as the drawing one — "this scene is not in the cache yet" has to
 * arrive as a readable sentence with the command to fix it, because a plugin
 * that renders an empty box has told the user nothing.
 */

import { escape } from "./escape";

/** How the drawing is framed. */
export interface PageOptions {
  /** Invert the drawing for a dark surface, the way Excalidraw's dark mode does. */
  dark: boolean;
  /** Follow the client's colour scheme instead of deciding here. */
  auto: boolean;
  /** Tallest the drawing may get, in px. */
  maxHeight: number;
  /** Exact height in px, when the fence asked for one. */
  height?: number;
  /** Intrinsic aspect ratio of the drawing. */
  width: number;
  intrinsicHeight: number;
  /** Line under the drawing. Empty hides it. */
  caption: string;
}

/**
 * Shared page scaffolding: the stylesheet, the dark-mode rule and the height
 * report. Everything either surface needs and neither should spell twice.
 */
function shell(bodyClass: string, body: string, styles: string): string {
  return (
    `<!doctype html><html><head><meta charset="utf-8">` +
    // Belt as well as braces. The drawing only ever holds `data:` images
    // (see `isInlineImage`), and this says so to the engine too — a client
    // that ships no CSP of its own (the desktop sets `csp: null`) would
    // otherwise let any `href` in a scene become a real request, outside the
    // `network:` allowlist. A srcdoc iframe inherits the parent's policy, so
    // this only ever narrows.
    `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; ` +
    `img-src data:; style-src 'unsafe-inline'; script-src 'unsafe-inline'">` +
    `<style>` +
    `:root{color-scheme:light dark}` +
    `html,body{margin:0;background:transparent;` +
    `font:13px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif}` +
    styles +
    `</style></head><body class="${bodyClass}">${body}` +
    `<script>requestAnimationFrame(function(){` +
    `var h=Math.ceil(document.body.getBoundingClientRect().height);` +
    `parent.postMessage({outlHeight:h},'*');});</script></body></html>`
  );
}

/**
 * The dark-mode filter.
 *
 * This is how Excalidraw itself renders dark mode: it does not recolour the
 * scene, it inverts it and spins the hue back, so a drawing made on white
 * reads correctly on black without touching a single element's colour. Doing
 * it per-element would mean deciding what each of a user's colours *means*.
 */
const DARK_FILTER = "invert(93%) hue-rotate(180deg)";

/** Render a drawing. */
export function drawingPage(svg: string, opts: PageOptions): string {
  const ratio = opts.intrinsicHeight > 0 ? opts.width / opts.intrinsicHeight : 1;

  const sizing =
    typeof opts.height === "number"
      ? `height:${opts.height}px`
      : `aspect-ratio:${ratio.toFixed(4)};max-height:${opts.maxHeight}px`;

  const styles =
    `.scene{${sizing};width:100%;display:flex;align-items:center;justify-content:center}` +
    `.scene svg{max-width:100%;max-height:100%}` +
    `.caption{padding:6px 2px 2px;opacity:.6;font-size:12px}` +
    (opts.dark ? `.scene{filter:${DARK_FILTER}}` : "") +
    (opts.auto
      ? `@media (prefers-color-scheme:dark){.scene{filter:${DARK_FILTER}}}`
      : "");

  const caption =
    opts.caption === "" ? "" : `<div class="caption">${escape(opts.caption)}</div>`;

  return shell("", `<div class="scene">${svg}</div>${caption}`, styles);
}

/** Render a message: a parse error, a missing key, an unsynced scene. */
export function messagePage(title: string, detail: string, command?: string): string {
  const styles =
    `.card{padding:12px 14px;border-radius:8px;border:1px solid rgba(128,128,128,.28);` +
    `background:rgba(128,128,128,.06)}` +
    `.title{font-weight:600;margin-bottom:4px}` +
    `.detail{opacity:.75}` +
    `code{font:12px/1.5 ui-monospace,SFMono-Regular,Menlo,monospace;` +
    `background:rgba(128,128,128,.16);padding:1px 5px;border-radius:4px;` +
    `display:inline-block;margin-top:8px;word-break:break-all}`;

  const hint = command ? `<code>${escape(command)}</code>` : "";
  const body =
    `<div class="card"><div class="title">✏️ ${escape(title)}</div>` +
    `<div class="detail">${escape(detail)}</div>${hint}</div>`;

  return shell("", body, styles);
}

/**
 * The caption under a drawing: what it is, when it was pulled, and what was
 * left out.
 *
 * The "left out" half is the point. A truncated or image-less render that says
 * nothing is indistinguishable from a complete one, and the user would only
 * find out by comparing against Excalidraw.
 */
export function caption(parts: {
  name?: string;
  syncedAt?: string;
  truncated?: number;
  unsupported?: number;
  filesDropped?: boolean;
}): string {
  const bits: string[] = [];
  if (parts.name) bits.push(parts.name);
  if (parts.syncedAt) {
    const when = relativeTime(parts.syncedAt);
    if (when !== "") bits.push(`synced ${when}`);
  }
  if (parts.truncated && parts.truncated > 0) {
    bits.push(`${parts.truncated} more element${parts.truncated === 1 ? "" : "s"} not drawn`);
  }
  if (parts.unsupported && parts.unsupported > 0) {
    bits.push(`${parts.unsupported} unsupported`);
  }
  if (parts.filesDropped) bits.push("images not cached");
  return bits.join(" · ");
}

/**
 * "3 hours ago" from an ISO timestamp.
 *
 * Returns `""` for anything unparseable rather than printing `NaN` — a stale
 * cache entry from an older version of this plugin should cost a caption, not
 * the drawing.
 */
function relativeTime(iso: string): string {
  const then = Date.parse(iso);
  if (!Number.isFinite(then)) return "";

  const seconds = Math.max(0, Math.round((Date.now() - then) / 1000));
  if (seconds < 60) return "just now";

  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;

  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;

  const days = Math.round(hours / 24);
  return `${days}d ago`;
}
