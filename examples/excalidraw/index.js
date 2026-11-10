"use strict";
(() => {
  // ../../plugin-sdk/src/index.ts
  function definePlugin(def) {
    if (def === null || typeof def !== "object") {
      throw new TypeError("definePlugin: expected a plugin definition object");
    }
    if (typeof def.activate !== "function") {
      throw new TypeError("definePlugin: `activate` must be a function");
    }
    if (def.deactivate !== void 0 && typeof def.deactivate !== "function") {
      throw new TypeError(
        "definePlugin: `deactivate` must be a function when provided"
      );
    }
    const host = globalThis;
    host.__outl_register?.(def);
    return def;
  }

  // src/cache.ts
  var INDEX_KEY = "scenes";
  var PREFIX = "scene/";
  var FILE_BUDGET_BYTES = 15e5;
  function readScene(ctx, id) {
    let raw;
    try {
      raw = ctx.storage.get(PREFIX + id);
    } catch (e) {
      return { kind: "unavailable", reason: String(e) };
    }
    if (!raw || !Array.isArray(raw.elements)) return { kind: "missing" };
    return { kind: "hit", scene: raw };
  }
  async function writeScene(ctx, scene) {
    await ctx.storage.set(PREFIX + scene.id, scene);
    const ids = await readIndex(ctx);
    if (ids.indexOf(scene.id) === -1) {
      ids.push(scene.id);
      await ctx.storage.set(INDEX_KEY, ids);
    }
  }
  async function readIndex(ctx) {
    const ids = await ctx.storage.get(INDEX_KEY);
    return Array.isArray(ids) ? ids : [];
  }
  async function clear(ctx) {
    const ids = await readIndex(ctx);
    for (let i = 0; i < ids.length; i++) {
      await ctx.storage.delete(PREFIX + ids[i]);
    }
    await ctx.storage.set(INDEX_KEY, []);
    return ids.length;
  }
  function packFiles(files, budgetBytes) {
    const out = {};
    if (!files) return { files: out, dropped: false };
    let total = 0;
    const ids = Object.keys(files);
    for (let i = 0; i < ids.length; i++) {
      const url = files[ids[i]] && files[ids[i]].dataURL;
      if (typeof url !== "string" || url === "") continue;
      if (!isInlineImage(url)) continue;
      total += url.length;
      if (budgetBytes !== void 0 && total > budgetBytes) return { files: {}, dropped: true };
      out[ids[i]] = url;
    }
    return { files: out, dropped: false };
  }
  function isInlineImage(url) {
    return /^data:image\/[a-zA-Z0-9.+-]+[;,]/i.test(url);
  }

  // src/share/aes.ts
  var BLOCK = 16;
  var IV_LEN = 12;
  var TAG_LEN = 16;
  var SBOX = buildSbox();
  function buildSbox() {
    const sbox = new Uint8Array(256);
    sbox[0] = 99;
    let p = 1;
    let q = 1;
    do {
      p = (p ^ p << 1 ^ (p & 128 ? 27 : 0)) & 255;
      q ^= q << 1;
      q ^= q << 2;
      q ^= q << 4;
      q &= 255;
      if (q & 128) q ^= 9;
      sbox[p] = (q ^ rotl(q, 1) ^ rotl(q, 2) ^ rotl(q, 3) ^ rotl(q, 4) ^ 99) & 255;
    } while (p !== 1);
    return sbox;
  }
  function rotl(byte, by) {
    return (byte << by | byte >> 8 - by) & 255;
  }
  function xtime(byte) {
    return (byte << 1 ^ (byte & 128 ? 27 : 0)) & 255;
  }
  var XTIME = buildXtime();
  function buildXtime() {
    const table = new Uint8Array(256);
    for (let i = 0; i < 256; i++) table[i] = xtime(i);
    return table;
  }
  function expandKey(key) {
    if (key.length !== 16) throw new Error(`AES-128 needs a 16-byte key, got ${key.length}`);
    const rounds = 10;
    const out = new Uint8Array(BLOCK * (rounds + 1));
    out.set(key, 0);
    let rcon = 1;
    for (let i = 16; i < out.length; i += 4) {
      let a = out[i - 4];
      let b = out[i - 3];
      let c = out[i - 2];
      let d = out[i - 1];
      if (i % 16 === 0) {
        const t = a;
        a = SBOX[b] ^ rcon;
        b = SBOX[c];
        c = SBOX[d];
        d = SBOX[t];
        rcon = xtime(rcon);
      }
      out[i] = out[i - 16] ^ a;
      out[i + 1] = out[i - 15] ^ b;
      out[i + 2] = out[i - 14] ^ c;
      out[i + 3] = out[i - 13] ^ d;
    }
    return out;
  }
  function encryptBlock(input, rk, out) {
    const ROUNDS = 10;
    let s0 = input[0] ^ rk[0];
    let s1 = input[1] ^ rk[1];
    let s2 = input[2] ^ rk[2];
    let s3 = input[3] ^ rk[3];
    let s4 = input[4] ^ rk[4];
    let s5 = input[5] ^ rk[5];
    let s6 = input[6] ^ rk[6];
    let s7 = input[7] ^ rk[7];
    let s8 = input[8] ^ rk[8];
    let s9 = input[9] ^ rk[9];
    let s10 = input[10] ^ rk[10];
    let s11 = input[11] ^ rk[11];
    let s12 = input[12] ^ rk[12];
    let s13 = input[13] ^ rk[13];
    let s14 = input[14] ^ rk[14];
    let s15 = input[15] ^ rk[15];
    for (let round = 1; round <= ROUNDS; round++) {
      s0 = SBOX[s0];
      s1 = SBOX[s1];
      s2 = SBOX[s2];
      s3 = SBOX[s3];
      s4 = SBOX[s4];
      s5 = SBOX[s5];
      s6 = SBOX[s6];
      s7 = SBOX[s7];
      s8 = SBOX[s8];
      s9 = SBOX[s9];
      s10 = SBOX[s10];
      s11 = SBOX[s11];
      s12 = SBOX[s12];
      s13 = SBOX[s13];
      s14 = SBOX[s14];
      s15 = SBOX[s15];
      let t = s1;
      s1 = s5;
      s5 = s9;
      s9 = s13;
      s13 = t;
      t = s2;
      s2 = s10;
      s10 = t;
      t = s6;
      s6 = s14;
      s14 = t;
      t = s15;
      s15 = s11;
      s11 = s7;
      s7 = s3;
      s3 = t;
      if (round < ROUNDS) {
        let all = s0 ^ s1 ^ s2 ^ s3;
        let a = s0;
        let b = s1;
        let c = s2;
        let d = s3;
        s0 = a ^ all ^ XTIME[a ^ b];
        s1 = b ^ all ^ XTIME[b ^ c];
        s2 = c ^ all ^ XTIME[c ^ d];
        s3 = d ^ all ^ XTIME[d ^ a];
        all = s4 ^ s5 ^ s6 ^ s7;
        a = s4;
        b = s5;
        c = s6;
        d = s7;
        s4 = a ^ all ^ XTIME[a ^ b];
        s5 = b ^ all ^ XTIME[b ^ c];
        s6 = c ^ all ^ XTIME[c ^ d];
        s7 = d ^ all ^ XTIME[d ^ a];
        all = s8 ^ s9 ^ s10 ^ s11;
        a = s8;
        b = s9;
        c = s10;
        d = s11;
        s8 = a ^ all ^ XTIME[a ^ b];
        s9 = b ^ all ^ XTIME[b ^ c];
        s10 = c ^ all ^ XTIME[c ^ d];
        s11 = d ^ all ^ XTIME[d ^ a];
        all = s12 ^ s13 ^ s14 ^ s15;
        a = s12;
        b = s13;
        c = s14;
        d = s15;
        s12 = a ^ all ^ XTIME[a ^ b];
        s13 = b ^ all ^ XTIME[b ^ c];
        s14 = c ^ all ^ XTIME[c ^ d];
        s15 = d ^ all ^ XTIME[d ^ a];
      }
      const k = round * BLOCK;
      s0 ^= rk[k];
      s1 ^= rk[k + 1];
      s2 ^= rk[k + 2];
      s3 ^= rk[k + 3];
      s4 ^= rk[k + 4];
      s5 ^= rk[k + 5];
      s6 ^= rk[k + 6];
      s7 ^= rk[k + 7];
      s8 ^= rk[k + 8];
      s9 ^= rk[k + 9];
      s10 ^= rk[k + 10];
      s11 ^= rk[k + 11];
      s12 ^= rk[k + 12];
      s13 ^= rk[k + 13];
      s14 ^= rk[k + 14];
      s15 ^= rk[k + 15];
    }
    out[0] = s0;
    out[1] = s1;
    out[2] = s2;
    out[3] = s3;
    out[4] = s4;
    out[5] = s5;
    out[6] = s6;
    out[7] = s7;
    out[8] = s8;
    out[9] = s9;
    out[10] = s10;
    out[11] = s11;
    out[12] = s12;
    out[13] = s13;
    out[14] = s14;
    out[15] = s15;
  }
  function decryptGcm(key, iv, cipherText) {
    if (iv.length !== IV_LEN) throw new Error(`expected a ${IV_LEN}-byte IV, got ${iv.length}`);
    if (cipherText.length < TAG_LEN) throw new Error("cipher text is shorter than its tag");
    const roundKeys = expandKey(key);
    const body = cipherText.subarray(0, cipherText.length - TAG_LEN);
    const out = new Uint8Array(body.length);
    const counter = new Uint8Array(BLOCK);
    counter.set(iv, 0);
    const keystream = new Uint8Array(BLOCK);
    let block = 2;
    for (let offset = 0; offset < body.length; offset += BLOCK) {
      counter[12] = block >>> 24 & 255;
      counter[13] = block >>> 16 & 255;
      counter[14] = block >>> 8 & 255;
      counter[15] = block & 255;
      block++;
      encryptBlock(counter, roundKeys, keystream);
      const end = Math.min(BLOCK, body.length - offset);
      for (let i = 0; i < end; i++) out[offset + i] = body[offset + i] ^ keystream[i];
    }
    return out;
  }

  // src/share/bytes.ts
  var ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  var BASE64_VALUES = buildBase64Table();
  function buildBase64Table() {
    const table = new Int16Array(128).fill(-1);
    for (let i = 0; i < ALPHABET.length; i++) {
      table[ALPHABET.charCodeAt(i)] = i;
    }
    table["-".charCodeAt(0)] = 62;
    table["_".charCodeAt(0)] = 63;
    return table;
  }
  function fromBase64(input) {
    const out = new Uint8Array(Math.floor(input.length * 3 / 4) + 2);
    let outIndex = 0;
    let buffer = 0;
    let bits = 0;
    for (let i = 0; i < input.length; i++) {
      const code = input.charCodeAt(i);
      if (code === 61 || code === 10 || code === 13 || code === 32) continue;
      const value = code < 128 ? BASE64_VALUES[code] : -1;
      if (value < 0) throw new Error("invalid base64");
      buffer = buffer << 6 | value;
      bits += 6;
      if (bits >= 8) {
        bits -= 8;
        out[outIndex++] = buffer >> bits & 255;
      }
    }
    return out.subarray(0, outIndex);
  }
  function readU32(bytes, offset) {
    if (offset + 4 > bytes.length) throw new Error("buffer ended mid-length");
    return (bytes[offset] << 24 >>> 0) + (bytes[offset + 1] << 16) + (bytes[offset + 2] << 8) + bytes[offset + 3];
  }
  function splitBuffers(bytes) {
    const CONCAT_BUFFERS_VERSION = 1;
    if (bytes.length < 8) throw new Error("buffer too short to be a chunk list");
    const version = readU32(bytes, 0);
    if (version > CONCAT_BUFFERS_VERSION) {
      throw new Error(`unsupported buffer version ${version}`);
    }
    const chunks = [];
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
  function utf8(bytes) {
    let out = "";
    let i = 0;
    while (i < bytes.length) {
      const b0 = bytes[i++];
      if (b0 < 128) {
        out += String.fromCharCode(b0);
        continue;
      }
      let codepoint = -1;
      if (b0 >= 192 && b0 < 224 && i < bytes.length) {
        codepoint = (b0 & 31) << 6 | bytes[i++] & 63;
      } else if (b0 >= 224 && b0 < 240 && i + 1 < bytes.length) {
        codepoint = (b0 & 15) << 12 | (bytes[i++] & 63) << 6 | bytes[i++] & 63;
      } else if (b0 >= 240 && i + 2 < bytes.length) {
        codepoint = (b0 & 7) << 18 | (bytes[i++] & 63) << 12 | (bytes[i++] & 63) << 6 | bytes[i++] & 63;
      }
      if (codepoint < 0) {
        out += "\uFFFD";
      } else if (codepoint > 65535) {
        const astral = codepoint - 65536;
        out += String.fromCharCode(55296 + (astral >> 10), 56320 + (astral & 1023));
      } else {
        out += String.fromCharCode(codepoint);
      }
    }
    return out;
  }

  // src/share/inflate.ts
  var Output = class {
    constructor(limit) {
      this.limit = limit;
      this.buf = new Uint8Array(Math.min(1024, limit));
    }
    limit;
    buf;
    len = 0;
    push(byte) {
      if (this.len === this.buf.length) this.reserve(1);
      this.buf[this.len++] = byte;
    }
    /** Copy `length` bytes from `distance` back — the LZ77 half of DEFLATE. */
    copyBack(distance, length) {
      if (distance > this.len) throw new Error("distance runs before the output");
      this.reserve(length);
      let from = this.len - distance;
      for (let i = 0; i < length; i++) this.buf[this.len++] = this.buf[from++];
    }
    reserve(extra) {
      if (this.len + extra > this.limit) {
        throw new Error(`decompressed past the ${this.limit}-byte limit`);
      }
      if (this.len + extra <= this.buf.length) return;
      let size = this.buf.length * 2;
      while (size < this.len + extra) size *= 2;
      const bigger = new Uint8Array(Math.min(size, this.limit));
      bigger.set(this.buf.subarray(0, this.len));
      this.buf = bigger;
    }
    take() {
      return this.buf.subarray(0, this.len);
    }
  };
  var Bits = class {
    constructor(data) {
      this.data = data;
    }
    data;
    /** Buffered bits, LSB-first. */
    acc = 0;
    /** How many bits in `acc` are valid. */
    count = 0;
    pos = 0;
    /** Buffer at least `n` bits, or as many as the stream has left. */
    fill(n) {
      while (this.count < n && this.pos < this.data.length) {
        this.acc |= this.data[this.pos++] << this.count;
        this.count += 8;
      }
    }
    one() {
      if (this.count === 0) {
        this.fill(1);
        if (this.count === 0) throw new Error("stream ended mid-symbol");
      }
      const value = this.acc & 1;
      this.acc >>>= 1;
      this.count--;
      return value;
    }
    take(count) {
      if (count === 0) return 0;
      this.fill(count);
      if (this.count < count) throw new Error("stream ended mid-field");
      const value = this.acc & (1 << count) - 1;
      this.acc >>>= count;
      this.count -= count;
      return value;
    }
    /** Drop the rest of the current byte — what a stored block starts with. */
    align() {
      const extra = this.count & 7;
      this.acc >>>= extra;
      this.count -= extra;
    }
    readByte() {
      if (this.count >= 8) {
        const value = this.acc & 255;
        this.acc >>>= 8;
        this.count -= 8;
        return value;
      }
      if (this.pos >= this.data.length) throw new Error("stream ended");
      return this.data[this.pos++];
    }
  };
  function huffman(lengths, count) {
    const MAX_BITS = 15;
    const counts = new Int32Array(MAX_BITS + 1);
    for (let i = 0; i < count; i++) counts[lengths[i]]++;
    counts[0] = 0;
    const offsets = new Int32Array(MAX_BITS + 1);
    for (let len = 1; len <= MAX_BITS; len++) {
      offsets[len] = offsets[len - 1] + counts[len - 1];
    }
    const symbols = new Int32Array(count);
    for (let i = 0; i < count; i++) {
      if (lengths[i] !== 0) symbols[offsets[lengths[i]]++] = i;
    }
    return { counts, symbols };
  }
  function decodeSymbol(bits, table) {
    bits.fill(15);
    let acc = bits.acc;
    let count = bits.count;
    let code = 0;
    let first = 0;
    let index = 0;
    for (let len = 1; len <= 15; len++) {
      if (count === 0) throw new Error("stream ended mid-symbol");
      code |= acc & 1;
      acc >>>= 1;
      count--;
      const available = table.counts[len];
      if (code - first < available) {
        bits.acc = acc;
        bits.count = count;
        return table.symbols[index + (code - first)];
      }
      index += available;
      first = first + available << 1;
      code <<= 1;
    }
    throw new Error("invalid Huffman code");
  }
  var LENGTH_BASE = [
    3,
    4,
    5,
    6,
    7,
    8,
    9,
    10,
    11,
    13,
    15,
    17,
    19,
    23,
    27,
    31,
    35,
    43,
    51,
    59,
    67,
    83,
    99,
    115,
    131,
    163,
    195,
    227,
    258
  ];
  var LENGTH_EXTRA = [
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    1,
    1,
    1,
    1,
    2,
    2,
    2,
    2,
    3,
    3,
    3,
    3,
    4,
    4,
    4,
    4,
    5,
    5,
    5,
    5,
    0
  ];
  var DIST_BASE = [
    1,
    2,
    3,
    4,
    5,
    7,
    9,
    13,
    17,
    25,
    33,
    49,
    65,
    97,
    129,
    193,
    257,
    385,
    513,
    769,
    1025,
    1537,
    2049,
    3073,
    4097,
    6145,
    8193,
    12289,
    16385,
    24577
  ];
  var DIST_EXTRA = [
    0,
    0,
    0,
    0,
    1,
    1,
    2,
    2,
    3,
    3,
    4,
    4,
    5,
    5,
    6,
    6,
    7,
    7,
    8,
    8,
    9,
    9,
    10,
    10,
    11,
    11,
    12,
    12,
    13,
    13
  ];
  var CODE_LENGTH_ORDER = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
  function inflateBlock(bits, out, lit, dist) {
    for (; ; ) {
      const symbol = decodeSymbol(bits, lit);
      if (symbol < 256) {
        out.push(symbol);
        continue;
      }
      if (symbol === 256) return;
      const lengthCode = symbol - 257;
      if (lengthCode >= LENGTH_BASE.length) throw new Error("invalid length code");
      const length = LENGTH_BASE[lengthCode] + bits.take(LENGTH_EXTRA[lengthCode]);
      const distCode = decodeSymbol(bits, dist);
      if (distCode >= DIST_BASE.length) throw new Error("invalid distance code");
      const distance = DIST_BASE[distCode] + bits.take(DIST_EXTRA[distCode]);
      out.copyBack(distance, length);
    }
  }
  function fixedTables() {
    const litLengths = new Int32Array(288);
    for (let i = 0; i < 144; i++) litLengths[i] = 8;
    for (let i = 144; i < 256; i++) litLengths[i] = 9;
    for (let i = 256; i < 280; i++) litLengths[i] = 7;
    for (let i = 280; i < 288; i++) litLengths[i] = 8;
    const distLengths = new Int32Array(30);
    for (let i = 0; i < 30; i++) distLengths[i] = 5;
    return { lit: huffman(litLengths, 288), dist: huffman(distLengths, 30) };
  }
  function dynamicTables(bits) {
    const litCount = bits.take(5) + 257;
    const distCount = bits.take(5) + 1;
    const codeCount = bits.take(4) + 4;
    const codeLengths = new Int32Array(19);
    for (let i2 = 0; i2 < codeCount; i2++) {
      codeLengths[CODE_LENGTH_ORDER[i2]] = bits.take(3);
    }
    const codeTable = huffman(codeLengths, 19);
    const lengths = new Int32Array(litCount + distCount);
    let i = 0;
    while (i < lengths.length) {
      const symbol = decodeSymbol(bits, codeTable);
      if (symbol < 16) {
        lengths[i++] = symbol;
      } else if (symbol === 16) {
        if (i === 0) throw new Error("repeat with no previous length");
        const previous = lengths[i - 1];
        let repeat = 3 + bits.take(2);
        while (repeat-- > 0 && i < lengths.length) lengths[i++] = previous;
      } else if (symbol === 17) {
        let repeat = 3 + bits.take(3);
        while (repeat-- > 0 && i < lengths.length) lengths[i++] = 0;
      } else {
        let repeat = 11 + bits.take(7);
        while (repeat-- > 0 && i < lengths.length) lengths[i++] = 0;
      }
    }
    return {
      lit: huffman(lengths.subarray(0, litCount), litCount),
      dist: huffman(lengths.subarray(litCount), distCount)
    };
  }
  function inflate(input, limitBytes) {
    let data = input;
    if (data.length > 2 && (data[0] & 15) === 8 && (data[0] << 8 | data[1]) % 31 === 0 && (data[1] & 32) === 0) {
      data = data.subarray(2);
    }
    const bits = new Bits(data);
    const out = new Output(limitBytes);
    const fixed = fixedTables();
    for (; ; ) {
      const final = bits.one();
      const type = bits.take(2);
      if (type === 0) {
        bits.align();
        const len = bits.readByte() | bits.readByte() << 8;
        bits.readByte();
        bits.readByte();
        for (let i = 0; i < len; i++) out.push(bits.readByte());
      } else if (type === 1) {
        inflateBlock(bits, out, fixed.lit, fixed.dist);
      } else if (type === 2) {
        const tables = dynamicTables(bits);
        inflateBlock(bits, out, tables.lit, tables.dist);
      } else {
        throw new Error("invalid block type");
      }
      if (final) break;
    }
    return out.take();
  }

  // src/share/index.ts
  var SHARE_BACKEND = "https://json.excalidraw.com/api/v2/";
  var MAX_PAYLOAD_BYTES = 32 * 1024;
  var MAX_SCENE_BYTES = 256 * 1024;
  function parseShareLink(raw) {
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
  function decodeSharePayload(body, key) {
    const bytes = fromBase64(body);
    const keyBytes = fromBase64(key);
    if (keyBytes.length !== 16) {
      throw new Error(`the link's key is ${keyBytes.length} bytes, expected 16`);
    }
    if (bytes.length === 0) throw new Error("the backend returned an empty payload");
    if (bytes.length > MAX_PAYLOAD_BYTES) {
      throw new Error(
        `that scene is ${Math.round(bytes.length / 1024)} KB compressed, over the ${MAX_PAYLOAD_BYTES / 1024} KB this can decrypt without blocking the app for too long \u2014 export it as a .excalidraw file and paste it into the fence instead`
      );
    }
    const json = decodeCurrent(bytes, keyBytes) ?? decodeLegacy(bytes, keyBytes);
    const parsed = JSON.parse(json);
    if (!parsed || !Array.isArray(parsed.elements)) {
      throw new Error("the decrypted payload is not a scene");
    }
    return parsed;
  }
  function decodeCurrent(bytes, key) {
    let chunks;
    try {
      chunks = splitBuffers(bytes);
    } catch {
      return null;
    }
    if (chunks.length < 3) return null;
    const metadata = readMetadata(chunks[0]);
    if (metadata === null) return null;
    const plain = decryptGcm(key, chunks[1], chunks[2]);
    const inner = metadata.compressed ? inflate(plain, MAX_SCENE_BYTES) : plain;
    const parts = splitBuffers(inner);
    if (parts.length < 2) throw new Error("the payload has no scene chunk");
    return utf8(parts[1]);
  }
  function readMetadata(chunk) {
    try {
      const info = JSON.parse(utf8(chunk));
      if (!info || typeof info !== "object") return null;
      if (info.encryption !== "AES-GCM") return null;
      return { compressed: info.compression === "pako@1" };
    } catch {
      return null;
    }
  }
  function decodeLegacy(bytes, key) {
    const IV_LEN2 = 12;
    if (bytes.length <= IV_LEN2) throw new Error("payload is too short to hold a scene");
    return utf8(decryptGcm(key, bytes.subarray(0, IV_LEN2), bytes.subarray(IV_LEN2)));
  }

  // src/api.ts
  var BASE = "https://api.excalidraw.com/api/v1";
  var CONTENT_TIMEOUT_MS = 2e4;
  var LIST_TIMEOUT_MS = 1e4;
  var PAGE_SIZE = 100;
  async function readToken(ctx) {
    const token = await ctx.secrets.get("token");
    if (typeof token !== "string" || token.trim() === "") return null;
    return token.trim();
  }
  var NO_TOKEN_HINT = "No Excalidraw+ API key. Run: outl plugin secret set app.outl.examples.excalidraw token";
  async function fetchShared(ctx, id, key) {
    const url = SHARE_BACKEND + encode(id);
    let response;
    try {
      response = await ctx.net.fetch(url, {
        headers: { Accept: "application/octet-stream" },
        timeoutMs: CONTENT_TIMEOUT_MS,
        encoding: "base64"
      });
    } catch (e) {
      return { ok: false, error: `request failed: ${String(e)}` };
    }
    if (!response.ok) {
      return {
        ok: false,
        error: response.status === 404 ? "that share link has expired or was never stored (HTTP 404)" : describeStatus(response.status)
      };
    }
    try {
      return { ok: true, value: decodeSharePayload(await response.text(), key) };
    } catch (e) {
      const reason = String(e);
      if (reason.indexOf("limit") !== -1 || reason.indexOf("over the") !== -1) {
        return { ok: false, error: reason.replace(/^Error:\s*/, "") };
      }
      return {
        ok: false,
        error: `could not decrypt the shared scene (${reason}) \u2014 is the link complete, key and all?`
      };
    }
  }
  function fetchContent(ctx, token, id) {
    return get(ctx, token, `/scenes/${encode(id)}/content`, CONTENT_TIMEOUT_MS);
  }
  async function fetchAllScenes(ctx, token, cap) {
    const out = [];
    let offset = 0;
    const maxPages = Math.ceil(cap / PAGE_SIZE) + 1;
    for (let page = 0; page < maxPages; page++) {
      const response = await get(
        ctx,
        token,
        `/scenes?limit=${PAGE_SIZE}&offset=${offset}`,
        LIST_TIMEOUT_MS
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
  async function get(ctx, token, path, timeoutMs) {
    const url = BASE + path;
    let response;
    try {
      response = await ctx.net.fetch(url, {
        headers: { Authorization: `Bearer ${token}`, Accept: "application/json" },
        timeoutMs
      });
    } catch (e) {
      return { ok: false, error: `request failed: ${String(e)}` };
    }
    if (!response.ok) return { ok: false, error: describeStatus(response.status) };
    try {
      return { ok: true, value: await response.json() };
    } catch (e) {
      return { ok: false, error: `unreadable response: ${String(e)}` };
    }
  }
  function describeStatus(status) {
    switch (status) {
      case 0:
        return "the request never left (refused by the host, or timed out)";
      case 401:
      case 403:
        return "the API key was rejected (HTTP " + status + ")";
      case 404:
        return "no such scene (HTTP 404)";
      case 429:
        return "rate limited by Excalidraw+ (HTTP 429) \u2014 try again shortly";
      default:
        return `HTTP ${status}`;
    }
  }
  function encode(id) {
    return encodeURIComponent(id);
  }

  // src/fence.ts
  var KNOWN_KEYS = ["scene", "theme", "height", "background", "title"];
  function parseFence(body) {
    const trimmed = body.trim();
    if (trimmed === "") {
      return {
        kind: "error",
        message: "Empty fence. Use `scene: <id>` for an Excalidraw+ scene, or paste a .excalidraw file."
      };
    }
    if (trimmed.charAt(0) === "{") return parseInlineScene(trimmed);
    return parseKeyValues(trimmed);
  }
  function parseInlineScene(raw) {
    let parsed;
    try {
      parsed = JSON.parse(raw);
    } catch (e) {
      return { kind: "error", message: `Not valid JSON: ${String(e)}` };
    }
    const scene = parsed;
    if (!scene || !Array.isArray(scene.elements)) {
      return {
        kind: "error",
        message: "JSON has no `elements` array \u2014 is this a .excalidraw file?"
      };
    }
    return { kind: "inline", scene, opts: {} };
  }
  function parseKeyValues(body) {
    const opts = {};
    let ref = null;
    const unknown = [];
    const lines = body.split("\n");
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i].trim();
      if (line === "" || /^#(\s|$)/.test(line)) continue;
      const colon = line.indexOf(":");
      const candidate = colon === -1 ? "" : line.slice(0, colon).trim().toLowerCase();
      if (candidate === "" || KNOWN_KEYS.indexOf(candidate) === -1) {
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
              message: `Unknown theme \`${value}\` \u2014 use auto, light or dark.`
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
          opts.height = Math.min(4e3, Math.round(n));
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
        message: `Unknown ${unknown.length === 1 ? "key" : "keys"} ${unknown.map((k) => `\`${k}\``).join(", ")}. Known: ${KNOWN_KEYS.join(", ")}.`
      };
    }
    if (ref === null) {
      return {
        kind: "error",
        message: "No scene. Use `scene: <id>`, paste an excalidraw.com share link, or paste a .excalidraw file instead."
      };
    }
    return { kind: "scene", ref, opts };
  }
  function readSceneRef(raw) {
    const value = raw.trim().replace(/^<|>$/g, "");
    if (value === "") return { kind: "error", message: "`scene:` is empty." };
    const share = parseShareLink(value);
    if (share !== null) return { kind: "ok", ref: share };
    if (value.indexOf("#room=") !== -1) {
      return {
        kind: "error",
        message: "That is a live collaboration link, which is a websocket session rather than a stored scene \u2014 there is nothing to fetch. Use the share link (Menu \u2192 Share \u2192 Shareable link), or paste a .excalidraw file."
      };
    }
    if (value.indexOf("#json=") !== -1) {
      return {
        kind: "error",
        message: `\`${value}\` looks like a share link but its \`#json=<id>,<key>\` part is malformed. Copy it again from Excalidraw (Menu \u2192 Share \u2192 Shareable link).`
      };
    }
    if (value.indexOf("#url=") !== -1) {
      return {
        kind: "error",
        message: "An excalidraw.com `#url=` link points at a .excalidraw file on some other host, and this plugin may only reach Excalidraw's own domains \u2014 the network permissions you approved name them, and config cannot widen a permission. Open that file and paste its contents here instead."
      };
    }
    if (value.indexOf("://") !== -1) {
      const id = lastPathSegment(value);
      if (id === "") {
        return { kind: "error", message: `No scene id in \`${value}\`.` };
      }
      if (value.indexOf("excalidraw.com") === -1) {
        return {
          kind: "error",
          message: `\`${value}\` is not an Excalidraw URL. Use the scene id, or run "Excalidraw: list scenes into a page" to get one.`
        };
      }
      return { kind: "ok", ref: { id } };
    }
    if (!/^[A-Za-z0-9_-]+$/.test(value)) {
      return { kind: "error", message: `\`${value}\` is not a scene id.` };
    }
    return { kind: "ok", ref: { id: value } };
  }
  function lastPathSegment(url) {
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
  function sceneRefsIn(text) {
    const refs = [];
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

  // src/commands.ts
  var FENCE_MARKER = "```excalidraw";
  var SCENE_CAP = 300;
  var SYNC_BUDGET_MS = 2e4;
  var DEFAULT_INDEX_PAGE = "excalidraw";
  async function syncScenes(ctx) {
    const refs = await referencedScenes(ctx);
    if (refs.length === 0) {
      ctx.ui.notify(
        "No Excalidraw scene referenced yet. Add a ```excalidraw fence with `scene: <id>`, or paste a share link."
      );
      return;
    }
    const shared = refs.filter((r) => typeof r.key === "string");
    const owned = refs.filter((r) => typeof r.key !== "string");
    const token = owned.length > 0 ? await readToken(ctx) : null;
    if (owned.length > 0 && !token) {
      if (shared.length === 0) {
        ctx.ui.notify(NO_TOKEN_HINT);
        return;
      }
      ctx.log.warn(`[excalidraw] ${owned.length} Excalidraw+ scene(s) skipped \u2014 ${NO_TOKEN_HINT}`);
    }
    const names = token ? await sceneNames(ctx, token) : {};
    let synced = 0;
    let deferred = 0;
    const failures = [];
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
      const fetched = isShared ? await fetchShared(ctx, ref.id, String(ref.key)) : await fetchContent(ctx, String(token), ref.id);
      if (!fetched.ok) {
        failures.push(`${ref.id}: ${fetched.error}`);
        ctx.log.warn(`[excalidraw] ${ref.id} not synced \u2014 ${fetched.error}`);
        continue;
      }
      await writeScene(ctx, toCachedScene(ref.id, fetched.value, names[ref.id]));
      synced++;
    }
    const skipped = owned.length > 0 && !token ? owned.length : 0;
    ctx.ui.notify(summary(synced, failures, skipped, deferred));
  }
  function toCachedScene(id, content, meta) {
    const packed = packFiles(content.files, FILE_BUDGET_BYTES);
    return {
      id,
      name: displayName(meta, id),
      sceneVersion: String(meta && meta.sceneVersion || ""),
      background: content.appState && content.appState.viewBackgroundColor || "",
      elements: Array.isArray(content.elements) ? content.elements : [],
      files: packed.files,
      filesDropped: packed.dropped,
      at: (/* @__PURE__ */ new Date()).toISOString()
    };
  }
  async function listScenes(ctx) {
    const token = await readToken(ctx);
    if (!token) {
      ctx.ui.notify(NO_TOKEN_HINT);
      return;
    }
    const scenes = await fetchAllScenes(ctx, token, SCENE_CAP);
    if (!scenes.ok) {
      ctx.ui.notify(`Could not list scenes \u2014 ${scenes.error}`);
      return;
    }
    if (scenes.value.length === 0) {
      ctx.ui.notify("That Excalidraw+ workspace has no scenes yet.");
      return;
    }
    const slug2 = pageSlug(ctx);
    const present = (await referencedScenes(ctx, slug2)).map((r) => r.id);
    const tree = [];
    for (let i = 0; i < scenes.value.length; i++) {
      const meta = scenes.value[i];
      const id = String(meta.id);
      if (present.indexOf(id) !== -1) continue;
      tree.push({
        text: headline(meta, id),
        children: [{ text: "```excalidraw\nscene: " + id + "\n```" }]
      });
    }
    if (tree.length === 0) {
      ctx.ui.notify(`All ${scenes.value.length} scenes are already on [[${slug2}]].`);
      return;
    }
    await ctx.page.appendTree(slug2, tree);
    ctx.ui.notify(
      `\u270F\uFE0F ${tree.length} scene${tree.length === 1 ? "" : "s"} added to [[${slug2}]] \xB7 run the sync command to draw them`
    );
  }
  async function clearCache(ctx) {
    const removed = await clear(ctx);
    ctx.ui.notify(
      removed === 0 ? "The Excalidraw cache was already empty." : `Cleared ${removed} cached scene${removed === 1 ? "" : "s"}.`
    );
  }
  function oldestFirst(ctx, refs) {
    const syncedAt = (ref) => {
      const hit = readScene(ctx, ref.id);
      if (hit.kind !== "hit") return 0;
      const at = Date.parse(hit.scene.at);
      return Number.isFinite(at) ? at : 0;
    };
    const ordered = refs.slice();
    ordered.sort((a, b) => syncedAt(a) - syncedAt(b));
    return ordered;
  }
  async function referencedScenes(ctx, page) {
    const filter = page ? { page, textContains: FENCE_MARKER } : { textContains: FENCE_MARKER };
    const blocks = await ctx.blocks.query(filter);
    const refs = [];
    for (let i = 0; i < blocks.length; i++) {
      const found = sceneRefsIn(blocks[i].text);
      for (let j = 0; j < found.length; j++) {
        if (!refs.some((r) => r.id === found[j].id)) refs.push(found[j]);
      }
    }
    return refs;
  }
  async function sceneNames(ctx, token) {
    const listed = await fetchAllScenes(ctx, token, SCENE_CAP);
    if (!listed.ok) {
      ctx.log.warn(`[excalidraw] scene names unavailable \u2014 ${listed.error}`);
      return {};
    }
    const out = {};
    for (let i = 0; i < listed.value.length; i++) {
      const meta = listed.value[i];
      if (meta.id) out[String(meta.id)] = meta;
    }
    return out;
  }
  function pageSlug(ctx) {
    const cfg = ctx.config.get() || {};
    const slug2 = (cfg.indexPage || "").trim();
    return slug2 === "" ? DEFAULT_INDEX_PAGE : slug2;
  }
  function displayName(meta, id) {
    if (!meta) return id;
    const name = (meta.name || "").trim();
    if (name === "" || meta.isUntitled) return id;
    return name;
  }
  function headline(meta, id) {
    const bits = [`**${displayName(meta, id)}**`];
    if (typeof meta.totalElements === "number") {
      bits.push(`${meta.totalElements} element${meta.totalElements === 1 ? "" : "s"}`);
    }
    const updated = (meta.updated || "").slice(0, 10);
    if (/^\d{4}-\d{2}-\d{2}$/.test(updated)) bits.push(`updated ${updated}`);
    return bits.join(" \xB7 ");
  }
  function summary(synced, failures, skipped, deferred) {
    const parts = [`\u270F\uFE0F ${synced} scene${synced === 1 ? "" : "s"} synced`];
    if (failures.length > 0) {
      parts.push(`${failures.length} failed (see the log)`);
    }
    if (skipped > 0) {
      parts.push(`${skipped} need an API key`);
    }
    if (deferred > 0) {
      parts.push(`${deferred} left \u2014 run it again`);
    }
    return parts.join(" \xB7 ");
  }

  // src/escape.ts
  function escape(value) {
    return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  }
  function asString(value) {
    return typeof value === "string" ? value.trim() : "";
  }

  // src/html.ts
  function shell(bodyClass, body, styles) {
    return `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'; script-src 'unsafe-inline'"><style>:root{color-scheme:light dark}html,body{margin:0;background:transparent;font:13px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif}` + styles + `</style></head><body class="${bodyClass}">${body}<script>requestAnimationFrame(function(){var h=Math.ceil(document.body.getBoundingClientRect().height);parent.postMessage({outlHeight:h},'*');});</script></body></html>`;
  }
  var DARK_FILTER = "invert(93%) hue-rotate(180deg)";
  function drawingPage(svg, opts) {
    const ratio = opts.intrinsicHeight > 0 ? opts.width / opts.intrinsicHeight : 1;
    const sizing = typeof opts.height === "number" ? `height:${opts.height}px` : `aspect-ratio:${ratio.toFixed(4)};max-height:${opts.maxHeight}px`;
    const styles = `.scene{${sizing};width:100%;display:flex;align-items:center;justify-content:center}.scene svg{max-width:100%;max-height:100%}.caption{padding:6px 2px 2px;opacity:.6;font-size:12px}` + (opts.dark ? `.scene{filter:${DARK_FILTER}}` : "") + (opts.auto ? `@media (prefers-color-scheme:dark){.scene{filter:${DARK_FILTER}}}` : "");
    const caption2 = opts.caption === "" ? "" : `<div class="caption">${escape(opts.caption)}</div>`;
    return shell("", `<div class="scene">${svg}</div>${caption2}`, styles);
  }
  function messagePage(title, detail, command) {
    const styles = `.card{padding:12px 14px;border-radius:8px;border:1px solid rgba(128,128,128,.28);background:rgba(128,128,128,.06)}.title{font-weight:600;margin-bottom:4px}.detail{opacity:.75}code{font:12px/1.5 ui-monospace,SFMono-Regular,Menlo,monospace;background:rgba(128,128,128,.16);padding:1px 5px;border-radius:4px;display:inline-block;margin-top:8px;word-break:break-all}`;
    const hint = command ? `<code>${escape(command)}</code>` : "";
    const body = `<div class="card"><div class="title">\u270F\uFE0F ${escape(title)}</div><div class="detail">${escape(detail)}</div>${hint}</div>`;
    return shell("", body, styles);
  }
  function caption(parts) {
    const bits = [];
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
    return bits.join(" \xB7 ");
  }
  function relativeTime(iso) {
    const then = Date.parse(iso);
    if (!Number.isFinite(then)) return "";
    const seconds = Math.max(0, Math.round((Date.now() - then) / 1e3));
    if (seconds < 60) return "just now";
    const minutes = Math.round(seconds / 60);
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.round(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.round(hours / 24);
    return `${days}d ago`;
  }

  // src/render/rough.ts
  function seeded(seed) {
    let s = Math.abs(Math.floor(seed || 1)) % 2147483647;
    if (s <= 0) s += 2147483646;
    return () => {
      s = s * 16807 % 2147483647;
      return (s - 1) / 2147483646;
    };
  }
  function jitter(roughness) {
    const r = typeof roughness === "number" ? roughness : 1;
    if (r <= 0) return 0;
    return Math.min(2.4, 0.9 * r);
  }
  function off(rand, amount) {
    return amount === 0 ? 0 : (rand() * 2 - 1) * amount;
  }
  function r2(n) {
    return Math.round(n * 100) / 100;
  }
  function along(ax, ay, bx, by, d) {
    const dx = bx - ax;
    const dy = by - ay;
    const len = Math.hypot(dx, dy);
    if (len === 0 || d <= 0) return [ax, ay];
    const t = Math.min(1, d / len);
    return [ax + dx * t, ay + dy * t];
  }
  function sketchEdge(rand, x1, y1, x2, y2, amount) {
    const mx = (x1 + x2) / 2 + off(rand, amount * 1.8);
    const my = (y1 + y2) / 2 + off(rand, amount * 1.8);
    const sx = x1 + off(rand, amount);
    const sy = y1 + off(rand, amount);
    const ex = x2 + off(rand, amount);
    const ey = y2 + off(rand, amount);
    return `M ${r2(sx)} ${r2(sy)} Q ${r2(mx)} ${r2(my)} ${r2(ex)} ${r2(ey)}`;
  }
  function doubled(build, amount) {
    const first = build();
    if (amount === 0) return first;
    return `${first} ${build()}`;
  }
  function corners(pts, radius) {
    const out = [];
    for (let i = 0; i < pts.length; i++) {
      const prev = pts[(i + pts.length - 1) % pts.length];
      const vertex = pts[i];
      const next = pts[(i + 1) % pts.length];
      const edgeIn = Math.hypot(vertex[0] - prev[0], vertex[1] - prev[1]);
      const edgeOut = Math.hypot(next[0] - vertex[0], next[1] - vertex[1]);
      const rIn = Math.min(radius, edgeIn / 2);
      const rOut = Math.min(radius, edgeOut / 2);
      out.push({
        start: along(vertex[0], vertex[1], prev[0], prev[1], rIn),
        vertex,
        end: along(vertex[0], vertex[1], next[0], next[1], rOut)
      });
    }
    return out;
  }
  function polygonFill(pts, radius) {
    if (pts.length < 3) return "";
    const cs = corners(pts, radius);
    let d = `M ${r2(cs[0].start[0])} ${r2(cs[0].start[1])}`;
    for (let i = 0; i < cs.length; i++) {
      const c = cs[i];
      if (radius > 0) {
        d += ` Q ${r2(c.vertex[0])} ${r2(c.vertex[1])} ${r2(c.end[0])} ${r2(c.end[1])}`;
      } else {
        d += ` L ${r2(c.vertex[0])} ${r2(c.vertex[1])}`;
      }
      const next = cs[(i + 1) % cs.length];
      d += ` L ${r2(next.start[0])} ${r2(next.start[1])}`;
    }
    return d + " Z";
  }
  function polygonSketch(rand, pts, radius, amount) {
    if (pts.length < 2) return "";
    const cs = corners(pts, radius);
    let d = "";
    for (let i = 0; i < cs.length; i++) {
      const from = cs[i].end;
      const to = cs[(i + 1) % cs.length].start;
      d += (d === "" ? "" : " ") + sketchEdge(rand, from[0], from[1], to[0], to[1], amount);
      if (radius > 0) {
        const c = cs[(i + 1) % cs.length];
        d += ` M ${r2(c.start[0] + off(rand, amount))} ${r2(c.start[1] + off(rand, amount))} Q ${r2(c.vertex[0])} ${r2(c.vertex[1])} ${r2(c.end[0])} ${r2(c.end[1])}`;
      }
    }
    return d;
  }
  function ellipseFill(cx, cy, rx, ry) {
    return arcs(cx, cy, rx, ry, () => 0);
  }
  function ellipseSketch(rand, cx, cy, rx, ry, amount) {
    return arcs(cx, cy, rx, ry, () => off(rand, amount));
  }
  function arcs(cx, cy, rx, ry, j) {
    const kappa = 0.5522847498;
    const ox = rx * kappa;
    const oy = ry * kappa;
    return `M ${r2(cx - rx + j())} ${r2(cy + j())} C ${r2(cx - rx + j())} ${r2(cy - oy + j())} ${r2(cx - ox + j())} ${r2(cy - ry + j())} ${r2(cx + j())} ${r2(cy - ry + j())} C ${r2(cx + ox + j())} ${r2(cy - ry + j())} ${r2(cx + rx + j())} ${r2(cy - oy + j())} ${r2(cx + rx + j())} ${r2(cy + j())} C ${r2(cx + rx + j())} ${r2(cy + oy + j())} ${r2(cx + ox + j())} ${r2(cy + ry + j())} ${r2(cx + j())} ${r2(cy + ry + j())} C ${r2(cx - ox + j())} ${r2(cy + ry + j())} ${r2(cx - rx + j())} ${r2(cy + oy + j())} ${r2(cx - rx + j())} ${r2(cy + j())} Z`;
  }
  function smooth(pts) {
    if (pts.length === 0) return "";
    if (pts.length === 1) return `M ${r2(pts[0][0])} ${r2(pts[0][1])}`;
    if (pts.length === 2) {
      return `M ${r2(pts[0][0])} ${r2(pts[0][1])} L ${r2(pts[1][0])} ${r2(pts[1][1])}`;
    }
    let d = `M ${r2(pts[0][0])} ${r2(pts[0][1])}`;
    for (let i = 1; i < pts.length - 1; i++) {
      const mx = (pts[i][0] + pts[i + 1][0]) / 2;
      const my = (pts[i][1] + pts[i + 1][1]) / 2;
      d += ` Q ${r2(pts[i][0])} ${r2(pts[i][1])} ${r2(mx)} ${r2(my)}`;
    }
    const last = pts[pts.length - 1];
    const beforeLast = pts[pts.length - 2];
    return d + ` Q ${r2(beforeLast[0])} ${r2(beforeLast[1])} ${r2(last[0])} ${r2(last[1])}`;
  }
  function polylineSketch(rand, pts, amount) {
    if (pts.length < 2) return "";
    let d = "";
    for (let i = 0; i < pts.length - 1; i++) {
      d += (d === "" ? "" : " ") + sketchEdge(rand, pts[i][0], pts[i][1], pts[i + 1][0], pts[i + 1][1], amount);
    }
    return d;
  }

  // src/render/attrs.ts
  var DEFAULT_STROKE = "#1e1e1e";
  function stroke(el) {
    const c = asString(el.strokeColor);
    return c === "" || c === "transparent" ? DEFAULT_STROKE : c;
  }
  function strokeWidth(el) {
    const w = typeof el.strokeWidth === "number" ? el.strokeWidth : 1;
    return w > 0 ? w : 1;
  }
  function dash(el) {
    const w = strokeWidth(el);
    if (el.strokeStyle === "dashed") return ` stroke-dasharray="${r2(w * 4)} ${r2(w * 4)}"`;
    if (el.strokeStyle === "dotted") return ` stroke-dasharray="${r2(w)} ${r2(w * 3)}"`;
    return "";
  }
  function strokeAttrs(el) {
    return ` fill="none" stroke="${escape(stroke(el))}" stroke-width="${r2(strokeWidth(el))}" stroke-linecap="round" stroke-linejoin="round"${dash(el)}`;
  }
  function opacity(el) {
    const o = typeof el.opacity === "number" ? el.opacity : 100;
    if (o >= 100) return "";
    return ` opacity="${r2(Math.max(0, o) / 100)}"`;
  }
  function rotation(el) {
    const a = typeof el.angle === "number" ? el.angle : 0;
    if (a === 0) return "";
    const cx = (el.x || 0) + (el.width || 0) / 2;
    const cy = (el.y || 0) + (el.height || 0) / 2;
    return ` transform="rotate(${r2(a * 180 / Math.PI)} ${r2(cx)} ${r2(cy)})"`;
  }
  function cornerRadius(el) {
    const roundness = el.roundness;
    if (!roundness) return 0;
    const shorter = Math.min(Math.abs(el.width || 0), Math.abs(el.height || 0));
    const proportional = 0.25;
    if (roundness.type === 3 || roundness.type === 1) return shorter * proportional;
    if (roundness.type === 2) {
      const fixed = typeof roundness.value === "number" ? roundness.value : 32;
      const cutoff = fixed / proportional;
      return shorter <= cutoff ? shorter * proportional : fixed;
    }
    return 0;
  }
  function box(el) {
    const x = el.x || 0;
    const y = el.y || 0;
    const w = el.width || 0;
    const h = el.height || 0;
    return [
      [x, y],
      [x + w, y],
      [x + w, y + h],
      [x, y + h]
    ];
  }
  function diamondPoints(el) {
    const x = el.x || 0;
    const y = el.y || 0;
    const w = el.width || 0;
    const h = el.height || 0;
    return [
      [x + w / 2, y],
      [x + w, y + h / 2],
      [x + w / 2, y + h],
      [x, y + h / 2]
    ];
  }
  function absolutePoints(el) {
    const MAX_POINTS = 5e3;
    const x = el.x || 0;
    const y = el.y || 0;
    const pts = [];
    const raw = Array.isArray(el.points) ? el.points : [];
    const count = Math.min(raw.length, MAX_POINTS);
    for (let i = 0; i < count; i++) {
      const p = raw[i];
      if (!Array.isArray(p) || p.length < 2) continue;
      const px = Number(p[0]);
      const py = Number(p[1]);
      if (!Number.isFinite(px) || !Number.isFinite(py)) continue;
      pts.push([x + px, y + py]);
    }
    return pts;
  }

  // src/render/fill.ts
  var NONE = "none";
  var GAP = 8;
  var Fills = class {
    seen = {};
    defs = [];
    /**
     * The `fill` value for one element.
     *
     * Registers a pattern on first use and returns `url(#id)`; returns the colour
     * itself for a solid fill, and `none` when there is nothing to fill.
     */
    paint(color, style, strokeWidth2) {
      const c = asString(color);
      if (c === "" || c === "transparent" || c === "none") return NONE;
      const kind = style === "solid" ? "solid" : style === "cross-hatch" ? "cross" : "hachure";
      if (kind === "solid") return c;
      const weight = Math.max(0.6, Math.round(strokeWidth2 * 60) / 100);
      const id = `f-${kind}-${slug(c)}-${String(weight).replace(".", "_")}`;
      if (!this.seen[id]) {
        this.seen[id] = true;
        this.defs.push(pattern(id, kind, c, weight));
      }
      return `url(#${id})`;
    }
    /** Every pattern registered so far, ready to drop inside `<defs>`. */
    toDefs() {
      return this.defs.join("");
    }
  };
  function pattern(id, kind, color, weight) {
    const lines = `<line x1="0" y1="0" x2="0" y2="${GAP}" stroke="${escape(color)}" stroke-width="${weight}"/>` + (kind === "cross" ? `<line x1="0" y1="0" x2="${GAP}" y2="0" stroke="${escape(color)}" stroke-width="${weight}"/>` : "");
    return `<pattern id="${id}" patternUnits="userSpaceOnUse" width="${GAP}" height="${GAP}" patternTransform="rotate(45)">${lines}</pattern>`;
  }
  function slug(color) {
    let out = "";
    for (let i = 0; i < color.length; i++) {
      const ch = color.charAt(i);
      out += /[A-Za-z0-9]/.test(ch) ? ch.toLowerCase() : "-";
    }
    return out;
  }

  // src/render/linear.ts
  function renderLinear(el, fills) {
    const pts = absolutePoints(el);
    if (pts.length < 2) return "";
    const rand = seeded(el.seed);
    const amount = jitter(el.roughness);
    const curved = !!el.roundness;
    let body = "";
    const closed = el.type === "line" && isClosed(pts);
    if (closed) {
      const fill = fills.paint(el.backgroundColor, el.fillStyle, strokeWidth(el));
      if (fill !== "none") {
        body += `<path d="${smooth(pts)} Z" fill="${escape(fill)}"/>`;
      }
    }
    const path = curved ? smooth(pts) : doubled(() => polylineSketch(rand, pts, amount), amount);
    body += `<path d="${path}"${strokeAttrs(el)}/>`;
    if (el.type === "arrow") {
      body += arrowhead(el, pts[1], pts[0], el.startArrowhead);
      body += arrowhead(el, pts[pts.length - 2], pts[pts.length - 1], el.endArrowhead);
    }
    return `<g${rotation(el)}${opacity(el)}>${body}</g>`;
  }
  function renderFreedraw(el) {
    const pts = absolutePoints(el);
    if (pts.length === 0) return "";
    const width = r2(strokeWidth(el) * 1.6);
    if (pts.length === 1) {
      return `<g${rotation(el)}${opacity(el)}><circle cx="${r2(pts[0][0])}" cy="${r2(pts[0][1])}" r="${r2(Math.max(0.8, strokeWidth(el)))}" fill="${escape(strokeOf(el))}"/></g>`;
    }
    return `<g${rotation(el)}${opacity(el)}><path d="${smooth(pts)}" fill="none" stroke="${escape(strokeOf(el))}" stroke-width="${width}" stroke-linecap="round" stroke-linejoin="round"/></g>`;
  }
  function strokeOf(el) {
    const c = asString(el.strokeColor);
    return c === "" || c === "transparent" ? "#1e1e1e" : c;
  }
  function isClosed(pts) {
    const first = pts[0];
    const last = pts[pts.length - 1];
    return Math.hypot(last[0] - first[0], last[1] - first[1]) < 1;
  }
  function arrowhead(el, from, tip, kind) {
    if (!kind || kind === "none" || !from || !tip) return "";
    const size = Math.min(24, 9 + strokeWidth(el) * 2.5);
    const angle = Math.atan2(tip[1] - from[1], tip[0] - from[0]);
    const spread = 0.45;
    const color = escape(strokeOf(el));
    const left = [
      tip[0] - size * Math.cos(angle - spread),
      tip[1] - size * Math.sin(angle - spread)
    ];
    const right = [
      tip[0] - size * Math.cos(angle + spread),
      tip[1] - size * Math.sin(angle + spread)
    ];
    switch (kind) {
      case "arrow":
        return `<path d="M ${r2(left[0])} ${r2(left[1])} L ${r2(tip[0])} ${r2(tip[1])} L ${r2(right[0])} ${r2(right[1])}" fill="none" stroke="${color}" stroke-width="${r2(strokeWidth(el))}" stroke-linecap="round" stroke-linejoin="round"/>`;
      case "triangle":
      case "triangle_outline":
        return `<path d="M ${r2(tip[0])} ${r2(tip[1])} L ${r2(left[0])} ${r2(left[1])} L ${r2(right[0])} ${r2(right[1])} Z"` + (kind === "triangle" ? ` fill="${color}"/>` : ` fill="none" stroke="${color}" stroke-width="${r2(strokeWidth(el))}"/>`);
      case "dot":
      case "circle":
      case "circle_outline":
        return `<circle cx="${r2(tip[0])}" cy="${r2(tip[1])}" r="${r2(size / 2.6)}"` + (kind === "circle_outline" ? ` fill="none" stroke="${color}" stroke-width="${r2(strokeWidth(el))}"/>` : ` fill="${color}"/>`);
      case "bar": {
        const bx = size * 0.6 * Math.cos(angle + Math.PI / 2);
        const by = size * 0.6 * Math.sin(angle + Math.PI / 2);
        return `<path d="M ${r2(tip[0] - bx)} ${r2(tip[1] - by)} L ${r2(tip[0] + bx)} ${r2(tip[1] + by)}" fill="none" stroke="${color}" stroke-width="${r2(strokeWidth(el))}" stroke-linecap="round"/>`;
      }
      default:
        return "";
    }
  }

  // src/render/shapes.ts
  function renderRectangle(el, fills) {
    return renderPolygon(el, fills, box(el), cornerRadius(el));
  }
  function renderDiamond(el, fills) {
    return renderPolygon(el, fills, diamondPoints(el), cornerRadius(el) / 2);
  }
  function renderPolygon(el, fills, pts, radius) {
    const rand = seeded(el.seed);
    const amount = jitter(el.roughness);
    const fill = fills.paint(el.backgroundColor, el.fillStyle, strokeWidth(el));
    let body = "";
    if (fill !== "none") {
      body += `<path d="${polygonFill(pts, radius)}" fill="${escape(fill)}"/>`;
    }
    body += `<path d="${doubled(() => polygonSketch(rand, pts, radius, amount), amount)}"${strokeAttrs(el)}/>`;
    return group(el, body);
  }
  function renderEllipse(el, fills) {
    const rand = seeded(el.seed);
    const amount = jitter(el.roughness);
    const cx = (el.x || 0) + (el.width || 0) / 2;
    const cy = (el.y || 0) + (el.height || 0) / 2;
    const rx = Math.abs(el.width || 0) / 2;
    const ry = Math.abs(el.height || 0) / 2;
    const fill = fills.paint(el.backgroundColor, el.fillStyle, strokeWidth(el));
    let body = "";
    if (fill !== "none") {
      body += `<path d="${ellipseFill(cx, cy, rx, ry)}" fill="${escape(fill)}"/>`;
    }
    body += `<path d="${doubled(() => ellipseSketch(rand, cx, cy, rx, ry, amount), amount)}"${strokeAttrs(el)}/>`;
    return group(el, body);
  }
  function renderImage(el, files) {
    const x = el.x || 0;
    const y = el.y || 0;
    const w = Math.abs(el.width || 0);
    const h = Math.abs(el.height || 0);
    const href = el.fileId ? files[el.fileId] : void 0;
    if (typeof href !== "string" || href === "") return group(el, placeholder(x, y, w, h));
    return group(
      el,
      `<image href="${escape(href)}" x="${r2(x)}" y="${r2(y)}" width="${r2(w)}" height="${r2(h)}" preserveAspectRatio="none"/>`
    );
  }
  function placeholder(x, y, w, h) {
    const label = w >= 90 && h >= 28 ? "image not cached" : "";
    const text = label === "" ? "" : `<text x="${r2(x + w / 2)}" y="${r2(y + h / 2 + 4)}" text-anchor="middle" font-family="ui-sans-serif, sans-serif" font-size="12" fill="#868e96">${label}</text>`;
    return `<rect x="${r2(x)}" y="${r2(y)}" width="${r2(w)}" height="${r2(h)}" rx="4" fill="rgba(134,142,150,.1)" stroke="#adb5bd" stroke-width="1" stroke-dasharray="4 4"/>` + text;
  }
  function renderFrame(el) {
    const x = el.x || 0;
    const y = el.y || 0;
    const w = Math.abs(el.width || 0);
    const h = Math.abs(el.height || 0);
    const name = asString(el.name);
    const label = name === "" ? "" : `<text x="${r2(x)}" y="${r2(y - 6)}" font-family="ui-sans-serif, sans-serif" font-size="12" fill="#868e96">${escape(name)}</text>`;
    return `<g${opacity(el)}><rect x="${r2(x)}" y="${r2(y)}" width="${r2(w)}" height="${r2(h)}" rx="6" fill="none" stroke="#bbb" stroke-width="1.5"/>${label}</g>`;
  }
  function group(el, body) {
    return `<g${rotation(el)}${opacity(el)}>${body}</g>`;
  }

  // src/render/text.ts
  var FONTS = {
    1: 'Excalifont, Virgil, "Segoe Print", "Bradley Hand", cursive',
    2: "Nunito, ui-sans-serif, system-ui, -apple-system, sans-serif",
    3: '"Comic Shanns", "Cascadia Code", ui-monospace, SFMono-Regular, monospace',
    5: 'Excalifont, Virgil, "Segoe Print", "Bradley Hand", cursive',
    6: "Nunito, ui-sans-serif, system-ui, -apple-system, sans-serif",
    7: '"Lilita One", ui-sans-serif, system-ui, sans-serif',
    8: '"Comic Shanns", ui-monospace, SFMono-Regular, monospace',
    9: '"Liberation Sans", ui-sans-serif, system-ui, sans-serif'
  };
  var DEFAULT_FONT = FONTS[1];
  function fontStack(id) {
    if (typeof id !== "number" || !Object.prototype.hasOwnProperty.call(FONTS, id)) {
      return DEFAULT_FONT;
    }
    return FONTS[id];
  }
  var BASELINE = 0.78;
  function renderText(el) {
    const raw = typeof el.text === "string" ? el.text : "";
    if (raw === "") return "";
    const fontSize = typeof el.fontSize === "number" && el.fontSize > 0 ? el.fontSize : 20;
    const lineHeight = typeof el.lineHeight === "number" && el.lineHeight > 0 ? el.lineHeight : 1.25;
    const lineBox = fontSize * lineHeight;
    const family = fontStack(el.fontFamily);
    const color = asString(el.strokeColor) || "#1e1e1e";
    const MAX_LINES = 2e3;
    const all = raw.split("\n");
    const lines = all.length > MAX_LINES ? all.slice(0, MAX_LINES) : all;
    const x = anchorX(el);
    const anchor = anchorName(el.textAlign);
    const top = (el.y || 0) + verticalOffset(el, lines.length, lineBox);
    let spans = "";
    for (let i = 0; i < lines.length; i++) {
      const baseline = top + i * lineBox + (lineBox - fontSize) / 2 + fontSize * BASELINE;
      const content = lines[i] === "" ? "\u200B" : escape(lines[i]);
      spans += `<tspan x="${r2(x)}" y="${r2(baseline)}">${content}</tspan>`;
    }
    return `<g${rotation(el)}${opacity(el)}><text font-family="${escape(family)}" font-size="${r2(fontSize)}" fill="${escape(color)}" text-anchor="${anchor}" style="white-space:pre">${spans}</text></g>`;
  }
  function anchorX(el) {
    const x = el.x || 0;
    const w = el.width || 0;
    if (el.textAlign === "center") return x + w / 2;
    if (el.textAlign === "right") return x + w;
    return x;
  }
  function anchorName(align) {
    if (align === "center") return "middle";
    if (align === "right") return "end";
    return "start";
  }
  function verticalOffset(el, lines, lineBox) {
    const height = el.height || 0;
    const total = lines * lineBox;
    if (el.verticalAlign === "middle") return Math.max(0, (height - total) / 2);
    if (el.verticalAlign === "bottom") return Math.max(0, height - total);
    return 0;
  }

  // src/render/scene.ts
  var PADDING = 16;
  var MAX_ELEMENTS = 1200;
  var SKIP = ["selection", "embeddable", "iframe", "magicframe"];
  function renderScene(elements, files, background, showBackground) {
    const live = [];
    let unsupported = 0;
    for (let i = 0; i < elements.length; i++) {
      const el = elements[i];
      if (!el || el.isDeleted) continue;
      if (SKIP.indexOf(el.type || "") !== -1 || !isDrawable(el)) {
        unsupported++;
        continue;
      }
      live.push(el);
    }
    const truncated = Math.max(0, live.length - MAX_ELEMENTS);
    const drawing = truncated > 0 ? live.slice(0, MAX_ELEMENTS) : live;
    const bounds = boundsOf(drawing);
    const fills = new Fills();
    let body = "";
    for (let i = 0; i < drawing.length; i++) {
      if (drawing[i].type === "frame") body += renderFrame(drawing[i]);
    }
    for (let i = 0; i < drawing.length; i++) {
      const el = drawing[i];
      if (el.type === "frame") continue;
      const drawn = renderElement(el, fills, files);
      if (drawn === "") unsupported++;
      body += drawn;
    }
    const width = Math.max(1, bounds.maxX - bounds.minX);
    const height = Math.max(1, bounds.maxY - bounds.minY);
    const backdrop = showBackground && isColor(background) ? `<rect x="${r2(bounds.minX)}" y="${r2(bounds.minY)}" width="${r2(width)}" height="${r2(height)}" fill="${escape(background)}"/>` : "";
    const defs = fills.toDefs();
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${r2(bounds.minX)} ${r2(bounds.minY)} ${r2(width)} ${r2(height)}" width="100%" height="100%" preserveAspectRatio="xMidYMid meet" role="img">` + (defs === "" ? "" : `<defs>${defs}</defs>`) + backdrop + body + `</svg>`;
    return { svg, drawn: drawing.length, truncated, unsupported, width, height };
  }
  function isDrawable(el) {
    const x = el.x || 0;
    const y = el.y || 0;
    const w = el.width || 0;
    const h = el.height || 0;
    const angle = el.angle || 0;
    return Number.isFinite(x + w) && Number.isFinite(y + h) && Number.isFinite(angle);
  }
  function renderElement(el, fills, files) {
    switch (el.type) {
      case "rectangle":
        return renderRectangle(el, fills);
      case "diamond":
        return renderDiamond(el, fills);
      case "ellipse":
        return renderEllipse(el, fills);
      case "line":
      case "arrow":
        return renderLinear(el, fills);
      case "freedraw":
        return renderFreedraw(el);
      case "text":
        return renderText(el);
      case "image":
        return renderImage(el, files);
      default:
        return "";
    }
  }
  function boundsOf(elements) {
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (let i = 0; i < elements.length; i++) {
      const corners2 = extentOf(elements[i]);
      for (let c = 0; c < corners2.length; c++) {
        const [x, y] = corners2[c];
        if (x < minX) minX = x;
        if (y < minY) minY = y;
        if (x > maxX) maxX = x;
        if (y > maxY) maxY = y;
      }
    }
    if (!Number.isFinite(minX) || !Number.isFinite(minY) || !Number.isFinite(maxX) || !Number.isFinite(maxY)) {
      return { minX: 0, minY: 0, maxX: 1, maxY: 1 };
    }
    return {
      minX: minX - PADDING,
      minY: minY - PADDING,
      maxX: maxX + PADDING,
      maxY: maxY + PADDING
    };
  }
  function extentOf(el) {
    const x = el.x || 0;
    const y = el.y || 0;
    const w = el.width || 0;
    const h = el.height || 0;
    const pts = Array.isArray(el.points) && el.points.length > 0 ? absolutePoints(el) : [
      [x, y],
      [x + w, y],
      [x + w, y + h],
      [x, y + h]
    ];
    const angle = typeof el.angle === "number" ? el.angle : 0;
    if (angle === 0) return pts;
    const cx = x + w / 2;
    const cy = y + h / 2;
    const cos = Math.cos(angle);
    const sin = Math.sin(angle);
    const out = [];
    for (let i = 0; i < pts.length; i++) {
      const dx = pts[i][0] - cx;
      const dy = pts[i][1] - cy;
      out.push([cx + dx * cos - dy * sin, cy + dx * sin + dy * cos]);
    }
    return out;
  }
  function isColor(value) {
    const v = asString(value);
    return v !== "" && v !== "transparent" && v !== "none";
  }

  // src/index.ts
  var DEFAULT_MAX_HEIGHT = 480;
  var SYNC_HINT = "outl plugin run app.outl.examples.excalidraw excalidraw-sync";
  var index_default = definePlugin({
    activate(ctx) {
      ctx.content.register("excalidraw", (body) => transform(ctx, body));
      ctx.commands.register("excalidraw-sync", () => syncScenes(ctx));
      ctx.commands.register("excalidraw-scenes", () => listScenes(ctx));
      ctx.commands.register("excalidraw-cache-clear", () => clearCache(ctx));
    }
  });
  function transform(ctx, body) {
    try {
      return render(ctx, body);
    } catch (e) {
      return rich(messagePage("Could not draw this scene", String(e)));
    }
  }
  function render(ctx, body) {
    const fence = parseFence(body);
    if (fence.kind === "error") {
      return rich(messagePage("Excalidraw fence", fence.message));
    }
    const config = ctx.config.get() || {};
    if (fence.kind === "inline") {
      const elements = Array.isArray(fence.scene.elements) ? fence.scene.elements : [];
      const background = fence.scene.appState && fence.scene.appState.viewBackgroundColor || "";
      return draw(
        elements,
        packFiles(fence.scene.files).files,
        background,
        fence.opts,
        config,
        ""
      );
    }
    const cached = readScene(ctx, fence.ref.id);
    if (cached.kind === "unavailable") {
      return rich(
        messagePage(
          "Cache unavailable",
          `Scenes are cached locally, and that cache could not be read: ${cached.reason}. A \`scene:\` fence needs the storage:local permission \u2014 a pasted .excalidraw file does not.`
        )
      );
    }
    if (cached.kind === "missing") {
      return rich(
        messagePage(
          "Not synced yet",
          `Scene ${fence.ref.id} is not in the local cache. Run \u201CExcalidraw: sync scenes\u201D from the command palette, or:`,
          SYNC_HINT
        )
      );
    }
    const scene = cached.scene;
    return draw(
      scene.elements,
      scene.files,
      scene.background,
      fence.opts,
      config,
      caption({
        name: fence.opts.title === void 0 ? scene.name : fence.opts.title,
        syncedAt: scene.at,
        filesDropped: scene.filesDropped
      })
    );
  }
  function draw(elements, files, background, opts, config, baseCaption) {
    if (elements.length === 0) {
      return rich(messagePage("Empty scene", "This scene has no elements to draw."));
    }
    const rendered = renderScene(elements, files, background, opts.background === true);
    const theme = opts.theme || config.theme || "auto";
    const notes = caption({
      truncated: rendered.truncated,
      unsupported: rendered.unsupported
    });
    const text = [baseCaption, notes].filter((s) => s !== "").join(" \xB7 ");
    return rich(
      drawingPage(rendered.svg, {
        dark: theme === "dark",
        auto: theme === "auto",
        maxHeight: clampHeight(config.maxHeight),
        height: opts.height,
        width: rendered.width,
        intrinsicHeight: rendered.height,
        caption: text
      })
    );
  }
  function clampHeight(value) {
    if (typeof value !== "number" || !Number.isFinite(value)) return DEFAULT_MAX_HEIGHT;
    return Math.min(2e3, Math.max(80, Math.round(value)));
  }
  function rich(content) {
    return { kind: "rich", content };
  }
})();
