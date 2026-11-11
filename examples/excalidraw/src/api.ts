/**
 * The Excalidraw+ public API client.
 *
 * Three reads, documented at <https://plus.excalidraw.com/docs/api>:
 *
 *   GET /api/v1/scenes                 — list, paginated with limit/offset
 *   GET /api/v1/scenes/{id}            — metadata (name, version, counts)
 *   GET /api/v1/scenes/{id}/content    — elements + embedded files
 *
 * Reads only. The plugin pulls drawings into notes; pushing a scene back is a
 * `PUT /content` away, and is deliberately not wired — a write that silently
 * replaces a whole scene is not something a render pass should be able to do.
 *
 * Every call returns a result, never throws: `ctx.net.fetch` hands back
 * `{ ok: false, error }` for a host outside the approved domain, and an API
 * key that stopped working is an ordinary Monday, not an exception.
 */

import type { PluginContext } from "@outl/plugin-sdk";
import { SHARE_BACKEND, decodeSharePayload } from "./share";
import type { SceneContent, SceneListResponse, SceneMetadata } from "./types";

/**
 * API origin. Hardcoded because it is *also* the `network:api.excalidraw.com`
 * permission the user approved — making it configurable would promise a
 * self-hosted instance the host would then refuse to connect to. Point a fork
 * elsewhere by changing both, together.
 */
const BASE = "https://api.excalidraw.com/api/v1";

/** A read that either carries a value or says why it does not. */
export type Result<T> = { ok: true; value: T } | { ok: false; error: string };

/** Scene content is the big one; a photo-heavy scene takes a moment. */
const CONTENT_TIMEOUT_MS = 20_000;
const LIST_TIMEOUT_MS = 10_000;

/** Page size for `scenes list`. The API caps `limit` at 100. */
const PAGE_SIZE = 100;

/**
 * Read the API key from the OS keychain.
 *
 * Returns `null` when unset — the caller tells the user how to set it, rather
 * than failing with a bare 401 from a request it should not have sent.
 */
export async function readToken(ctx: PluginContext): Promise<string | null> {
  const token = await ctx.secrets.get("token");
  if (typeof token !== "string" || token.trim() === "") return null;
  return token.trim();
}

/** One sentence telling the user how to set the key. */
export const NO_TOKEN_HINT =
  "No Excalidraw+ API key. Run: outl plugin secret set app.outl.examples.excalidraw token";

/**
 * Fetch and decrypt an excalidraw.com share link's scene.
 *
 * A different backend from the rest of this file, and a different trust
 * story: no token, no account, and the decryption key comes from the link
 * the user pasted rather than from the keychain. The response is **binary**
 * (AES-GCM cipher text), so it is read as base64 — a text read would replace
 * every byte that is not valid UTF-8 and corrupt the payload silently.
 *
 * Needs `network:json.excalidraw.com`.
 */
export async function fetchShared(
  ctx: PluginContext,
  id: string,
  key: string,
): Promise<Result<SceneContent>> {
  const url = SHARE_BACKEND + encode(id);

  let response: Awaited<ReturnType<PluginContext["net"]["fetch"]>>;
  try {
    response = await ctx.net.fetch(url, {
      headers: { Accept: "application/octet-stream" },
      timeoutMs: CONTENT_TIMEOUT_MS,
      encoding: "base64",
    });
  } catch (e) {
    return { ok: false, error: `request failed: ${String(e)}` };
  }

  if (!response.ok) {
    return {
      ok: false,
      error:
        response.status === 404
          ? "that share link has expired or was never stored (HTTP 404)"
          : describeStatus(response.status),
    };
  }

  try {
    return { ok: true, value: decodeSharePayload(await response.text(), key) };
  } catch (e) {
    const reason = String(e);
    // A size refusal is not a key problem, and telling someone to re-check a
    // link that is perfectly fine wastes their time. Those messages already
    // say what to do, so pass them through untouched.
    if (reason.indexOf("limit") !== -1 || reason.indexOf("over the") !== -1) {
      return { ok: false, error: reason.replace(/^Error:\s*/, "") };
    }
    // Otherwise: a wrong key decrypts to noise, which fails at inflate or at
    // JSON.parse rather than announcing itself — so say what it likely is.
    return {
      ok: false,
      error: `could not decrypt the shared scene (${reason}) — is the link complete, key and all?`,
    };
  }
}

