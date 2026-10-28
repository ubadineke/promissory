/**
 * Unified address decoding (ZIP 316, revision 0): Bech32m, then F4Jumble, then the
 * HRP padding and the typecode/length/value items.
 *
 * This is a full parse, but the payer app (`promissory-app`) remains the authoritative check:
 * it is what actually builds transactions to the address.
 */

import { Bech32Error, bech32mDecode, wordsToBytes } from "./bech32m.js";
import { f4jumbleInv } from "./f4jumble.js";

/** Networks Promissory runs on. */
export type Network = "main" | "regtest";

/** Every network a unified address can be encoded for. */
export type AddressNetwork = Network | "test";

export const TYPECODE_P2PKH = 0x00;
export const TYPECODE_P2SH = 0x01;
export const TYPECODE_SAPLING = 0x02;
/** Orchard; Ironwood shares this receiver since NU6.3. */
export const TYPECODE_ORCHARD = 0x03;

const HRP_NETWORK: Record<string, AddressNetwork> = {
  u: "main",
  utest: "test",
  uregtest: "regtest",
};

const KNOWN_LENGTHS: Record<number, number> = {
  [TYPECODE_P2PKH]: 20,
  [TYPECODE_P2SH]: 20,
  [TYPECODE_SAPLING]: 43,
  [TYPECODE_ORCHARD]: 43,
};

const PADDING_LEN = 16;

export class AddressError extends Error {
  override name = "AddressError";
}

export interface Receiver {
  typecode: number;
  data: Uint8Array;
}

export interface UnifiedAddress {
  network: AddressNetwork;
  receivers: Receiver[];
}

/** Reads a Bitcoin-style CompactSize integer, rejecting non-canonical encodings. */
function readCompactSize(buf: Uint8Array, pos: number): [value: number, next: number] {
  const first = buf[pos];
  if (first === undefined) throw new AddressError("truncated item");
  if (first < 0xfd) return [first, pos + 1];
  const width = first === 0xfd ? 2 : first === 0xfe ? 4 : 8;
  if (width === 8 || pos + 1 + width > buf.length) {
    throw new AddressError("invalid item length");
  }
  let value = 0;
  for (let i = width - 1; i >= 0; i--) value = value * 256 + buf[pos + 1 + i]!;
  const min = width === 2 ? 0xfd : 0x10000;
  if (value < min) throw new AddressError("non-canonical item length");
  return [value, pos + 1 + width];
}

/** Decodes a unified address into its network and receivers. Throws `AddressError`. */
export function decodeUnifiedAddress(address: string): UnifiedAddress {
  let hrp: string;
  let bytes: Uint8Array;
  try {
    const decoded = bech32mDecode(address.trim());
    hrp = decoded.hrp;
    bytes = wordsToBytes(decoded.words);
  } catch (e) {
    if (e instanceof Bech32Error) throw new AddressError(`not a unified address (${e.message})`);
    throw e;
  }
  const network = HRP_NETWORK[hrp];
  if (!network) throw new AddressError(`unknown address prefix "${hrp}"`);
  if (bytes.length < 48) throw new AddressError("too short for a unified address");

  let raw: Uint8Array;
  try {
    raw = f4jumbleInv(bytes);
  } catch {
    throw new AddressError("invalid unified address length");
  }
  const body = raw.subarray(0, raw.length - PADDING_LEN);
  const padding = raw.subarray(raw.length - PADDING_LEN);
  for (let i = 0; i < PADDING_LEN; i++) {
    if (padding[i] !== (i < hrp.length ? hrp.charCodeAt(i) : 0)) {
      throw new AddressError("invalid unified address (bad padding)");
    }
  }

  const receivers: Receiver[] = [];
  let pos = 0;
  while (pos < body.length) {
    const [typecode, afterType] = readCompactSize(body, pos);
    const [len, afterLen] = readCompactSize(body, afterType);
    if (typecode > 0x02000000) throw new AddressError(`typecode ${typecode} out of range`);
    if (afterLen + len > body.length) throw new AddressError("truncated item");
    const expected = KNOWN_LENGTHS[typecode];
    if (expected !== undefined && len !== expected) {
      throw new AddressError(`receiver ${typecode} has length ${len}, expected ${expected}`);
    }
    if (receivers.some((r) => r.typecode === typecode)) {
      throw new AddressError(`duplicate receiver ${typecode}`);
    }
    receivers.push({ typecode, data: body.slice(afterLen, afterLen + len) });
    pos = afterLen + len;
  }
  if (receivers.length === 0) throw new AddressError("unified address has no receivers");
  const types = new Set(receivers.map((r) => r.typecode));
  if (types.has(TYPECODE_P2PKH) && types.has(TYPECODE_P2SH)) {
    throw new AddressError("unified address has both P2PKH and P2SH receivers");
  }
  if ([...types].every((t) => t === TYPECODE_P2PKH || t === TYPECODE_P2SH)) {
    throw new AddressError("unified address has only transparent receivers");
  }
  return { network, receivers };
}

/**
 * Checks that `address` is a unified address for `network` with an Orchard receiver,
 * the only kind of payee Promissory accepts. Throws `AddressError`.
 */
export function checkPayeeAddress(address: string, network: Network): void {
  const ua = decodeUnifiedAddress(address);
  if (ua.network !== network) {
    throw new AddressError(`address is for ${ua.network}, not ${network}`);
  }
  if (!ua.receivers.some((r) => r.typecode === TYPECODE_ORCHARD)) {
    throw new AddressError("address has no Orchard receiver");
  }
}

/** `true` if `address` passes `checkPayeeAddress`. */
export function isValidPayeeAddress(address: string, network: Network): boolean {
  try {
    checkPayeeAddress(address, network);
    return true;
  } catch {
    return false;
  }
}
