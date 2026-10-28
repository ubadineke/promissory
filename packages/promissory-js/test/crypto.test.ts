import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import { blake2b } from "../src/blake2b.js";
import { bech32mDecode, bech32mEncode, bytesToWords, wordsToBytes, Bech32Error } from "../src/bech32m.js";
import { f4jumble, f4jumbleInv } from "../src/f4jumble.js";
import vectors from "./f4jumble-vectors.json" with { type: "json" };
import { hex, unhex } from "./vectors.js";

describe("blake2b", () => {
  it("matches the RFC 7693 vector for 'abc'", () => {
    expect(hex(blake2b(new TextEncoder().encode("abc")))).toBe(
      "ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d1" +
        "7d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923",
    );
  });

  it("matches node:crypto across block boundaries", () => {
    for (const n of [0, 1, 127, 128, 129, 255, 256, 257, 1000]) {
      const data = Uint8Array.from({ length: n }, (_, i) => (i * 31 + 7) & 0xff);
      expect(hex(blake2b(data))).toBe(createHash("blake2b512").update(data).digest("hex"));
    }
  });

  it("rejects bad parameters", () => {
    expect(() => blake2b(new Uint8Array(), 0)).toThrow(RangeError);
    expect(() => blake2b(new Uint8Array(), 65)).toThrow(RangeError);
    expect(() => blake2b(new Uint8Array(), 64, new Uint8Array(15))).toThrow(RangeError);
  });
});

describe("f4jumble", () => {
  it("matches the zcash-test-vectors set", () => {
    for (const v of vectors) {
      expect(hex(f4jumble(unhex(v.normal)))).toBe(v.jumbled);
      expect(hex(f4jumbleInv(unhex(v.jumbled)))).toBe(v.normal);
    }
  });

  it("rejects messages shorter than 38 bytes", () => {
    expect(() => f4jumble(new Uint8Array(37))).toThrow(RangeError);
    expect(() => f4jumbleInv(new Uint8Array(37))).toThrow(RangeError);
    expect(f4jumbleInv(f4jumble(new Uint8Array(38).fill(1)))).toEqual(new Uint8Array(38).fill(1));
  });
});

describe("bech32m", () => {
  it("round-trips", () => {
    const bytes = Uint8Array.from({ length: 70 }, (_, i) => i);
    const s = bech32mEncode("uregtest", bytesToWords(bytes));
    const d = bech32mDecode(s);
    expect(d.hrp).toBe("uregtest");
    expect(wordsToBytes(d.words)).toEqual(bytes);
    expect(bech32mDecode(s.toUpperCase()).hrp).toBe("uregtest");
  });

  it("rejects bad input", () => {
    const s = bech32mEncode("u", bytesToWords(new Uint8Array(50)));
    const flipped = s.slice(0, -1) + (s.endsWith("q") ? "p" : "q");
    expect(() => bech32mDecode(flipped)).toThrow(/checksum/);
    expect(() => bech32mDecode(s.slice(0, 5) + s.slice(5).toUpperCase())).toThrow(/mixed case/);
    expect(() => bech32mDecode("u1qqqqqqb")).toThrow(/invalid character/);
    expect(() => bech32mDecode("noseparator")).toThrow(Bech32Error);
    expect(() => bech32mDecode("u1qqq")).toThrow(/too short/);
  });
});
