/** Bech32m (BIP 350) decoding without the 90-character limit, as ZIP 316 requires. */

const CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
const BECH32M_CONST = 0x2bc830a3;
const GEN = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];

export class Bech32Error extends Error {
  override name = "Bech32Error";
}

function polymod(values: Iterable<number>): number {
  let chk = 1;
  for (const v of values) {
    const top = chk >>> 25;
    chk = ((chk & 0x1ffffff) << 5) ^ v;
    for (let i = 0; i < 5; i++) if ((top >>> i) & 1) chk ^= GEN[i]!;
  }
  return chk >>> 0;
}

function* hrpExpand(hrp: string): Generator<number> {
  for (let i = 0; i < hrp.length; i++) yield hrp.charCodeAt(i) >> 5;
  yield 0;
  for (let i = 0; i < hrp.length; i++) yield hrp.charCodeAt(i) & 31;
}

export interface Bech32Decoded {
  hrp: string;
  /** Data as 5-bit words, checksum removed. */
  words: number[];
}

/** Decodes a Bech32m string. Mixed case, bad characters and bad checksums are errors. */
export function bech32mDecode(input: string): Bech32Decoded {
  const lower = input.toLowerCase();
  if (input !== lower && input !== input.toUpperCase()) {
    throw new Bech32Error("mixed case");
  }
  const sep = lower.lastIndexOf("1");
  if (sep < 1) throw new Bech32Error("missing human-readable prefix");
  if (lower.length - sep - 1 < 6) throw new Bech32Error("too short");
  const hrp = lower.slice(0, sep);
  for (let i = 0; i < hrp.length; i++) {
    const c = hrp.charCodeAt(i);
    if (c < 33 || c > 126) throw new Bech32Error("invalid prefix character");
  }
  const words: number[] = [];
  for (const ch of lower.slice(sep + 1)) {
    const w = CHARSET.indexOf(ch);
    if (w < 0) throw new Bech32Error(`invalid character "${ch}"`);
    words.push(w);
  }
  const check = polymod((function* () {
    yield* hrpExpand(hrp);
    yield* words;
  })());
  if (check !== BECH32M_CONST) throw new Bech32Error("bad checksum");
  return { hrp, words: words.slice(0, -6) };
}

/** Regroups 5-bit words into bytes. Leftover padding must be under 5 bits and zero. */
export function wordsToBytes(words: readonly number[]): Uint8Array {
  const out: number[] = [];
  let acc = 0;
  let bits = 0;
  for (const w of words) {
    acc = ((acc << 5) | w) & 0xfff;
    bits += 5;
    if (bits >= 8) {
      bits -= 8;
      out.push((acc >>> bits) & 0xff);
    }
  }
  if (bits >= 5 || (acc & ((1 << bits) - 1)) !== 0) {
    throw new Bech32Error("invalid padding");
  }
  return Uint8Array.from(out);
}

/** Regroups bytes into 5-bit words, zero-padding the last one. */
export function bytesToWords(bytes: Uint8Array): number[] {
  const out: number[] = [];
  let acc = 0;
  let bits = 0;
  for (const b of bytes) {
    acc = ((acc << 8) | b) & 0x1fff;
    bits += 8;
    while (bits >= 5) {
      bits -= 5;
      out.push((acc >>> bits) & 31);
    }
  }
  if (bits > 0) out.push((acc << (5 - bits)) & 31);
  return out;
}

/** Encodes 5-bit words as a lowercase Bech32m string. */
export function bech32mEncode(hrp: string, words: readonly number[]): string {
  const values = [...hrpExpand(hrp), ...words, 0, 0, 0, 0, 0, 0];
  const mod = polymod(values) ^ BECH32M_CONST;
  const checksum = Array.from({ length: 6 }, (_, i) => (mod >>> (5 * (5 - i))) & 31);
  return hrp + "1" + [...words, ...checksum].map((w) => CHARSET[w]).join("");
}
