import { describe, expect, it } from "vitest";
import {
  encodeSubscribeLink,
  LinkError,
  parseSubscribeLink,
  subscribeParamErrors,
  subscribeTotalZat,
  type LinkParam,
  type SubscribeParams,
} from "../src/link.js";
import { encodeUa, MAIN_UA, REGTEST_UA_1 } from "./vectors.js";

const BASE = "https://pay.example/s";

const weekly: SubscribeParams = {
  network: "main",
  payee: MAIN_UA,
  amountZat: 1_000_000,
  everySecs: 604_800,
  count: 12,
};

const full: SubscribeParams = {
  network: "regtest",
  payee: REGTEST_UA_1,
  amountZat: 1_000_000,
  everySecs: 180,
  count: 4,
  memo: "Café & co. 100% = ok? #1 ☕",
  start: 1_792_967_720,
  keeper: "http://127.0.0.1:8737",
  hook: "https://hooks.example/so?x=1&y=2",
};

/** The fragment of a valid link with one parameter replaced (or removed with `null`). */
function withParam(param: string, value: string | null): string {
  const pairs = new URL(encodeSubscribeLink(BASE, full)).hash
    .slice(1)
    .split("&")
    .filter((p) => !p.startsWith(`${param}=`));
  if (value !== null) pairs.push(`${param}=${value}`);
  return `#${pairs.join("&")}`;
}

function expectLinkError(link: string, param: LinkParam, message: RegExp | string): void {
  let err: unknown;
  try {
    parseSubscribeLink(link);
  } catch (e) {
    err = e;
  }
  expect(err).toBeInstanceOf(LinkError);
  expect((err as LinkError).param).toBe(param);
  if (typeof message === "string") expect((err as LinkError).message).toBe(message);
  else expect((err as LinkError).message).toMatch(message);
}

describe("encodeSubscribeLink", () => {
  it("produces the documented format", () => {
    expect(encodeSubscribeLink(BASE, weekly)).toBe(
      `${BASE}#v=1&net=main&to=${MAIN_UA}&amt=1000000&every=604800&n=12`,
    );
  });

  it("percent-encodes optional values and replaces an existing fragment", () => {
    const link = encodeSubscribeLink(`${BASE}#old`, full);
    expect(link.startsWith(`${BASE}#v=1&net=regtest&`)).toBe(true);
    expect(link).toContain(`memo=${encodeURIComponent(full.memo!)}`);
    expect(link).toContain("hook=https%3A%2F%2Fhooks.example%2Fso%3Fx%3D1%26y%3D2");
    expect(link).not.toContain("#old");
  });

  it("omits an empty memo", () => {
    expect(encodeSubscribeLink(BASE, { ...weekly, memo: "" })).not.toContain("memo=");
  });

  it("refuses invalid parameters", () => {
    expect(() => encodeSubscribeLink(BASE, { ...weekly, amountZat: 0 })).toThrow(LinkError);
    expect(() => encodeSubscribeLink(BASE, { ...weekly, network: "regtest" })).toThrow(/address is for main/);
  });
});

