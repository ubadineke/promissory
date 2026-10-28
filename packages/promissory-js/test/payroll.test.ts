import { describe, expect, it } from "vitest";
import type { FetchLike } from "../src/http.js";
import { PayerClient } from "../src/payer.js";
import {
  fiatToZat,
  parsePayrollCadence,
  parsePayrollCsv,
  payrollFundingZat,
  splitCsvLine,
  zatToFiat,
} from "../src/payroll.js";
import { MAIN_UA, REGTEST_UA_1, REGTEST_UA_2 } from "./vectors.js";

describe("parsePayrollCsv", () => {
  it("parses rows with a header, comments, blank lines and quotes", () => {
    const csv = [
      "name,ua,amount,cadence,count",
      "# October",
      "",
      `Ada,${REGTEST_UA_1},0.25,monthly,12`,
      `"Lovelace, Bo",${REGTEST_UA_2},0.01,180,3`,
    ].join("\r\n");
    const p = parsePayrollCsv(csv, "regtest");
    expect(p.errors).toEqual([]);
    expect(p.rows).toEqual([
      { line: 4, name: "Ada", ua: REGTEST_UA_1, amountZat: 25_000_000, everySecs: 2_592_000, count: 12 },
      { line: 5, name: "Lovelace, Bo", ua: REGTEST_UA_2, amountZat: 1_000_000, everySecs: 180, count: 3 },
    ]);
    expect(p.totalZat).toBe(12 * 25_000_000 + 3 * 1_000_000);
    expect(p.fundingZat).toBe(payrollFundingZat(25_000_000, 12) + payrollFundingZat(1_000_000, 3));
  });

  it("accepts a headerless file", () => {
    const p = parsePayrollCsv(`Ada,${REGTEST_UA_1},0.01,weekly,1\n`, "regtest");
    expect(p.rows[0]?.line).toBe(1);
  });

  it("reports every error with its line number", () => {
    const csv = [
      "name,ua,amount,cadence,count",
      `Ada,${REGTEST_UA_1},0.01,weekly,0`,
      ",u1nope,abc,daily,3",
      `Bo,${REGTEST_UA_2},0.01`,
      `Cy,${MAIN_UA},0.000000001,59,2`,
      `Di,${REGTEST_UA_1},0.02,biweekly,4`,
    ].join("\n");
    const p = parsePayrollCsv(csv, "regtest");
    expect(p.rows.map((r) => r.name)).toEqual(["Di"]);
    expect(p.errors.filter((e) => e.line === 2)).toHaveLength(1);
    expect(p.errors.filter((e) => e.line === 3)).toHaveLength(4);
    expect(p.errors.find((e) => e.line === 4)?.message).toMatch(/expected 5 fields/);
    const five = p.errors.filter((e) => e.line === 5).map((e) => e.message);
    expect(five).toHaveLength(3);
    expect(five[0]).toMatch(/^ua: address is for main/);
  });

  it("flags an empty file", () => {
    expect(parsePayrollCsv("name,ua,amount,cadence,count\n", "regtest").errors).toEqual([
      { line: 0, message: "no rows: expected name,ua,amount,cadence,count" },
    ]);
  });
});

describe("payroll helpers", () => {
  it("parses cadences", () => {
    expect(parsePayrollCadence("Weekly")).toBe(604_800);
    expect(parsePayrollCadence("biweekly")).toBe(1_209_600);
    expect(parsePayrollCadence(" 60 ")).toBe(60);
    expect(() => parsePayrollCadence("59")).toThrow(/at least 60/);
    expect(() => parsePayrollCadence("daily")).toThrow(/weekly, biweekly, monthly/);
  });

  it("splits quoted fields", () => {
    expect(splitCsvLine(' "a ""b"", c" , d,')).toEqual(['a "b", c', "d", ""]);
    expect(() => splitCsvLine('"open')).toThrow(/unclosed/);
  });

  it("matches promissory-core's shared funding", () => {
    // promissory-core: funding_needed(0.01, 3) = 3 × 1_010_000 + 15_000; plus 5_000 for the change output.
    expect(payrollFundingZat(1_000_000, 3)).toBe(3_030_000 + 15_000 + 5_000);
    expect(payrollFundingZat(1_000_000, 1)).toBe(1_010_000 + 10_000);
  });

  it("converts fiat to whole zatoshis and back", () => {
    expect(fiatToZat(50_000, 500_000)).toBe(10_000_000);
    expect(fiatToZat(1, 3)).toBe(33_333_333);
    expect(zatToFiat(10_000_000, 500_000)).toBe(50_000);
    expect(() => fiatToZat(0, 1)).toThrow();
  });
});

describe("PayerClient batches and rates", () => {
  const calls: { url: string; method: string; headers: Record<string, string>; body: unknown }[] = [];
  const fetch: FetchLike = async (url, init = {}) => {
    calls.push({
      url,
      method: init.method ?? "GET",
      headers: Object.fromEntries(new Headers(init.headers).entries()),
      body: typeof init.body === "string" ? JSON.parse(init.body) : undefined,
    });
    if (url.includes("/rates")) {
      return new Response(JSON.stringify({ vs: "ngn", rate: 512_000, zec_ngn: 512_000, fetched_at: 1, source: "coingecko" }));
    }
    if (url.endsWith("/batches") && init.method === "POST") {
      return new Response(JSON.stringify({ error: "row 2: count", rows: [{ line: 2, message: "count" }] }), { status: 400 });
    }
    return new Response(JSON.stringify({}));
  };
  const payer = new PayerClient("http://app/", { fetch });

  it("calls the batch endpoints with the CSRF header on writes", async () => {
    await payer.listBatches();
    await payer.getBatch(3);
    await payer.cancelBatch(3, { keep_blobs: true });
    await expect(payer.createBatch({ csv: "x", name: "Oct" })).rejects.toMatchObject({
      status: 400,
      message: "row 2: count",
      body: { rows: [{ line: 2, message: "count" }] },
    });
    expect(calls.map((c) => `${c.method} ${c.url}`)).toEqual([
      "GET http://app/api/v1/batches",
      "GET http://app/api/v1/batches/3",
      "POST http://app/api/v1/batches/3/cancel",
      "POST http://app/api/v1/batches",
    ]);
    expect(calls[2]!.headers["x-promissory-app"]).toBe("1");
    expect(calls[2]!.body).toEqual({ keep_blobs: true });
    expect(calls[3]!.body).toEqual({ csv: "x", name: "Oct" });
  });

  it("gets rates", async () => {
    const r = await payer.rates();
    expect(r.zec_ngn).toBe(512_000);
    expect(calls.at(-1)!.url).toBe("http://app/api/v1/rates?vs=ngn");
  });
});