/** `GET /scenes/{id}/content`. */
export function fetchContent(
  ctx: PluginContext,
  token: string,
  id: string,
): Promise<Result<SceneContent>> {
  return get<SceneContent>(ctx, token, `/scenes/${encode(id)}/content`, CONTENT_TIMEOUT_MS);
}

/**
 * Every scene in the workspace, following `hasNextPage`.
 *
 * `cap` bounds the walk: each page is a blocking request on the plugin thread,
 * so a workspace with thousands of scenes must not be able to hold the whole
 * plugin surface while it pages through all of them.
 */
export async function fetchAllScenes(
  ctx: PluginContext,
  token: string,
  cap: number,
): Promise<Result<SceneMetadata[]>> {
  const out: SceneMetadata[] = [];
  let offset = 0;

  // Bound the walk by **pages read**, not by rows kept. Deleted scenes are
  // filtered out after the fetch, so a workspace whose next hundred rows are
  // all deleted leaves `out` empty while `hasNextPage` stays true — and a cap
  // on `out.length` never fires. That is an unbounded run of blocking
  // requests, 10s of timeout each, reached by deleting enough scenes.
  const maxPages = Math.ceil(cap / PAGE_SIZE) + 1;

  for (let page = 0; page < maxPages; page++) {
    const response = await get<SceneListResponse>(
      ctx,
      token,
      `/scenes?limit=${PAGE_SIZE}&offset=${offset}`,
      LIST_TIMEOUT_MS,
    );
    if (!response.ok) return response;

    const rows = Array.isArray(response.value.data) ? response.value.data : [];
    for (let i = 0; i < rows.length; i++) {
      const meta = rows[i] && rows[i].metadata;
      if (meta && meta.id && !meta.isDeleted) out.push(meta);
    }

    if (!response.value.hasNextPage || rows.length === 0 || out.length >= cap) break;
    offset += rows.length;
  }

  return { ok: true, value: out.slice(0, cap) };
}

/** Issue one authenticated GET and parse the body. */
async function get<T>(
  ctx: PluginContext,
  token: string,
  path: string,
  timeoutMs: number,
): Promise<Result<T>> {
  const url = BASE + path;
  let response: Awaited<ReturnType<PluginContext["net"]["fetch"]>>;
  try {
    response = await ctx.net.fetch(url, {
      headers: { Authorization: `Bearer ${token}`, Accept: "application/json" },
      timeoutMs,
    });
  } catch (e) {
    // The host returns refusals rather than throwing, so reaching here means
    // something below the gate broke. Report it instead of losing the turn.
    return { ok: false, error: `request failed: ${String(e)}` };
  }

  if (!response.ok) return { ok: false, error: describeStatus(response.status) };

  try {
    return { ok: true, value: (await response.json<T>()) as T };
  } catch (e) {
    return { ok: false, error: `unreadable response: ${String(e)}` };
  }
}

/**
 * Turn a status into something a notification can say.
 *
 * `0` is the host's own refusal — an unapproved host, a blocked redirect, a
 * body over the cap, a timeout — not an answer from the API.
 */
function describeStatus(status: number): string {
  switch (status) {
    case 0:
      return "the request never left (refused by the host, or timed out)";
    case 401:
    case 403:
      return "the API key was rejected (HTTP " + status + ")";
    case 404:
      return "no such scene (HTTP 404)";
    case 429:
      return "rate limited by Excalidraw+ (HTTP 429) — try again shortly";
    default:
      return `HTTP ${status}`;
  }
}

/** Keep a scene id from escaping its path segment. */
function encode(id: string): string {
  return encodeURIComponent(id);
}
