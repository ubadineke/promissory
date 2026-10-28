/** F4Jumble (ZIP 316), the unkeyed permutation that wraps a unified address encoding. */

import { blake2b } from "./blake2b.js";

export const F4JUMBLE_MIN = 38;
export const F4JUMBLE_MAX = 4194368;

const PREFIX = [85, 65, 95, 70, 52, 74, 117, 109, 98, 108, 101, 95]; // "UA_F4Jumble_"

function hPers(i: number): Uint8Array {
  return Uint8Array.from([...PREFIX, 72, i, 0, 0]); // "H"
}

function gPers(i: number, j: number): Uint8Array {
  return Uint8Array.from([...PREFIX, 71, i, j & 0xff, j >>> 8]); // "G"
}

function hRound(i: number, u: Uint8Array, len: number): Uint8Array {
  return blake2b(u, len, hPers(i));
}

function gRound(i: number, u: Uint8Array, len: number): Uint8Array {
  const out = new Uint8Array(len);
  for (let j = 0; j * 64 < len; j++) {
    const block = blake2b(u, 64, gPers(i, j));
    out.set(block.subarray(0, Math.min(64, len - j * 64)), j * 64);
  }
  return out;
}

function xorInto(target: Uint8Array, source: Uint8Array): void {
  for (let i = 0; i < target.length; i++) target[i] = target[i]! ^ source[i]!;
}

function split(msg: Uint8Array): [Uint8Array, Uint8Array] {
  if (msg.length < F4JUMBLE_MIN || msg.length > F4JUMBLE_MAX) {
    throw new RangeError(`F4Jumble input must be ${F4JUMBLE_MIN}..${F4JUMBLE_MAX} bytes`);
  }
  const lL = Math.min(64, Math.floor(msg.length / 2));
  return [msg.slice(0, lL), msg.slice(lL)];
}

export function f4jumble(msg: Uint8Array): Uint8Array {
  const [a, b] = split(msg);
  xorInto(b, gRound(0, a, b.length));
  xorInto(a, hRound(0, b, a.length));
  xorInto(b, gRound(1, a, b.length));
  xorInto(a, hRound(1, b, a.length));
  return Uint8Array.from([...a, ...b]);
}

export function f4jumbleInv(msg: Uint8Array): Uint8Array {
  const [c, d] = split(msg);
  xorInto(c, hRound(1, d, c.length));
  xorInto(d, gRound(1, c, d.length));
  xorInto(c, hRound(0, d, c.length));
  xorInto(d, gRound(0, c, d.length));
  return Uint8Array.from([...c, ...d]);
}
