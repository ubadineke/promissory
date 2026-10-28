import { describe, expect, it } from "vitest";
import {
  AddressError,
  checkPayeeAddress,
  decodeUnifiedAddress,
  isValidPayeeAddress,
  TYPECODE_ORCHARD,
} from "../src/address.js";
import { bech32mEncode, bytesToWords } from "../src/bech32m.js";
import { encodeUa, MAIN_UA, REGTEST_UA_1, REGTEST_UA_2 } from "./vectors.js";

describe("decodeUnifiedAddress", () => {
  it("decodes the real regtest addresses", () => {
    for (const ua of [REGTEST_UA_1, REGTEST_UA_2]) {
      const d = decodeUnifiedAddress(ua);
      expect(d.network).toBe("regtest");
      expect(d.receivers.map((r) => [r.typecode, r.data.length])).toEqual([[TYPECODE_ORCHARD, 43]]);
    }
  });

  it("decodes the mainnet Orchard-only address", () => {
    const d = decodeUnifiedAddress(MAIN_UA);
    expect(d.network).toBe("main");
    expect(d.receivers.map((r) => r.typecode)).toEqual([TYPECODE_ORCHARD]);
  });

  it("tolerates surrounding whitespace and upper case", () => {
    expect(decodeUnifiedAddress(`  ${MAIN_UA}\n`).network).toBe("main");
    expect(decodeUnifiedAddress(MAIN_UA.toUpperCase()).network).toBe("main");
  });

  it("decodes multi-receiver synthetic addresses", () => {
    const ua = encodeUa("u", [{ typecode: 0, len: 20 }, { typecode: 2 }, { typecode: 3 }]);
    expect(decodeUnifiedAddress(ua).receivers.map((r) => r.typecode)).toEqual([0, 2, 3]);
  });

  it("rejects a single changed character", () => {
    const i = 30;
    const c = MAIN_UA[i] === "q" ? "p" : "q";
    expect(() => decodeUnifiedAddress(MAIN_UA.slice(0, i) + c + MAIN_UA.slice(i + 1))).toThrow(/checksum/);
  });

  it("rejects valid Bech32m with the wrong padding", () => {
    // Encoded for "u" but relabelled "uregtest": the checksum is recomputed, the padding is not.
    const words = bytesToWords(Uint8Array.from({ length: 64 }, (_, i) => i));
    expect(() => decodeUnifiedAddress(bech32mEncode("uregtest", words))).toThrow(/padding/);
  });

  it("rejects unknown prefixes and Sapling addresses", () => {
    expect(() => decodeUnifiedAddress(encodeUa("zu", [{ typecode: 3 }]))).toThrow(/unknown address prefix/);
    expect(() => decodeUnifiedAddress("zs1z7rejlpsa98s2rrrfkwmaxu53e4ue0ulcrw0h4x5g8jl04tak0d3mm47vdtahatqrlkngh9sly"))
      .toThrow(AddressError);
  });

  it("rejects malformed items", () => {
    expect(() => decodeUnifiedAddress(encodeUa("u", [{ typecode: 3, len: 42 }]))).toThrow(/length 42/);
    expect(() => decodeUnifiedAddress(encodeUa("u", [{ typecode: 3 }, { typecode: 3 }]))).toThrow(/duplicate/);
    expect(() => decodeUnifiedAddress(encodeUa("u", [{ typecode: 0, len: 20 }, { typecode: 1, len: 20 }, { typecode: 3 }])))
      .toThrow(/both P2PKH and P2SH/);
    expect(() => decodeUnifiedAddress(encodeUa("u", [{ typecode: 0, len: 20 }, { typecode: 0x10, len: 30 }])))
      .not.toThrow();
    expect(() => decodeUnifiedAddress(encodeUa("u", [{ typecode: 0, len: 20 }]))).toThrow(/too short/);
  });
});

describe("checkPayeeAddress", () => {
  it("accepts Orchard receivers on the right network", () => {
    expect(() => checkPayeeAddress(REGTEST_UA_1, "regtest")).not.toThrow();
    expect(() => checkPayeeAddress(MAIN_UA, "main")).not.toThrow();
    expect(isValidPayeeAddress(REGTEST_UA_2, "regtest")).toBe(true);
  });

  it("rejects the wrong network", () => {
    expect(() => checkPayeeAddress(MAIN_UA, "regtest")).toThrow("address is for main, not regtest");
    expect(() => checkPayeeAddress(REGTEST_UA_1, "main")).toThrow("address is for regtest, not main");
    expect(() => checkPayeeAddress(encodeUa("utest", [{ typecode: 3 }]), "main")).toThrow(/test, not main/);
  });

  it("rejects addresses without an Orchard receiver", () => {
    const sapling = encodeUa("u", [{ typecode: 0, len: 20 }, { typecode: 2 }]);
    expect(() => checkPayeeAddress(sapling, "main")).toThrow("address has no Orchard receiver");
    expect(isValidPayeeAddress(sapling, "main")).toBe(false);
    expect(isValidPayeeAddress("", "main")).toBe(false);
  });
});
