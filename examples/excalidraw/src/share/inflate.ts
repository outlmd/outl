/**
 * DEFLATE decompression (RFC 1951), with the zlib wrapper (RFC 1950) peeled
 * when present.
 *
 * Excalidraw compresses a share payload with `pako.deflate`, which emits the
 * zlib framing. Boa has no `DecompressionStream` and the plugin ships as one
 * bundle, so this is the decoder. It follows the structure of zlib's own
 * `puff.c` reference implementation — the canonical-Huffman decode below is
 * that algorithm, which is worth knowing before changing it.
 */

/**
 * A growable output buffer. DEFLATE does not announce its output size.
 *
 * `limit` is not optional in practice: a compressed stream says nothing about
 * how much it expands to, and DEFLATE reaches about 1029:1 on degenerate
 * input. Without a ceiling here, a 32 KB payload that passed the input check
 * can still ask for 32 MB — which on this interpreter is minutes of inflating
 * and a buffer the mobile client gets killed for.
 */
class Output {
  private buf: Uint8Array;
  private len = 0;

  constructor(private readonly limit: number) {
    // Never start above the limit: `push` only consults `reserve` once the
    // buffer is full, so a 1 KB initial buffer would let a smaller limit be
    // overrun before anything checked it.
    this.buf = new Uint8Array(Math.min(1024, limit));
  }

  push(byte: number): void {
    if (this.len === this.buf.length) this.reserve(1);
    this.buf[this.len++] = byte;
  }

  /** Copy `length` bytes from `distance` back — the LZ77 half of DEFLATE. */
  copyBack(distance: number, length: number): void {
    if (distance > this.len) throw new Error("distance runs before the output");
    this.reserve(length);
    let from = this.len - distance;
    for (let i = 0; i < length; i++) this.buf[this.len++] = this.buf[from++];
  }

  private reserve(extra: number): void {
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

  take(): Uint8Array {
    return this.buf.subarray(0, this.len);
  }
}

/**
 * Reads the bit stream LSB-first, as DEFLATE specifies.
 *
 * Bits are pulled into a 32-bit accumulator rather than read one at a time
 * from the array. A Huffman symbol is up to 15 bits, and a call per bit is
 * what made this the second-slowest phase of a share decode under Boa.
 * `acc` and `count` are read directly by `decodeSymbol`, which walks them in
 * locals; that is why they are not private.
 */
class Bits {
  /** Buffered bits, LSB-first. */
  acc = 0;
  /** How many bits in `acc` are valid. */
  count = 0;
  private pos = 0;

  constructor(private readonly data: Uint8Array) {}

  /** Buffer at least `n` bits, or as many as the stream has left. */
  fill(n: number): void {
    while (this.count < n && this.pos < this.data.length) {
      this.acc |= this.data[this.pos++] << this.count;
      this.count += 8;
    }
  }

  one(): number {
    if (this.count === 0) {
      this.fill(1);
      if (this.count === 0) throw new Error("stream ended mid-symbol");
    }
    const value = this.acc & 1;
    this.acc >>>= 1;
    this.count--;
    return value;
  }

  take(count: number): number {
    if (count === 0) return 0;
    this.fill(count);
    if (this.count < count) throw new Error("stream ended mid-field");
    const value = this.acc & ((1 << count) - 1);
    this.acc >>>= count;
    this.count -= count;
    return value;
  }

  /** Drop the rest of the current byte — what a stored block starts with. */
  align(): void {
    const extra = this.count & 7;
    this.acc >>>= extra;
    this.count -= extra;
  }

  readByte(): number {
    if (this.count >= 8) {
      const value = this.acc & 0xff;
      this.acc >>>= 8;
      this.count -= 8;
      return value;
    }
    if (this.pos >= this.data.length) throw new Error("stream ended");
    return this.data[this.pos++];
  }
}

/** A canonical Huffman table: how many codes per length, and the symbols. */
interface Huffman {
  counts: Int32Array;
  symbols: Int32Array;
}

/** Build the table from each symbol's code length (0 = unused). */
function huffman(lengths: Int32Array | number[], count: number): Huffman {
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

/**
 * Decode one symbol, walking code lengths shortest-first.
 *
 * Reads straight out of the accumulator in locals and writes it back once.
 * A symbol is at most 15 bits, so one `fill` covers it except at the very
 * end of the stream, where `count` runs out and the loop says so.
 */
function decodeSymbol(bits: Bits, table: Huffman): number {
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
    first = (first + available) << 1;
    code <<= 1;
  }
  throw new Error("invalid Huffman code");
}

// RFC 1951 §3.2.5 — the length and distance code tables.
const LENGTH_BASE = [
  3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
  163, 195, 227, 258,
];
const LENGTH_EXTRA = [
  0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE = [
  1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049,
  3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA = [
  0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13,
];
/** The order code lengths are themselves written in (RFC 1951 §3.2.7). */
const CODE_LENGTH_ORDER = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/** Emit one block's symbols until the end-of-block code. */
function inflateBlock(bits: Bits, out: Output, lit: Huffman, dist: Huffman): void {
  for (;;) {
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

/** The fixed tables of RFC 1951 §3.2.6, built once. */
function fixedTables(): { lit: Huffman; dist: Huffman } {
  const litLengths = new Int32Array(288);
  for (let i = 0; i < 144; i++) litLengths[i] = 8;
  for (let i = 144; i < 256; i++) litLengths[i] = 9;
  for (let i = 256; i < 280; i++) litLengths[i] = 7;
  for (let i = 280; i < 288; i++) litLengths[i] = 8;

  const distLengths = new Int32Array(30);
  for (let i = 0; i < 30; i++) distLengths[i] = 5;

  return { lit: huffman(litLengths, 288), dist: huffman(distLengths, 30) };
}

/** Read a dynamic block's two tables (RFC 1951 §3.2.7). */
function dynamicTables(bits: Bits): { lit: Huffman; dist: Huffman } {
  const litCount = bits.take(5) + 257;
  const distCount = bits.take(5) + 1;
  const codeCount = bits.take(4) + 4;

  const codeLengths = new Int32Array(19);
  for (let i = 0; i < codeCount; i++) {
    codeLengths[CODE_LENGTH_ORDER[i]] = bits.take(3);
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
    dist: huffman(lengths.subarray(litCount), distCount),
  };
}

/**
 * Inflate a DEFLATE stream, with or without the zlib wrapper.
 *
 * Excalidraw's `pako.deflate` writes the wrapper; a raw stream is accepted
 * too, so this does not break if that changes. The trailing Adler-32
 * checksum is not verified — the same trade the GCM tag takes in `aes.ts`,
 * and a corrupt stream fails in the decoder long before it reaches here.
 *
 * `limitBytes` bounds the **output**, which is the only bound that matters
 * against a compression bomb: the input size says nothing about it.
 */
export function inflate(input: Uint8Array, limitBytes: number): Uint8Array {
  let data = input;

  // zlib header: low nibble 8 (deflate) and the two bytes a multiple of 31.
  if (
    data.length > 2 &&
    (data[0] & 0x0f) === 8 &&
    ((data[0] << 8) | data[1]) % 31 === 0 &&
    (data[1] & 0x20) === 0 // no preset dictionary
  ) {
    data = data.subarray(2);
  }

  const bits = new Bits(data);
  const out = new Output(limitBytes);
  const fixed = fixedTables();

  for (;;) {
    const final = bits.one();
    const type = bits.take(2);

    if (type === 0) {
      bits.align();
      const len = bits.readByte() | (bits.readByte() << 8);
      bits.readByte();
      bits.readByte(); // ~len, not checked
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
