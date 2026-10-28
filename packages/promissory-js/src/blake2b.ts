/**
 * BLAKE2b (RFC 7693) with a personalization string, as F4Jumble needs it.
 * 64-bit words are held as little-endian pairs of 32-bit halves.
 */

const IV = new Uint32Array([
  0xf3bcc908, 0x6a09e667, 0x84caa73b, 0xbb67ae85, 0xfe94f82b, 0x3c6ef372, 0x5f1d36f1, 0xa54ff53a,
  0xade682d1, 0x510e527f, 0x2b3e6c1f, 0x9b05688c, 0xfb41bd6b, 0x1f83d9ab, 0x137e2179, 0x5be0cd19,
]);

// prettier-ignore
const SIGMA = [
  0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
  14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3,
  11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4,
  7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8,
  9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13,
  2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9,
  12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11,
  13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10,
  6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5,
  10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0,
  0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
  14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3,
].map((x) => x * 2);

const v = new Uint32Array(32);
const m = new Uint32Array(32);

/** v[a] += v[b] (64-bit). */
function add64(a: number, b: number): void {
  const lo = v[a]! + v[b]!;
  let hi = v[a + 1]! + v[b + 1]!;
  if (lo >= 0x100000000) hi++;
  v[a] = lo;
  v[a + 1] = hi;
}

/** v[a] += (hi:lo) (64-bit). */
function add64c(a: number, lo0: number, hi0: number): void {
  const lo = v[a]! + lo0;
  let hi = v[a + 1]! + hi0;
  if (lo >= 0x100000000) hi++;
  v[a] = lo;
  v[a + 1] = hi;
}

function g(a: number, b: number, c: number, d: number, ix: number, iy: number): void {
  add64(a, b);
  add64c(a, m[ix]!, m[ix + 1]!);
  // d = rotr64(d ^ a, 32)
  let x0 = v[d]! ^ v[a]!;
  let x1 = v[d + 1]! ^ v[a + 1]!;
  v[d] = x1;
  v[d + 1] = x0;
  add64(c, d);
  // b = rotr64(b ^ c, 24)
  x0 = v[b]! ^ v[c]!;
  x1 = v[b + 1]! ^ v[c + 1]!;
  v[b] = (x0 >>> 24) ^ (x1 << 8);
  v[b + 1] = (x1 >>> 24) ^ (x0 << 8);
  add64(a, b);
  add64c(a, m[iy]!, m[iy + 1]!);
  // d = rotr64(d ^ a, 16)
  x0 = v[d]! ^ v[a]!;
  x1 = v[d + 1]! ^ v[a + 1]!;
  v[d] = (x0 >>> 16) ^ (x1 << 16);
  v[d + 1] = (x1 >>> 16) ^ (x0 << 16);
  add64(c, d);
  // b = rotr64(b ^ c, 63)
  x0 = v[b]! ^ v[c]!;
  x1 = v[b + 1]! ^ v[c + 1]!;
  v[b] = (x1 >>> 31) ^ (x0 << 1);
  v[b + 1] = (x0 >>> 31) ^ (x1 << 1);
}

function compress(h: Uint32Array, block: Uint8Array, t: number, last: boolean): void {
  for (let i = 0; i < 16; i++) {
    v[i] = h[i]!;
    v[i + 16] = IV[i]!;
  }
  v[24] = v[24]! ^ t;
  v[25] = v[25]! ^ Math.floor(t / 0x100000000);
  if (last) {
    v[28] = ~v[28]!;
    v[29] = ~v[29]!;
  }
  for (let i = 0; i < 32; i++) {
    const o = 4 * i;
    m[i] = block[o]! | (block[o + 1]! << 8) | (block[o + 2]! << 16) | (block[o + 3]! << 24);
  }
  for (let r = 0; r < 12; r++) {
    const s = r * 16;
    g(0, 8, 16, 24, SIGMA[s]!, SIGMA[s + 1]!);
    g(2, 10, 18, 26, SIGMA[s + 2]!, SIGMA[s + 3]!);
    g(4, 12, 20, 28, SIGMA[s + 4]!, SIGMA[s + 5]!);
    g(6, 14, 22, 30, SIGMA[s + 6]!, SIGMA[s + 7]!);
    g(0, 10, 20, 30, SIGMA[s + 8]!, SIGMA[s + 9]!);
    g(2, 12, 22, 24, SIGMA[s + 10]!, SIGMA[s + 11]!);
    g(4, 14, 16, 26, SIGMA[s + 12]!, SIGMA[s + 13]!);
    g(6, 8, 18, 28, SIGMA[s + 14]!, SIGMA[s + 15]!);
  }
  for (let i = 0; i < 16; i++) h[i] = h[i]! ^ v[i]! ^ v[i + 16]!;
}

/**
 * Unkeyed BLAKE2b of `input` with an output of `outLen` bytes (1..64) and an optional
 * 16-byte personalization.
 */
export function blake2b(input: Uint8Array, outLen = 64, personal?: Uint8Array): Uint8Array {
  if (!Number.isInteger(outLen) || outLen < 1 || outLen > 64) {
    throw new RangeError("blake2b output length must be 1..64 bytes");
  }
  if (personal && personal.length !== 16) {
    throw new RangeError("blake2b personalization must be 16 bytes");
  }
  const h = new Uint32Array(IV);
  h[0] = h[0]! ^ 0x01010000 ^ outLen;
  if (personal) {
    for (let i = 0; i < 4; i++) {
      const o = 4 * i;
      h[12 + i] =
        h[12 + i]! ^
        (personal[o]! | (personal[o + 1]! << 8) | (personal[o + 2]! << 16) | (personal[o + 3]! << 24));
    }
  }

  const block = new Uint8Array(128);
  let t = 0;
  let off = 0;
  // Every full block except the last is compressed as non-final.
  while (input.length - off > 128) {
    block.set(input.subarray(off, off + 128));
    t += 128;
    compress(h, block, t, false);
    off += 128;
  }
  block.fill(0);
  block.set(input.subarray(off));
  t += input.length - off;
  compress(h, block, t, true);

  const out = new Uint8Array(outLen);
  for (let i = 0; i < outLen; i++) out[i] = h[i >> 2]! >>> (8 * (i & 3));
  return out;
}
