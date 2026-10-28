import { describe, expect, it } from "vitest";
import { checkSchedule, emptySchedule, visibleErrors } from "./schedule";

const REGTEST_UA =
  "uregtest13543hcxsg0xcd7he0nsux3kmz2rqdg905n69x86mlyth67pesmxjpkfg6mwesu9chlpggf6clg58n7zth7uhd4ud9n655l8hkykuwwxj";

describe("checkSchedule", () => {
  it("turns a valid form into Subscribe parameters", () => {
    const r = checkSchedule({ ...emptySchedule(), payee: REGTEST_UA, amount: "0.01", count: "4", cadence: { preset: "custom", custom: "180" } }, "regtest");
    expect(r.errors).toEqual({});
    expect(r.params).toEqual({ network: "regtest", payee: REGTEST_UA, amountZat: 1_000_000, everySecs: 180, count: 4 });
  });

  it("explains every problem in plain words", () => {
    const r = checkSchedule(
      { payee: REGTEST_UA, amount: "0.000000001", cadence: { preset: "custom", custom: "30" }, count: "200", memo: "x".repeat(401), keeper: "ftp://k" },
      "main",
    );
    expect(r.params).toBeUndefined();
    expect(r.errors).toEqual({
      payee: "Address is for regtest, not main",
      amount: "At most 8 decimal places",
      every: "Use a whole number of seconds, at least 60",
      count: "Enter a whole number from 1 to 120",
      memo: "Keep the memo within 400 bytes",
      keeper: "Enter an http:// or https:// URL",
    });
  });

  it("asks for missing fields", () => {
    const r = checkSchedule(emptySchedule(), "main");
    expect(r.errors.payee).toBe("Paste the payee’s unified address");
    expect(r.errors.amount).toBe("Enter an amount like 0.01");
    expect(checkSchedule({ ...emptySchedule(), amount: "0" }, "main").errors.amount).toBe("Enter an amount greater than 0");
  });

  it("hides errors on untouched fields until submit", () => {
    const f = emptySchedule();
    const { errors } = checkSchedule(f, "main");
    expect(visibleErrors(f, errors, false)).toEqual({});
    expect(Object.keys(visibleErrors(f, errors, true)).sort()).toEqual(["amount", "payee"]);
  });
});
