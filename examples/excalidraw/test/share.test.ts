/**
 * The share-link decoder, against payloads built the way excalidraw.com
 * builds them.
 *
 * Node's zlib is the same DEFLATE pako emits and its `aes-128-gcm` is the
 * same cipher WebCrypto uses, so a payload assembled here exercises the real
 * format rather than restating the decoder's own assumptions.
 *
 * The two refusal tests are the point of the file. `MAX_SCENE_BYTES` and
 * `isInlineImage` are each one line, they are the only thing standing between
 * a shared scene and a frozen app or a tracking beacon, and nothing else
 * fails if somebody deletes them.
 */

import { test, expect } from "bun:test";
import { deflateSync, deflateRawSync } from "node:zlib";
import { randomBytes, createCipheriv } from "node:crypto";

import { decodeSharePayload, parseShareLink } from "../src/share";
import { fromBase64, splitBuffers, utf8 } from "../src/share/bytes";
import { inflate } from "../src/share/inflate";
import { decryptGcm } from "../src/share/aes";
import { isInlineImage, packFiles } from "../src/cache";

/** Excalidraw's `concatBuffers`: `[u32 BE version=1] ([u32 BE len][bytes])*`. */
function concatBuffers(...buffers: Buffer[]): Buffer {
  const head = Buffer.alloc(4);
  head.writeUInt32BE(1, 0);
  const parts: Buffer[] = [head];
  for (const b of buffers) {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(b.length, 0);
    parts.push(len, b);
  }
  return Buffer.concat(parts);
}

const KEY = randomBytes(16);
const KEY_B64URL = KEY.toString("base64url");

const SCENE = {
  type: "excalidraw",
  version: 2,
  source: "test",
  appState: { viewBackgroundColor: "#ffffff" },
  elements: [
    { id: "a", type: "rectangle", x: 0, y: 0, width: 100, height: 50, seed: 1 },
    { id: "b", type: "text", x: 0, y: 60, width: 200, height: 25, text: "açaí ✏️ 漢字", seed: 2 },
  ],
};

/** A payload in the current format: concat → deflate → AES-GCM → concat. */
function buildPayload(scene: unknown, opts: { raw?: boolean; body?: Buffer } = {}): string {
  const inner =
    opts.body ??
    concatBuffers(Buffer.from(JSON.stringify(null)), Buffer.from(JSON.stringify(scene)));
  const deflated = opts.raw ? deflateRawSync(inner) : deflateSync(inner, { level: 9 });

  const iv = randomBytes(12);
  const cipher = createCipheriv("aes-128-gcm", KEY, iv);
  const encrypted = Buffer.concat([cipher.update(deflated), cipher.final(), cipher.getAuthTag()]);

  const meta = Buffer.from(
    JSON.stringify({ version: 2, compression: "pako@1", encryption: "AES-GCM" }),
  );
  return concatBuffers(meta, iv, encrypted).toString("base64");
}

test("decodes a scene in the current format, exactly", () => {
  const scene = decodeSharePayload(buildPayload(SCENE), KEY_B64URL);
  expect(scene).toEqual(SCENE);
});

test("non-ASCII text survives the UTF-8 decode", () => {
  const scene = decodeSharePayload(buildPayload(SCENE), KEY_B64URL);
  expect(scene.elements?.[1].text).toBe("açaí ✏️ 漢字");
});

test("a raw DEFLATE stream inflates too", () => {
  const scene = decodeSharePayload(buildPayload(SCENE, { raw: true }), KEY_B64URL);
  expect(scene.elements).toHaveLength(2);
});

test("the legacy format — iv ‖ ciphertext, no framing — still decodes", () => {
  const iv = randomBytes(12);
  const cipher = createCipheriv("aes-128-gcm", KEY, iv);
  const encrypted = Buffer.concat([
    cipher.update(Buffer.from(JSON.stringify(SCENE))),
    cipher.final(),
    cipher.getAuthTag(),
  ]);
  const payload = Buffer.concat([iv, encrypted]).toString("base64");

  expect(decodeSharePayload(payload, KEY_B64URL)).toEqual(SCENE);
});

