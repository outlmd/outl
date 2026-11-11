/**
 * The three commands.
 *
 * `sync` is the one that matters: a content transformer cannot reach the
 * network (it must return its descriptor synchronously, and a blocking fetch
 * on every repaint would stall the plugin thread), so pulling scenes is a
 * separate, explicit act. The transformer then only ever reads the cache.
 *
 * That split is also what makes a drawing work offline: once synced, a scene
 * renders from `storage.json` with no key and no connection.
 */

import type { PluginContext } from "@outl/plugin-sdk";
import { NO_TOKEN_HINT, fetchAllScenes, fetchContent, fetchShared, readToken } from "./api";
import { FILE_BUDGET_BYTES, clear, packFiles, readScene, writeScene } from "./cache";
import { sceneRefsIn, type SceneRef } from "./fence";
import type { CachedScene, PluginConfig, SceneContent, SceneMetadata } from "./types";

/** Substring that narrows `blocks.query` to blocks holding a fence. */
const FENCE_MARKER = "```excalidraw";

/** Most scenes `excalidraw-scenes` will list, and the index cap for `sync`. */
const SCENE_CAP = 300;

/**
 * How long one sync may hold the plugin thread before stopping and saying so.
 *
 * Every fetch is blocking and every share link costs CPU to decrypt, so the
 * work is linear in how many fences the workspace has — and
 * `excalidraw-scenes` will happily write 300 of them into a page. At 200ms
 * each that is a minute of frozen UI on the TUI, and a scene that times out
 * costs 20s on its own.
 *
 * Scenes are taken oldest-cached-first, so the next run picks up where this
 * one stopped instead of redoing the same prefix.
 */
const SYNC_BUDGET_MS = 20_000;

/** Default page the scene index is written to. */
const DEFAULT_INDEX_PAGE = "excalidraw";

/**
 * Pull every scene referenced by a fence in the workspace into the cache.
 *
 * Fetches the scene list once for names, then one content request per
 * referenced scene — rather than two requests per scene — so a page with
 * twenty fences costs twenty-one round trips, not forty.
 */
export async function syncScenes(ctx: PluginContext): Promise<void> {
  const refs = await referencedScenes(ctx);
  if (refs.length === 0) {
    ctx.ui.notify(
      "No Excalidraw scene referenced yet. Add a ```excalidraw fence with `scene: <id>`, or paste a share link.",
    );
    return;
  }

  // A share link carries its own key, so it syncs with no account at all.
  // Only Excalidraw+ ids need the token — asking for one when every fence is
  // a share link would block a flow that has no use for it.
  const shared = refs.filter((r) => typeof r.key === "string");
  const owned = refs.filter((r) => typeof r.key !== "string");

  const token = owned.length > 0 ? await readToken(ctx) : null;
  if (owned.length > 0 && !token) {
    if (shared.length === 0) {
      ctx.ui.notify(NO_TOKEN_HINT);
      return;
    }
    ctx.log.warn(`[excalidraw] ${owned.length} Excalidraw+ scene(s) skipped — ${NO_TOKEN_HINT}`);
  }

  // Names are cosmetic, so a failed listing degrades to ids instead of
  // aborting a sync that would otherwise have worked.
  const names = token ? await sceneNames(ctx, token) : {};

  let synced = 0;
  let deferred = 0;
  const failures: string[] = [];

  const queue = oldestFirst(ctx, refs);
  const started = Date.now();

  for (let i = 0; i < queue.length; i++) {
    if (Date.now() - started > SYNC_BUDGET_MS) {
      deferred = queue.length - i;
      break;
    }

    const ref = queue[i];
    const isShared = typeof ref.key === "string";
    if (!isShared && !token) continue;

    const fetched = isShared
      ? await fetchShared(ctx, ref.id, String(ref.key))
      : await fetchContent(ctx, String(token), ref.id);

    if (!fetched.ok) {
      failures.push(`${ref.id}: ${fetched.error}`);
      ctx.log.warn(`[excalidraw] ${ref.id} not synced — ${fetched.error}`);
      continue;
    }

    await writeScene(ctx, toCachedScene(ref.id, fetched.value, names[ref.id]));
    synced++;
  }

  const skipped = owned.length > 0 && !token ? owned.length : 0;
  ctx.ui.notify(summary(synced, failures, skipped, deferred));
}

/** Fold a fetched scene into the shape the cache and the renderer want. */
function toCachedScene(
  id: string,
  content: SceneContent,
  meta: SceneMetadata | undefined,
): CachedScene {
  const packed = packFiles(content.files, FILE_BUDGET_BYTES);
  return {
    id,
    name: displayName(meta, id),
    sceneVersion: String((meta && meta.sceneVersion) || ""),
    background: (content.appState && content.appState.viewBackgroundColor) || "",
    elements: Array.isArray(content.elements) ? content.elements : [],
    files: packed.files,
    filesDropped: packed.dropped,
    at: new Date().toISOString(),
  };
}

/**
 * Write the workspace's scenes into a page, one block each with a ready-made
 * fence under it.
 *
 * Re-runnable on purpose: it reads what the page already references and appends
 * only what is missing. `appendTree` appends, so without that check running
 * the command twice would double every entry — and a command whose second run
 * damages the page is a command nobody runs.
 */
