/**
 * Parsing the body of a ```excalidraw fence.
 *
 * Two forms, and the choice is the point: a fence either **references** a scene
 * living in Excalidraw+ (`scene: <id>`, synced and cached), or **carries** one
 * (a pasted `.excalidraw` file). The first keeps the markdown short and the
 * drawing editable where it was drawn; the second keeps working with no key,
 * no network and no cache — including on a machine that has never seen the
 * Excalidraw+ workspace.
 *
 * A third form is recognised only to be refused: a `#json=<id>,<key>` share
 * link from excalidraw.com. Those scenes are end-to-end encrypted with the key
 * in the URL fragment, and this plugin has no crypto primitives to decrypt one
 * with — so it says exactly that instead of rendering an empty frame.
 */

import { parseShareLink } from "./share";
import type { SceneContent, Theme } from "./types";

/** What a fence asked for, once parsed. */
export type Fence =
  | { kind: "scene"; ref: SceneRef; opts: FenceOptions }
  | { kind: "inline"; scene: SceneContent; opts: FenceOptions }
  | { kind: "error"; message: string };

/**
 * A scene the sync has to go and get.
 *
 * `key` present means an excalidraw.com share link: the scene lives on the
 * public backend and is decrypted with that key. Absent means an Excalidraw+
 * scene id, fetched from the API with the workspace's token. The id is the
 * cache key either way, so the transformer does not care which it was.
 */
export interface SceneRef {
  id: string;
  key?: string;
}

/** Per-fence rendering overrides. Absent fields fall back to the config. */
export interface FenceOptions {
  theme?: Theme;
  /** Hard height in px for the drawing box. */
  height?: number;
  /** Draw the scene's own background colour behind the drawing. */
  background?: boolean;
  /** Title shown above the drawing; `""` hides the caption entirely. */
  title?: string;
}

/** Keys the fence understands, so a typo is reported rather than ignored. */
const KNOWN_KEYS = ["scene", "theme", "height", "background", "title"];

/**
 * Parse a fence body.
 *
 * JSON (a body whose first non-space character is `{`) is read as a scene file.
 * Anything else is read as `key: value` lines.
 */
export function parseFence(body: string): Fence {
  const trimmed = body.trim();
  if (trimmed === "") {
    return {
      kind: "error",
      message:
        "Empty fence. Use `scene: <id>` for an Excalidraw+ scene, or paste a .excalidraw file.",
    };
  }
  if (trimmed.charAt(0) === "{") return parseInlineScene(trimmed);
  return parseKeyValues(trimmed);
}

/** Read a pasted `.excalidraw` file (or any `{ elements: [...] }` object). */
function parseInlineScene(raw: string): Fence {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch (e) {
    return { kind: "error", message: `Not valid JSON: ${String(e)}` };
  }
  const scene = parsed as SceneContent;
  if (!scene || !Array.isArray(scene.elements)) {
    return {
      kind: "error",
      message: "JSON has no `elements` array — is this a .excalidraw file?",
    };
  }
  return { kind: "inline", scene, opts: {} };
}

