/**
 * AES-128 in counter mode, enough to decrypt an Excalidraw share payload.
 *
 * **Why hand-rolled:** the plugin runtime is Boa, a pure ECMAScript engine
 * with no WebCrypto, and the payload is AES-GCM. There is no `crypto.subtle`
 * to call.
 *
 * **Why CTR for a GCM cipher text:** GCM *is* CTR for confidentiality — the
 * keystream is `AES(J0 + n)` with `J0 = IV ‖ 0⁰³¹1` for a 12-byte IV, so the
 * first plaintext block uses counter 2. The part this skips is GHASH, which
 * authenticates. See `verifyTag` below for what that costs and why it is the
 * right trade here.
 */

/** AES block size, and the IV length Excalidraw uses (`IV_LENGTH_BYTES`). */
const BLOCK = 16;
const IV_LEN = 12;

/** GCM's authentication tag, carried in the last 16 bytes of the cipher text. */
const TAG_LEN = 16;

/**
 * The AES S-box, computed rather than written out.
 *
 * 256 magic literals in a diff are unreviewable; this is the standard
 * construction (multiplicative inverse in GF(2⁸), then the affine transform),
 * which a reader can check against the spec in a way a table cannot be.
 */
const SBOX = buildSbox();

function buildSbox(): Uint8Array {
  const sbox = new Uint8Array(256);
  sbox[0] = 0x63;

  let p = 1;
  let q = 1;
  do {
    // p *= 3 in GF(2^8)
    p = (p ^ (p << 1) ^ (p & 0x80 ? 0x1b : 0)) & 0xff;
    // q /= 3 in GF(2^8)
    q ^= q << 1;
    q ^= q << 2;
    q ^= q << 4;
    q &= 0xff;
    if (q & 0x80) q ^= 0x09;

    sbox[p] = (q ^ rotl(q, 1) ^ rotl(q, 2) ^ rotl(q, 3) ^ rotl(q, 4) ^ 0x63) & 0xff;
  } while (p !== 1);

  return sbox;
}

/** Rotate a byte left. */
function rotl(byte: number, by: number): number {
  return ((byte << by) | (byte >> (8 - by))) & 0xff;
}

/** Multiply by x in GF(2⁸) — the one primitive MixColumns needs. */
function xtime(byte: number): number {
  return ((byte << 1) ^ (byte & 0x80 ? 0x1b : 0)) & 0xff;
}

/**
 * `xtime` as a table.
 *
 * MixColumns calls it sixteen times per round. On an interpreter a call is
 * far dearer than an indexed read, and 256 bytes buys all of them.
 */
const XTIME = buildXtime();

function buildXtime(): Uint8Array {
  const table = new Uint8Array(256);
  for (let i = 0; i < 256; i++) table[i] = xtime(i);
  return table;
}

/**
 * Expand a 16-byte key into the 11 round keys AES-128 uses.
 *
 * Returned flat: round `r`'s key is bytes `[r * 16, r * 16 + 16)`.
 */
function expandKey(key: Uint8Array): Uint8Array {
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
      // RotWord + SubWord + Rcon
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

/**
 * Encrypt one block, writing the result into `out`.
 *
 * The state lives in sixteen locals rather than a `Uint8Array`, and the
 * rounds are inlined. That is not premature: measured under Boa, the array
 * version spent 400ms on 2.3 KB because every one of the ~880 element reads
 * and writes per block goes through a generic indexed-property path, and
 * each round called four functions on top. Locals and two lookup tables are
 * what make a share link decode in a reasonable time on the TUI.
 */
function encryptBlock(input: Uint8Array, rk: Uint8Array, out: Uint8Array): void {
  const ROUNDS = 10;

  // Round 0 is just the key addition.
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
    // SubBytes
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

    // ShiftRows. The state is column-major, so row r is s[r], s[r+4],
    // s[r+8], s[r+12], and row r rotates left by r.
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

    // MixColumns, skipped in the final round.
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

    // AddRoundKey
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

/**
 * Decrypt an AES-GCM cipher text with a 12-byte IV, **without** verifying the
 * authentication tag.
 *
 * The trade, stated rather than hidden: GCM's tag proves the bytes were not
 * tampered with, and GHASH is a second primitive's worth of code to check it.
 * What it protects here is a drawing the user chose to open, fetched over TLS
 * from the host their own link names, and rendered into inert SVG — never
 * executed, never written to the op log. A forged payload yields garbage
 * elements, which the renderer already treats as unsupported. The tag bytes
 * are dropped rather than ignored, so the plaintext length is right.
 */
export function decryptGcm(key: Uint8Array, iv: Uint8Array, cipherText: Uint8Array): Uint8Array {
  if (iv.length !== IV_LEN) throw new Error(`expected a ${IV_LEN}-byte IV, got ${iv.length}`);
  if (cipherText.length < TAG_LEN) throw new Error("cipher text is shorter than its tag");

  const roundKeys = expandKey(key);
  const body = cipherText.subarray(0, cipherText.length - TAG_LEN);
  const out = new Uint8Array(body.length);

  const counter = new Uint8Array(BLOCK);
  counter.set(iv, 0);

  const keystream = new Uint8Array(BLOCK);

  // J0 is IV‖0⁰³¹1 for a 12-byte IV, and J0 is spent on the tag — so the
  // first block of plaintext is encrypted under counter 2.
  let block = 2;
  for (let offset = 0; offset < body.length; offset += BLOCK) {
    counter[12] = (block >>> 24) & 0xff;
    counter[13] = (block >>> 16) & 0xff;
    counter[14] = (block >>> 8) & 0xff;
    counter[15] = block & 0xff;
    block++;

    encryptBlock(counter, roundKeys, keystream);

    const end = Math.min(BLOCK, body.length - offset);
    for (let i = 0; i < end; i++) out[offset + i] = body[offset + i] ^ keystream[i];
  }

  return out;
}