test("a realistic scene round-trips", () => {
  const big = {
    type: "excalidraw",
    elements: Array.from({ length: 400 }, (_, i) => ({
      id: `el-${i}`,
      type: "rectangle",
      x: i * 7,
      y: (i % 13) * 11,
      width: 120,
      height: 80,
      strokeColor: "#1971c2",
      backgroundColor: "#a5d8ff",
      fillStyle: "hachure",
      seed: i,
    })),
  };
  expect(decodeSharePayload(buildPayload(big), KEY_B64URL)).toEqual(big);
});

test("a compression bomb is refused by the output ceiling", () => {
  // The input ceiling cannot catch this: DEFLATE reaches ~1029:1, so 32 KB of
  // payload expands to 32 MB — minutes of inflating on the plugin
  // interpreter, and a buffer the mobile client gets killed for. Only a cap
  // on the *output* stops it, and nothing else in the suite covers that.
  const bomb = concatBuffers(
    Buffer.from(JSON.stringify(null)),
    Buffer.alloc(32 * 1024 * 1024, 0),
  );
  const payload = buildPayload(null, { body: bomb });

  expect(Buffer.from(payload, "base64").length).toBeLessThan(32 * 1024);

  const started = Date.now();
  expect(() => decodeSharePayload(payload, KEY_B64URL)).toThrow(/limit/);
  expect(Date.now() - started).toBeLessThan(2000);
});

test("a remote image URL is never kept as a scene's file", () => {
  // The field is called `dataURL` and holds whatever the scene's author put
  // there. It lands in an `<image href>`, which the webview fetches — a
  // request the plugin's `network:` allowlist never sees.
  for (const hostile of [
    "https://attacker.example/beacon.png",
    "//attacker.example/beacon.png",
    "http://169.254.169.254/latest/meta-data/",
    "javascript:alert(1)",
    "blob:https://attacker.example/x",
  ]) {
    expect(isInlineImage(hostile)).toBe(false);
    expect(packFiles({ a: { dataURL: hostile } }).files).toEqual({});
  }

  // And a real inline image still gets through, whatever the scheme's case.
  for (const good of ["data:image/png;base64,AAAA", "DATA:image/svg+xml,<svg/>"]) {
    expect(isInlineImage(good)).toBe(true);
  }
});

test("AES-128-GCM matches Node's own cipher", () => {
  const key = Buffer.from("000102030405060708090a0b0c0d0e0f", "hex");
  const iv = Buffer.from("000102030405060708090a0b", "hex");
  const plain = Buffer.from("The quick brown fox jumps over the lazy dog, twice over.");

  const c = createCipheriv("aes-128-gcm", key, iv);
  const ct = Buffer.concat([c.update(plain), c.final(), c.getAuthTag()]);

  const got = decryptGcm(new Uint8Array(key), new Uint8Array(iv), new Uint8Array(ct));
  expect(Buffer.from(got)).toEqual(plain);
});

test("parses a share link, and declines what it cannot fetch", () => {
  expect(parseShareLink("https://excalidraw.com/#json=A1b2C3,xYz-_123")).toEqual({
    id: "A1b2C3",
    key: "xYz-_123",
  });
  expect(parseShareLink("https://excalidraw.com/#room=a,b")).toBeNull();
  expect(parseShareLink("https://excalidraw.com/")).toBeNull();
});

test("a malformed payload fails with a readable reason", () => {
  expect(() => decodeSharePayload("AAAA", "AAAA")).toThrow(/expected 16/);
  expect(() => decodeSharePayload("", KEY_B64URL)).toThrow(/empty payload/);
  expect(() => decodeSharePayload(Buffer.from("junk").toString("base64"), KEY_B64URL)).toThrow();
});

test("the byte primitives", () => {
  const chunks = splitBuffers(new Uint8Array(concatBuffers(Buffer.from("ab"), Buffer.from("cde"))));
  expect(chunks).toHaveLength(2);
  expect(utf8(chunks[1])).toBe("cde");

  expect(utf8(fromBase64("8J-Yig"))).toBe("😊");
  expect(utf8(inflate(new Uint8Array(deflateSync(Buffer.from("xxx"))), 1024))).toBe("xxx");
});