describe("parseSubscribeLink", () => {
  it("round-trips", () => {
    expect(parseSubscribeLink(encodeSubscribeLink(BASE, weekly))).toEqual(weekly);
    expect(parseSubscribeLink(encodeSubscribeLink(BASE, full))).toEqual(full);
  });

  it("reads a raw + as a space, like URLSearchParams and the Rust parser", () => {
    const link = encodeSubscribeLink(BASE, weekly);
    expect(parseSubscribeLink(`${link}&memo=Pro+plan%2B`).memo).toBe("Pro plan+");
  });

  it("accepts a full URL, a fragment or the bare fragment content", () => {
    const link = encodeSubscribeLink(BASE, weekly);
    const fragment = link.slice(link.indexOf("#"));
    expect(parseSubscribeLink(fragment)).toEqual(weekly);
    expect(parseSubscribeLink(fragment.slice(1))).toEqual(weekly);
    expect(parseSubscribeLink(`  ${link}  `)).toEqual(weekly);
  });

  it("ignores parameter order and unknown parameters", () => {
    const p = parseSubscribeLink(
      `#n=12&every=604800&zzz=1&amt=1000000&to=${MAIN_UA}&future=%E0%A4%A&net=main&v=1&toString=1`,
    );
    expect(p).toEqual(weekly);
  });

  it("treats an empty memo as no memo", () => {
    expect(parseSubscribeLink(withParam("memo", "")).memo).toBeUndefined();
  });

  it("accepts a 400-byte memo and rejects 401 bytes", () => {
    const ok = "é".repeat(200); // 2 bytes each
    expect(parseSubscribeLink(withParam("memo", encodeURIComponent(ok))).memo).toBe(ok);
    expectLinkError(
      withParam("memo", encodeURIComponent(`${ok}x`)),
      "memo",
      "memo is 401 bytes; the limit is 400 bytes of UTF-8",
    );
  });

  it("names each missing required parameter", () => {
    for (const p of ["net", "to", "amt", "every", "n"] as const) {
      expectLinkError(withParam(p, null), p, `missing required parameter "${p}"`);
    }
    expectLinkError(withParam("v", null), "v", 'missing required parameter "v"');
    expectLinkError(BASE, "v", 'missing required parameter "v"');
    expectLinkError("", "v", 'missing required parameter "v"');
  });

  it("rejects other versions", () => {
    expectLinkError(withParam("v", "2"), "v", 'unsupported link version "2"; expected 1');
  });

  it("validates net", () => {
    expectLinkError(withParam("net", "test"), "net", 'net must be "main" or "regtest"');
  });

  it("validates to", () => {
    expectLinkError(withParam("to", "u1nope"), "to", /^to: not a unified address/);
    expectLinkError(withParam("to", MAIN_UA), "to", "to: address is for main, not regtest");
    expectLinkError(
      withParam("to", encodeUa("uregtest", [{ typecode: 2 }])),
      "to",
      "to: address has no Orchard receiver",
    );
  });

  it("validates amt", () => {
    for (const bad of ["0", "-5", "1.5", "1e6", "", "abc", "9007199254740993"]) {
      expectLinkError(withParam("amt", bad), "amt", "amt must be a whole number of zatoshis greater than 0");
    }
    expect(parseSubscribeLink(withParam("amt", "1")).amountZat).toBe(1);
  });

  it("validates every", () => {
    expectLinkError(withParam("every", "59"), "every", "every must be a whole number of seconds, at least 60");
    expectLinkError(withParam("every", "60.5"), "every", /at least 60/);
    expect(parseSubscribeLink(withParam("every", "60")).everySecs).toBe(60);
  });

  it("validates n", () => {
    for (const bad of ["0", "121", "x"]) {
      expectLinkError(withParam("n", bad), "n", "n must be a whole number from 1 to 120");
    }
    expect(parseSubscribeLink(withParam("n", "1")).count).toBe(1);
    expect(parseSubscribeLink(withParam("n", "120")).count).toBe(120);
  });

  it("validates start, keeper and hook", () => {
    expectLinkError(withParam("start", "0"), "start", "start must be a unix time in seconds");
    expectLinkError(withParam("start", "soon"), "start", /unix time/);
    expectLinkError(withParam("keeper", "ftp%3A%2F%2Fk"), "keeper", "keeper must be an http(s) URL");
    expectLinkError(withParam("keeper", "not%20a%20url"), "keeper", /http\(s\) URL/);
    expectLinkError(withParam("hook", "javascript%3Aalert(1)"), "hook", "hook must be an http(s) URL");
  });

  it("rejects duplicates and bad percent-encoding of known parameters", () => {
    expectLinkError(`${withParam("n", "4")}&n=5`, "n", "n appears more than once");
    expectLinkError(withParam("memo", "%E0%A4%A"), "memo", "memo is not valid percent-encoding");
  });

  it("reports the first error in parameter order", () => {
    expectLinkError(`#v=1&net=main&to=${MAIN_UA}&amt=0&every=1&n=0`, "amt", /amt/);
  });
});

describe("subscribeParamErrors", () => {
  it("collects every error at once", () => {
    const errors = subscribeParamErrors({ network: "main", payee: REGTEST_UA_1, amountZat: 0, everySecs: 30 });
    expect(Object.keys(errors).sort()).toEqual(["amt", "every", "n", "to"]);
  });

  it("is empty for valid parameters", () => {
    expect(subscribeParamErrors(full)).toEqual({});
  });

  it("skips the network check on the address when net is invalid", () => {
    const errors = subscribeParamErrors({ ...weekly, network: "nope" as never });
    expect(errors.net).toBeDefined();
    expect(errors.to).toBeUndefined();
  });
});

it("subscribeTotalZat multiplies amount by count", () => {
  expect(subscribeTotalZat(weekly)).toBe(12_000_000);
});
