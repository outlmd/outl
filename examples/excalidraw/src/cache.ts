/**
 * The local scene cache.
 *
 * `excalidraw-sync` writes it; the content transformer reads it. It lives in
 * `ctx.storage` — per-device, plaintext, outside the op log — which is the
 * right home for exactly this: a **projection of someone else's state** that
 * can be thrown away and fetched again. Nothing here needs to converge between
 * devices, so nothing here is modelled as an `Op` (root invariant 7).
 *
 * `ctx.storage` has no `keys()`, so the cache keeps its own index under one
 * key. Without it, "clear the cache" could only clear what the current page
 * happens to reference, and a scene you stopped referencing would be
 * unreachable *and* undeletable.
 */

import type { PluginContext } from "@outl/plugin-sdk";
import type { CachedScene } from "./types";

const INDEX_KEY = "scenes";
const PREFIX = "scene/";

/**
 * Keep the embedded images for a scene only while they are this small, in
 * total. They arrive as base64 `dataURL`s and land in a plaintext
 * `storage.json`, so a few photo-heavy scenes would otherwise turn a cache
 * into a hundred-megabyte file the host re-parses on every turn.
 *
 * Only the cache needs a ceiling: a pasted scene's images are already in the
 * markdown, so nothing is saved by dropping them.
 */
export const FILE_BUDGET_BYTES = 1_500_000;

/**
 * What a cache lookup found — or why it could not look.
 *
 * `missing` and `unavailable` are different facts and the user needs different
 * things from them: the first is fixed by running the sync, the second cannot
 * be fixed by running anything. Collapsing them into `null` is what made an
 * ungranted `storage:local` render as a blank box.
 */
export type CacheRead =
  | { kind: "hit"; scene: CachedScene }
  | { kind: "missing" }
  | { kind: "unavailable"; reason: string };

/**
 * Read a cached scene.
 *
 * **Synchronous on purpose.** A content transformer must return its descriptor
 * within the call, so it cannot `await`; the SDK types `ctx.storage.get` as a
 * `Promise` for a future async runtime, while the shipped Boa runtime resolves
 * it inside the turn (see the describe → apply note in `docs/plugin-api.md`).
 * The cast is that gap, in one place, instead of at every call site.
 *
 * **And it catches.** `ctx.storage.get` *throws* when `storage:local` was not
 * granted — the one host call in this plugin's render path that does. An
 * uncaught throw here leaves the host with no descriptor at all, so the fence
 * draws nothing and says nothing; the user sees an empty block and has no way
 * to learn that a permission is the reason.
 */
export function readScene(ctx: PluginContext, id: string): CacheRead {
  let raw: CachedScene | null;
  try {
    raw = ctx.storage.get<CachedScene>(PREFIX + id) as unknown as CachedScene | null;
  } catch (e) {
    return { kind: "unavailable", reason: String(e) };
  }

  if (!raw || !Array.isArray(raw.elements)) return { kind: "missing" };
  return { kind: "hit", scene: raw };
}

/** Store a scene and record it in the index. */
export async function writeScene(ctx: PluginContext, scene: CachedScene): Promise<void> {
  await ctx.storage.set(PREFIX + scene.id, scene);
  const ids = await readIndex(ctx);
  if (ids.indexOf(scene.id) === -1) {
    ids.push(scene.id);
    await ctx.storage.set(INDEX_KEY, ids);
  }
}

/** Every scene id the cache holds. */
async function readIndex(ctx: PluginContext): Promise<string[]> {
  const ids = await ctx.storage.get<string[]>(INDEX_KEY);
  return Array.isArray(ids) ? ids : [];
}

/** Drop every cached scene. Returns how many were removed. */
export async function clear(ctx: PluginContext): Promise<number> {
  const ids = await readIndex(ctx);
  for (let i = 0; i < ids.length; i++) {
    await ctx.storage.delete(PREFIX + ids[i]);
  }
  await ctx.storage.set(INDEX_KEY, []);
  return ids.length;
}

/**
 * Flatten a scene's `files` map to `fileId` → `dataURL`, dropping the lot when
 * the total goes over `budgetBytes`. Omit the budget for no limit.
 *
 * All-or-nothing, rather than keeping the images that happen to fit: a scene
 * rendered with half its pictures looks like a bug in the renderer, while a
 * scene rendered with none of them says so in the caption.
 */
export function packFiles(
  files: Record<string, { dataURL?: string }> | null | undefined,
  budgetBytes?: number,
): { files: Record<string, string>; dropped: boolean } {
  const out: Record<string, string> = {};
  if (!files) return { files: out, dropped: false };

  let total = 0;
  const ids = Object.keys(files);
  for (let i = 0; i < ids.length; i++) {
    const url = files[ids[i]] && files[ids[i]].dataURL;
    if (typeof url !== "string" || url === "") continue;
    // Only inline images. The field is named `dataURL` but it is whatever the
    // scene's author put there, and the renderer drops it into an
    // `<image href>`: an `https:` value would make the webview fetch it on
    // every repaint. That request never passes `ctx.net.fetch`, so the
    // `network:` allowlist the user approved would not see it — a read
    // receipt and an IP for whoever shared the scene.
    if (!isInlineImage(url)) continue;
    total += url.length;
    if (budgetBytes !== undefined && total > budgetBytes) return { files: {}, dropped: true };
    out[ids[i]] = url;
  }
  return { files: out, dropped: false };
}

/**
 * Is this a self-contained image, rather than something the client would go
 * and fetch?
 *
 * `data:image/...` only. `blob:` and `asset:` cannot resolve inside a
 * null-origin iframe anyway, and every remote scheme is the leak above.
 */
export function isInlineImage(url: string): boolean {
  // Case-insensitive: a URL scheme is, per RFC 3986, and rejecting
  // `DATA:image/png` would drop a legitimate image rather than an attack.
  return /^data:image\/[a-zA-Z0-9.+-]+[;,]/i.test(url);
}
