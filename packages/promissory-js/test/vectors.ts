import { bech32mEncode, bytesToWords } from "../src/bech32m.js";
import { f4jumble } from "../src/f4jumble.js";

/** Real regtest addresses from our chain, each with an Orchard receiver. */
export const REGTEST_UA_1 =
  "uregtest13543hcxsg0xcd7he0nsux3kmz2rqdg905n69x86mlyth67pesmxjpkfg6mwesu9chlpggf6clg58n7zth7uhd4ud9n655l8hkykuwwxj";
export const REGTEST_UA_2 =
  "uregtest1ynahn0vg27j478s8azkuzthxhfagywduxzgx3ymx5armlml42rtxeeamhlywpj5p4fp469a5zck46tc7rfd79rd0nmps3lhauugm2y9q";
/** Orchard-only mainnet UA. */
export const MAIN_UA =
  "u10gsry8xrghdry83eg87jfjepxfft8jefk5wg6yjkwavzk9x8ksvhz0xagf7tk33r840qye5zqvpup0xhten3de9wj6pje3cmru93g4mv";

/** Encodes a synthetic unified address from raw items, for negative tests. */
export function encodeUa(hrp: string, items: { typecode: number; len?: number; data?: Uint8Array }[]): string {
  const bytes: number[] = [];
  for (const it of items) {
    const data = it.data ?? new Uint8Array(it.len ?? 43).fill(7);
    bytes.push(it.typecode, data.length, ...data);
  }
  for (let i = 0; i < 16; i++) bytes.push(i < hrp.length ? hrp.charCodeAt(i) : 0);
  return bech32mEncode(hrp, bytesToWords(f4jumble(Uint8Array.from(bytes))));
}

export const hex = (b: Uint8Array) => Buffer.from(b).toString("hex");
export const unhex = (s: string) => Uint8Array.from(Buffer.from(s, "hex"));
