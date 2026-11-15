/**
 * The launch itself: what a client reports about it, and what it waits
 * on.
 *
 * Three things live here because all three are answers to "why is the
 * first frame late", and each one had a per-client copy (or no
 * implementation at all) before:
 *
 * - **`bootMark`** — the frontend half of the backend's `boot phases`
 *   line. Nothing in Rust can see when the webview painted, so without
 *   this the measured launch stops at "the workspace opened", which on
 *   a cold start is about half of what the user waited for.
 * - **`waitWorkspaceReady`** — one call that resolves when the
 *   workspace is published, replacing the desktop's 500 ms `setInterval`
 *   and the phone's 50 × 200 ms retry.
 * - **`cacheBootStyle` / `BOOT_STYLE_KEY`** — the one-frame theme
 *   cache. The palette comes over IPC, which is two round-trips after
 *   the document parses, so a light-theme user saw the brand's dark
 *   background first. The cache is what the `<head>` script applies
 *   before the first pixel.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { PageView } from "../api/types";

export { PreviewOutline } from "./PreviewOutline";

/**
 * The milestones a client reports, in launch order.
 *
 * Declared mirror of `outl_tauri_shared::commands::boot::BOOT_MARKS`,
 * pinned by that crate's `tests/wire_mirrors.rs`. The names are a wire
 * contract — the log, the CI budget checker and the simulator job all
 * match on the string — so a client inventing `firstContent` is not a
 * type error anywhere, just a mark that never appears.
 */
export const BOOT_MARKS = [
  "html_parsed",
  "theme_applied",
  "first_content",
  "interactive",
] as const;

export type BootMark = (typeof BOOT_MARKS)[number];

/**
 * Report a launch milestone to the backend.
 *
 * **Fire and forget, and deliberately not awaited anywhere.** A
 * measurement that can delay the thing it measures is worse than no
 * measurement; a client that has to handle its failure stops reporting
 * it. The rejection is swallowed for the same reason — on a client
 * built before the command existed, every mark would otherwise be an
 * unhandled rejection in the console on every launch.
 *
 * `first_content` also releases the backend's deferred boot work (sync,
 * the watcher, the orphan sweep), so it is the one mark a client must
 * not skip. Reporting it late costs a late sync; not reporting it at
 * all costs the gate's whole deadline.
 */
export function bootMark(mark: BootMark): void {
  void invoke("boot_mark", { name: mark }).catch(() => undefined);
}

/**
 * Resolve once the workspace is published, or after `timeoutMs` without
 * it.
 *
 * `false` means **not yet** — ask again. It is not a failure, and a
 * caller that renders it as one turns a cold 700k-op replay into an
 * error screen on every launch of a large workspace.
 *
 * A rejection is a real failure (the backend could not take the wait),
 * and is left to the caller: the loading screen has somewhere to put it
 * and this function does not.
 */
export function waitWorkspaceReady(timeoutMs?: number): Promise<boolean> {
  return invoke<boolean>("wait_workspace_ready", { timeoutMs });
}

/**
 * `localStorage` key holding the inline style declarations the theme
 * last installed on `<html>`.
 *
 * The **only** reader is the inline `<head>` script in each client's
 * `index.html`, which cannot import anything (it runs before any module
 * loads, which is the entire point of it) and so holds its own
 * three-line read. That duplication is bounded on purpose: the script
 * reads a key and assigns a string, it does not know what a palette is.
 * The writer is `applyPaletteToRoot`. Per-viewer, per-install, disposable:
 * a cleared or unreadable store just means the first frame paints the
 * brand default, which is what it did before this existed.
 */
export const BOOT_STYLE_KEY = "outl.boot.style";

/**
 * Remember `declarations` as the style to paint before the next
 * launch's first pixel.
 *
 * Best-effort by construction. `localStorage` throws in a private
 * window, with site data blocked, and during a thumbnail capture — and
 * none of those are reasons to fail installing a theme.
 */
export function cacheBootStyle(declarations: string): void {
  try {
    localStorage.setItem(BOOT_STYLE_KEY, declarations);
  } catch {
    // No cache this launch. The next frame paints the real palette.
  }
}

/** The event {@link onPagePreview} subscribes to. */
export const PAGE_PREVIEW_EVENT = "page-preview";

/**
 * Subscribe to the pre-workspace page preview.
 *
 * The backend reads today's journal `.md` and its sidecar off disk
 * before it opens the op log, which costs 8-11 ms regardless of
 * workspace size, against hundreds of milliseconds of snapshot boot
 * plus index load (seconds on a cold one). So this is what a client
 * paints instead of a spinner.
 *
 * **What a client must not do with it.** Not edit it: every mutation
 * needs the workspace, which by definition is not open yet, so the
 * guarantee lives in the renderer — {@link PreviewOutline} has no
 * interaction in it at all, which a flag on the payload could not
 * match. Not treat
 * it as authoritative about anything the op log knows and the file does
 * not yet — a peer's edit that arrived as ops, or a collapsed flag,
 * which lives in the log. And not clear a sticky `md_ahead_of_log`
 * banner on it: `md_ahead_of_log_checked` is `false` precisely because
 * the preview never asked that question.
 *
 * The real view replaces it wholesale when `workspace-ready` lands. The
 * page id is the same (`NodeId::from_slug` is deterministic), so the
 * swap does not remount the outline under the user.
 *
 * Fires at most once per launch, and may not fire at all — a fresh
 * workspace or a day with no journal yet has nothing to preview.
 */
export function onPagePreview(
  handler: (view: PageView) => void,
): Promise<UnlistenFn> {
  return listen<PageView>(PAGE_PREVIEW_EVENT, (e) => handler(e.payload));
}