/** Read the `key: value` form. */
function parseKeyValues(body: string): Fence {
  const opts: FenceOptions = {};
  let ref: SceneRef | null = null;
  const unknown: string[] = [];

  const lines = body.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    // A comment is `#` followed by space or nothing. Not just a leading `#`:
    // a share link's fragment starts with one, and pasting `#json=<id>,<key>`
    // without the origin would otherwise be dropped in silence.
    if (line === "" || /^#(\s|$)/.test(line)) continue;

    // Split on the first colon only when what precedes it is a key this
    // fence knows. A pasted URL holds one too — `https:` — and reading that
    // as a key turned the documented shorthand (a share link on a line of
    // its own) into "unknown key `https`".
    const colon = line.indexOf(":");
    const candidate = colon === -1 ? "" : line.slice(0, colon).trim().toLowerCase();

    if (candidate === "" || KNOWN_KEYS.indexOf(candidate) === -1) {
      // A bare line is the common shorthand: the scene id, a share link, or
      // an Excalidraw+ URL.
      if (ref === null) {
        const read = readSceneRef(line);
        if (read.kind === "error") return read;
        ref = read.ref;
        continue;
      }
      unknown.push(line);
      continue;
    }

    const key = candidate;
    const value = line.slice(colon + 1).trim();

    switch (key) {
      case "scene": {
        const read = readSceneRef(value);
        if (read.kind === "error") return read;
        ref = read.ref;
        break;
      }
      case "theme": {
        if (value !== "auto" && value !== "light" && value !== "dark") {
          return {
            kind: "error",
            message: `Unknown theme \`${value}\` — use auto, light or dark.`,
          };
        }
        opts.theme = value;
        break;
      }
      case "height": {
        const n = Number(value);
        if (!Number.isFinite(n) || n <= 0) {
          return { kind: "error", message: `\`height: ${value}\` is not a number of pixels.` };
        }
        opts.height = Math.min(4000, Math.round(n));
        break;
      }
      case "background":
        opts.background = value !== "none" && value !== "transparent" && value !== "false";
        break;
      case "title":
        opts.title = value;
        break;
      default:
        unknown.push(key);
    }
  }

  if (unknown.length > 0) {
    return {
      kind: "error",
      message: `Unknown ${unknown.length === 1 ? "key" : "keys"} ${unknown
        .map((k) => `\`${k}\``)
        .join(", ")}. Known: ${KNOWN_KEYS.join(", ")}.`,
    };
  }
  if (ref === null) {
    return {
      kind: "error",
      message:
        "No scene. Use `scene: <id>`, paste an excalidraw.com share link, or paste a .excalidraw file instead.",
    };
  }
  return { kind: "scene", ref, opts };
}

/**
 * Turn whatever the user pasted into a scene id.
 *
 * Accepts a bare id or an Excalidraw+ URL carrying one, and refuses the
 * excalidraw.com link formats **by name**, because each is refused for its own
 * reason and a user who pasted one deserves the specific next step rather than
 * a 404 three seconds later.
 */
function readSceneRef(
  raw: string,
): { kind: "ok"; ref: SceneRef } | { kind: "error"; message: string } {
  const value = raw.trim().replace(/^<|>$/g, "");
  if (value === "") return { kind: "error", message: "`scene:` is empty." };

  // A share link carries its own decryption key in the fragment, so it needs
  // no Excalidraw+ account. The sync fetches and decrypts it.
  const share = parseShareLink(value);
  if (share !== null) return { kind: "ok", ref: share };

  if (value.indexOf("#room=") !== -1) {
    return {
      kind: "error",
      message:
        "That is a live collaboration link, which is a websocket session rather than a stored scene — there is nothing to fetch. Use the share link (Menu → Share → Shareable link), or paste a .excalidraw file.",
    };
  }

  if (value.indexOf("#json=") !== -1) {
    return {
      kind: "error",
      message: `\`${value}\` looks like a share link but its \`#json=<id>,<key>\` part is malformed. Copy it again from Excalidraw (Menu → Share → Shareable link).`,
    };
  }

  if (value.indexOf("#url=") !== -1) {
    return {
      kind: "error",
      message:
        "An excalidraw.com `#url=` link points at a .excalidraw file on some other host, and this plugin may only reach Excalidraw's own domains — the network permissions you approved name them, and config cannot widen a permission. Open that file and paste its contents here instead.",
    };
  }

  if (value.indexOf("://") !== -1) {
    const id = lastPathSegment(value);
    if (id === "") {
      return { kind: "error", message: `No scene id in \`${value}\`.` };
    }
    // An Excalidraw+ URL ends in the scene id; a vanity slug after it would
    // be read as the id and 404. Running `excalidraw-scenes` prints the real
    // ids, so the message names that rather than guessing harder.
    if (value.indexOf("excalidraw.com") === -1) {
      return {
        kind: "error",
        message: `\`${value}\` is not an Excalidraw URL. Use the scene id, or run "Excalidraw: list scenes into a page" to get one.`,
      };
    }
    return { kind: "ok", ref: { id } };
  }

  if (!/^[A-Za-z0-9_-]+$/.test(value)) {
    return { kind: "error", message: `\`${value}\` is not a scene id.` };
  }
  return { kind: "ok", ref: { id: value } };
}

/** Last non-empty path segment of a URL, query and fragment dropped. */
function lastPathSegment(url: string): string {
  let path = url;
  const hash = path.indexOf("#");
  if (hash !== -1) path = path.slice(0, hash);
  const query = path.indexOf("?");
  if (query !== -1) path = path.slice(0, query);

  const parts = path.split("/");
  for (let i = parts.length - 1; i >= 0; i--) {
    const part = parts[i].trim();
    if (part !== "" && /^[A-Za-z0-9_-]+$/.test(part)) return part;
  }
  return "";
}

/**
 * Every scene a block's text references through a ```excalidraw fence.
 *
 * `excalidraw-sync` needs this: it walks the workspace's blocks, so it has raw
 * markdown in hand with no fence parser applied yet. Refs are de-duplicated by
 * id, so one scene referenced from five pages is fetched once.
 */
export function sceneRefsIn(text: string): SceneRef[] {
  const refs: SceneRef[] = [];
  const fence = /```excalidraw[ \t]*\n([\s\S]*?)```/g;

  let match = fence.exec(text);
  while (match !== null) {
    const parsed = parseFence(match[1]);
    if (parsed.kind === "scene" && !refs.some((r) => r.id === parsed.ref.id)) {
      refs.push(parsed.ref);
    }
    match = fence.exec(text);
  }
  return refs;
}
