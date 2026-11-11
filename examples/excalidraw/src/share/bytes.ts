/**
 * Byte plumbing for a shared excalidraw.com link.
 *
 * None of this exists in the plugin runtime: Boa is an ECMAScript engine, so
 * `atob`, `TextDecoder` and `DataView` conveniences from the browser are not
 * there. Each one here is the smallest version that does the job the share
 * payload needs, and nothing more.
 */

const ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/**
 * Character code → 6-bit value, built once.
 *
 * A lookup table rather than `indexOf` per character, and both alphabets in
 * one: the encryption key rides in a URL fragment so it is base64url (`-_`),
 * while the response body from the host is standard base64 (`+/`).
 */
const BASE64_VALUES = buildBase64Table();

function buildBase64Table(): Int16Array {
  const table = new Int16Array(128).fill(-1);
  for (let i = 0; i < ALPHABET.length; i++) {
    table[ALPHABET.charCodeAt(i)] = i;
  }
  table["-".charCodeAt(0)] = 62;
  table["_".charCodeAt(0)] = 63;
  return table;
}

/**
 * Decode standard **or** URL-safe base64 into bytes.
 *
 * Single pass, no intermediate string: this runs on an interpreter where
 * building a cleaned copy character by character costs more than the decode
 * itself.
 */
export function fromBase64(input: string): Uint8Array {
  const out = new Uint8Array(Math.floor((input.length * 3) / 4) + 2);
  let outIndex = 0;
  let buffer = 0;
  let bits = 0;

  for (let i = 0; i < input.length; i++) {
    const code = input.charCodeAt(i);
    if (code === 61 || code === 10 || code === 13 || code === 32) continue; // = \n \r space

    const value = code < 128 ? BASE64_VALUES[code] : -1;
    if (value < 0) throw new Error("invalid base64");

    buffer = (buffer << 6) | value;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out[outIndex++] = (buffer >> bits) & 0xff;
    }
  }
  return out.subarray(0, outIndex);
}

/** Read a big-endian uint32. */
function readU32(bytes: Uint8Array, offset: number): number {
  if (offset + 4 > bytes.length) throw new Error("buffer ended mid-length");
  return (
    ((bytes[offset] << 24) >>> 0) +
    (bytes[offset + 1] << 16) +
    (bytes[offset + 2] << 8) +
    bytes[offset + 3]
  );
}

/**
 * Undo Excalidraw's `concatBuffers`.
 *
 * The layout is a 4-byte big-endian version, then `[u32 length][bytes]` per
 * chunk until the cursor reaches the end. Defined in the app's
 * `packages/excalidraw/data/encode.ts`; the version is 1 and a higher one
 * means the buffer was written by a format this does not know.
 */
export function splitBuffers(bytes: Uint8Array): Uint8Array[] {
  const CONCAT_BUFFERS_VERSION = 1;
  if (bytes.length < 8) throw new Error("buffer too short to be a chunk list");

  const version = readU32(bytes, 0);
  if (version > CONCAT_BUFFERS_VERSION) {
    throw new Error(`unsupported buffer version ${version}`);
  }

  const chunks: Uint8Array[] = [];
  let cursor = 4;
  while (cursor < bytes.length) {
    const size = readU32(bytes, cursor);
    cursor += 4;
    if (cursor + size > bytes.length) throw new Error("chunk runs past the buffer");
    chunks.push(bytes.subarray(cursor, cursor + size));
    cursor += size;
  }
  return chunks;
}

/**
 * Decode UTF-8 bytes to a string.
 *
 * Handles the four sequence lengths and pairs an astral codepoint into
 * surrogates. A malformed sequence becomes U+FFFD rather than throwing: this
 * runs on a scene someone else wrote, and one bad byte should not cost the
 * drawing.
 */
export function utf8(bytes: Uint8Array): string {
  // `out += ch` looks like the slow way to build a string and is the fast
  // one here. Measured on the plugin interpreter over a 31 KB scene:
  // 275ms this way, 332ms via `String.fromCharCode.apply` over 4096- or
  // 512-character chunks, 306ms batching into a local string first. Boa
  // appends cheaply; the chunked versions pay for the array and the `apply`.
  // Don't "optimise" this back without measuring again.
  let out = "";
  let i = 0;

  while (i < bytes.length) {
    const b0 = bytes[i++];

    if (b0 < 0x80) {
      out += String.fromCharCode(b0);
      continue;
    }

    let codepoint = -1;
    if (b0 >= 0xc0 && b0 < 0xe0 && i < bytes.length) {
      codepoint = ((b0 & 0x1f) << 6) | (bytes[i++] & 0x3f);
    } else if (b0 >= 0xe0 && b0 < 0xf0 && i + 1 < bytes.length) {
      codepoint = ((b0 & 0x0f) << 12) | ((bytes[i++] & 0x3f) << 6) | (bytes[i++] & 0x3f);
    } else if (b0 >= 0xf0 && i + 2 < bytes.length) {
      codepoint =
        ((b0 & 0x07) << 18) |
        ((bytes[i++] & 0x3f) << 12) |
        ((bytes[i++] & 0x3f) << 6) |
        (bytes[i++] & 0x3f);
    }

    if (codepoint < 0) {
      out += "\ufffd";
    } else if (codepoint > 0xffff) {
      const astral = codepoint - 0x10000;
      out += String.fromCharCode(0xd800 + (astral >> 10), 0xdc00 + (astral & 0x3ff));
    } else {
      out += String.fromCharCode(codepoint);
    }
  }
  return out;
}