export async function listScenes(ctx: PluginContext): Promise<void> {
  const token = await readToken(ctx);
  if (!token) {
    ctx.ui.notify(NO_TOKEN_HINT);
    return;
  }

  const scenes = await fetchAllScenes(ctx, token, SCENE_CAP);
  if (!scenes.ok) {
    ctx.ui.notify(`Could not list scenes — ${scenes.error}`);
    return;
  }
  if (scenes.value.length === 0) {
    ctx.ui.notify("That Excalidraw+ workspace has no scenes yet.");
    return;
  }

  const slug = pageSlug(ctx);
  const present = (await referencedScenes(ctx, slug)).map((r) => r.id);

  const tree: { text: string; children?: { text: string }[] }[] = [];
  for (let i = 0; i < scenes.value.length; i++) {
    const meta = scenes.value[i];
    const id = String(meta.id);
    if (present.indexOf(id) !== -1) continue;

    tree.push({
      text: headline(meta, id),
      children: [{ text: "```excalidraw\nscene: " + id + "\n```" }],
    });
  }

  if (tree.length === 0) {
    ctx.ui.notify(`All ${scenes.value.length} scenes are already on [[${slug}]].`);
    return;
  }

  await ctx.page.appendTree(slug, tree);
  ctx.ui.notify(
    `✏️ ${tree.length} scene${tree.length === 1 ? "" : "s"} added to [[${slug}]]` +
      ` · run the sync command to draw them`,
  );
}

/** Drop every cached scene. */
export async function clearCache(ctx: PluginContext): Promise<void> {
  const removed = await clear(ctx);
  ctx.ui.notify(
    removed === 0
      ? "The Excalidraw cache was already empty."
      : `Cleared ${removed} cached scene${removed === 1 ? "" : "s"}.`,
  );
}

/**
 * Order scenes so the least fresh goes first: never cached, then
 * oldest-synced. That is what lets the time budget above make progress — a
 * stable input order would re-sync the same prefix on every run and never
 * reach the tail.
 */
function oldestFirst(ctx: PluginContext, refs: SceneRef[]): SceneRef[] {
  const syncedAt = (ref: SceneRef): number => {
    const hit = readScene(ctx, ref.id);
    if (hit.kind !== "hit") return 0;
    const at = Date.parse(hit.scene.at);
    return Number.isFinite(at) ? at : 0;
  };

  const ordered = refs.slice();
  ordered.sort((a, b) => syncedAt(a) - syncedAt(b));
  return ordered;
}

/** Scenes referenced by fences, across the workspace or on one page. */
async function referencedScenes(ctx: PluginContext, page?: string): Promise<SceneRef[]> {
  const filter = page
    ? { page, textContains: FENCE_MARKER }
    : { textContains: FENCE_MARKER };
  const blocks = await ctx.blocks.query(filter);

  const refs: SceneRef[] = [];
  for (let i = 0; i < blocks.length; i++) {
    const found = sceneRefsIn(blocks[i].text);
    for (let j = 0; j < found.length; j++) {
      if (!refs.some((r) => r.id === found[j].id)) refs.push(found[j]);
    }
  }
  return refs;
}

/** `id` → metadata for every scene the key can see. Empty on failure. */
async function sceneNames(
  ctx: PluginContext,
  token: string,
): Promise<Record<string, SceneMetadata>> {
  const listed = await fetchAllScenes(ctx, token, SCENE_CAP);
  if (!listed.ok) {
    ctx.log.warn(`[excalidraw] scene names unavailable — ${listed.error}`);
    return {};
  }

  const out: Record<string, SceneMetadata> = {};
  for (let i = 0; i < listed.value.length; i++) {
    const meta = listed.value[i];
    if (meta.id) out[String(meta.id)] = meta;
  }
  return out;
}

/** The configured index page, falling back to the default. */
function pageSlug(ctx: PluginContext): string {
  const cfg = ctx.config.get<PluginConfig>() || {};
  const slug = (cfg.indexPage || "").trim();
  return slug === "" ? DEFAULT_INDEX_PAGE : slug;
}

/** A scene's name, or its id when it has none worth showing. */
function displayName(meta: SceneMetadata | undefined, id: string): string {
  if (!meta) return id;
  const name = (meta.name || "").trim();
  if (name === "" || meta.isUntitled) return id;
  return name;
}

/** The block text for one scene in the index page. */
function headline(meta: SceneMetadata, id: string): string {
  const bits: string[] = [`**${displayName(meta, id)}**`];
  if (typeof meta.totalElements === "number") {
    bits.push(`${meta.totalElements} element${meta.totalElements === 1 ? "" : "s"}`);
  }
  const updated = (meta.updated || "").slice(0, 10);
  if (/^\d{4}-\d{2}-\d{2}$/.test(updated)) bits.push(`updated ${updated}`);
  return bits.join(" · ");
}

/**
 * What the sync says when it finishes.
 *
 * Failures are counted in the notification and detailed in the log: a partial
 * sync that reports only its successes leaves a stale drawing on screen with
 * nothing saying so.
 */
function summary(
  synced: number,
  failures: string[],
  skipped: number,
  deferred: number,
): string {
  const parts: string[] = [`✏️ ${synced} scene${synced === 1 ? "" : "s"} synced`];
  if (failures.length > 0) {
    parts.push(`${failures.length} failed (see the log)`);
  }
  if (skipped > 0) {
    parts.push(`${skipped} need an API key`);
  }
  if (deferred > 0) {
    parts.push(`${deferred} left — run it again`);
  }
  return parts.join(" · ");
}
