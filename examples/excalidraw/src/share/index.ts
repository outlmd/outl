/**
 * Reading an excalidraw.com share link.
 *
 * A `#json=<id>,<key>` link is a scene stored on Excalidraw's backend,
 * encrypted with a key that never leaves the URL fragment. Opening one means
 * doing by hand what the web app does with WebCrypto and pako:
 *
 *   GET https://json.excalidraw.com/api/v2/<id>     (bytes, as base64)
 *     → splitBuffers  →  [encoding metadata, IV, cipher text]
 *     → AES-128-GCM decrypt with the fragment's key
 *     → inflate
 *     → splitBuffers  →  [contents metadata, scene JSON]
 *
 * The layout is Excalidraw's `compressData`
 * (`packages/excalidraw/data/encode.ts`); older links skip the two wrappers
 * and are handled as the legacy case below.
 *
 * **This runs in the sync command, never in the transformer** — it needs the
 * network, and a transformer cannot fetch. The result lands in the cache like
 * any other scene, so a shared drawing renders offline afterwards.
 */

import type { SceneContent } from "../types";
import { decryptGcm } from "./aes";
import { fromBase64, splitBuffers, utf8 } from "./bytes";
import { inflate } from "./inflate";

/** Where a `#json=` scene is stored. Needs `network:json.excalidraw.com`. */
export const SHARE_BACKEND = "https://json.excalidraw.com/api/v2/";

/**
 * Largest share payload this will decrypt, in bytes **before** decompression.
 *
 * Decoding is pure JS on an interpreter, and it is linear in the payload:
 * measured at roughly 0.3ms per compressed byte (2.4 KB → ~0.75s), split
 * between AES, inflate and building the JSON string.
 *
 * This bounds the AES and the download. It says **nothing** about how much
 * the payload expands to — DEFLATE reaches about 1029:1 on degenerate input,
 * so 32 KB here can still be 32 MB out. `MAX_SCENE_BYTES` is the ceiling that
 * matters for that, and the two are not interchangeable.
 *
 * Both exist because the work blocks the plugin thread, which on the TUI is
 * the UI. Past either, the sync says so and names the way around it.
 */
const MAX_PAYLOAD_BYTES = 32 * 1024;

/**
 * Largest scene this will decompress to, in bytes.
 *
 * The input ceiling above does **not** bound this: DEFLATE reaches about
 * 1029:1 on degenerate input, so a payload that passes at 32 KB can still
 * expand to 32 MB — measured at roughly five minutes of inflating on this
 * interpreter, a 32 MB buffer and a 67 MB string, which is a kill on mobile.
 *
 * Sized against what the renderer will actually draw: it stops at 1200
 * elements, and 200 elements is about 31 KB of scene JSON, so 256 KB is
 * already more than can appear on screen.
 */
const MAX_SCENE_BYTES = 256 * 1024;

/** A share link, split into the parts needed to fetch and decrypt it. */
export interface ShareRef {
  /** Backend id — also the cache key, so a link syncs like any scene. */
  id: string;
  /** base64url AES-128 key from the URL fragment. Never sent anywhere. */
  key: string;
}

/**
 * Parse `#json=<id>,<key>` out of an excalidraw.com URL.
 *
 * Returns `null` for anything else, including `#room=` — a live collaboration
 * link is a websocket session, not a stored scene, so it has nothing to fetch.
 */
export function parseShareLink(raw: string): ShareRef | null {
  const marker = raw.indexOf("#json=");
  if (marker === -1) return null;

  const payload = raw.slice(marker + "#json=".length).trim();
  const comma = payload.indexOf(",");
  if (comma <= 0) return null;

  const id = payload.slice(0, comma).trim();
  const key = payload.slice(comma + 1).trim();
  if (!/^[A-Za-z0-9_-]+$/.test(id) || !/^[A-Za-z0-9_-]+$/.test(key)) return null;

  return { id, key };
}

/**
 * Decrypt and decompress a share payload into a scene.
 *
 * `body` is the response body as base64 (`ctx.net.fetch` with
 * `encoding: "base64"`); `key` is the fragment's key. Throws with a readable
 * message on anything malformed — the caller turns that into a sync failure
 * the user can see.
 */
export function decodeSharePayload(body: string, key: string): SceneContent {
  const bytes = fromBase64(body);
  const keyBytes = fromBase64(key);

  if (keyBytes.length !== 16) {
    throw new Error(`the link's key is ${keyBytes.length} bytes, expected 16`);
  }
  if (bytes.length === 0) throw new Error("the backend returned an empty payload");
  if (bytes.length > MAX_PAYLOAD_BYTES) {
    throw new Error(
      `that scene is ${Math.round(bytes.length / 1024)} KB compressed, over the ` +
        `${MAX_PAYLOAD_BYTES / 1024} KB this can decrypt without blocking the app for too long — ` +
        `export it as a .excalidraw file and paste it into the fence instead`,
    );
  }

  const json = decodeCurrent(bytes, keyBytes) ?? decodeLegacy(bytes, keyBytes);
  const parsed = JSON.parse(json) as SceneContent;

  if (!parsed || !Array.isArray(parsed.elements)) {
    throw new Error("the decrypted payload is not a scene");
  }
  return parsed;
}

/** The `compressData` layout. Returns `null` when this is not that format. */
function decodeCurrent(bytes: Uint8Array, key: Uint8Array): string | null {
  let chunks: Uint8Array[];
  try {
    chunks = splitBuffers(bytes);
  } catch {
    // Not a chunk list — an older link, handled by the legacy path.
    return null;
  }
  if (chunks.length < 3) return null;

  const metadata = readMetadata(chunks[0]);
  if (metadata === null) return null;

  const plain = decryptGcm(key, chunks[1], chunks[2]);
  const inner = metadata.compressed ? inflate(plain, MAX_SCENE_BYTES) : plain;

  // The inner buffer is itself `concatBuffers(contentsMetadata, sceneJson)`.
  const parts = splitBuffers(inner);
  if (parts.length < 2) throw new Error("the payload has no scene chunk");
  return utf8(parts[1]);
}

/** The encoding metadata chunk, or `null` when it is not JSON we know. */
function readMetadata(chunk: Uint8Array): { compressed: boolean } | null {
  try {
    const info = JSON.parse(utf8(chunk)) as {
      compression?: string | null;
      encryption?: string | null;
    };
    if (!info || typeof info !== "object") return null;
    // `encryption: null` would mean a plaintext payload, which the backend
    // does not produce for a share link; treat it as unknown rather than
    // guessing at a format nobody writes.
    if (info.encryption !== "AES-GCM") return null;
    return { compressed: info.compression === "pako@1" };
  } catch {
    return null;
  }
}

/**
 * Links from before the compressed format: a 12-byte IV followed by the
 * AES-GCM cipher text of the scene JSON, with no chunk framing and no
 * compression.
 */
function decodeLegacy(bytes: Uint8Array, key: Uint8Array): string {
  const IV_LEN = 12;
  if (bytes.length <= IV_LEN) throw new Error("payload is too short to hold a scene");
  return utf8(decryptGcm(key, bytes.subarray(0, IV_LEN), bytes.subarray(IV_LEN)));
}
