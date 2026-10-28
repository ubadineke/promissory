import { describe, expect, it } from "vitest";
import {
  formatCadence,
  formatCountdown,
  formatDuration,
  FormatError,
  formatZec,
  parseZec,
  truncateMiddle,
} from "../src/format.js";

describe("formatZec", () => {
  it("formats with at least two decimals and no trailing zeros beyond", () => {
    expect(formatZec(0)).toBe("0.00");
    expect(formatZec(1)).toBe("0.00000001");
    expect(formatZec(1_000_000)).toBe("0.01");
    expect(formatZec(4_060_000)).toBe("0.0406");
    expect(formatZec(10_000)).toBe("0.0001");
    expect(formatZec(100_000_000)).toBe("1.00");
    expect(formatZec(624_990_000)).toBe("6.2499");
  });

  it("groups thousands and handles the supply cap exactly", () => {
    expect(formatZec(2_100_000_000_000_000)).toBe("21,000,000.00");
    expect(formatZec(123_456_789_012_345n)).toBe("1,234,567.89012345");
    expect(formatZec(123_456_700_000_000, { grouping: false })).toBe("1234567.00");
  });

  it("supports options and negatives", () => {
    expect(formatZec(1_000_000, { unit: true })).toBe("0.01 ZEC");
    expect(formatZec(100_000_000, { minDecimals: 0 })).toBe("1");
    expect(formatZec(150_000_000, { minDecimals: 8 })).toBe("1.50000000");
    expect(formatZec(-1_000_000)).toBe("-0.01");
  });

  it("rejects fractional zatoshis", () => {
    expect(() => formatZec(0.5)).toThrow(FormatError);
    expect(() => formatZec(Number.NaN)).toThrow(FormatError);
  });
});

describe("parseZec", () => {
  it("parses without float error", () => {
    expect(parseZec("0.01")).toBe(1_000_000);
    expect(parseZec("0.1")).toBe(10_000_000);
    expect(parseZec("0.3")).toBe(30_000_000);
    expect(parseZec("1")).toBe(100_000_000);
    expect(parseZec("1.")).toBe(100_000_000);
    expect(parseZec(".5")).toBe(50_000_000);
    expect(parseZec("0.00000001")).toBe(1);
    expect(parseZec(" 2.5 ZEC ")).toBe(250_000_000);
    expect(parseZec("21000000")).toBe(2_100_000_000_000_000);
    expect(parseZec("0")).toBe(0);
  });

  it("rejects malformed input", () => {
    for (const bad of ["", ".", "abc", "1,5", "1.2.3", "-1", "+1", "1e-3", "0x10"]) {
      expect(() => parseZec(bad), bad).toThrow("enter an amount like 0.01");
    }
    expect(() => parseZec("0.000000001")).toThrow("at most 8 decimal places");
    expect(() => parseZec("21000000.00000001")).toThrow(/supply/);
  });

  it("round-trips with formatZec", () => {
    for (const zat of [1, 99, 1_000_000, 4_060_000, 123_456_789]) {
      expect(parseZec(formatZec(zat, { grouping: false }))).toBe(zat);
    }
  });
});

describe("durations", () => {
  it("uses the largest exact unit", () => {
    expect(formatDuration(180)).toBe("3 min");
    expect(formatDuration(604_800)).toBe("1 week");
    expect(formatDuration(1_209_600)).toBe("2 weeks");
    expect(formatDuration(86_400)).toBe("1 day");
    expect(formatDuration(2_592_000)).toBe("30 days");
    expect(formatDuration(3_600)).toBe("1 hour");
    expect(formatDuration(7_200)).toBe("2 hours");
    expect(formatDuration(5_400)).toBe("90 min");
    expect(formatDuration(60)).toBe("1 min");
    expect(formatDuration(90)).toBe("90 s");
    expect(() => formatDuration(0)).toThrow(FormatError);
    expect(() => formatDuration(1.5)).toThrow(FormatError);
  });

  it("labels cadences", () => {
    expect(formatCadence(604_800)).toBe("every week");
    expect(formatCadence(86_400)).toBe("every day");
    expect(formatCadence(180)).toBe("every 3 min");
    expect(formatCadence(60)).toBe("every minute");
    expect(formatCadence(2_592_000)).toBe("every 30 days");
    expect(formatCadence(1_209_600)).toBe("every 2 weeks");
    expect(formatCadence(0)).toBe("once");
  });

  it("formats countdowns", () => {
    expect(formatCountdown(0)).toBe("00:00");
    expect(formatCountdown(-5)).toBe("00:00");
    expect(formatCountdown(65.9)).toBe("01:05");
    expect(formatCountdown(3_725)).toBe("1:02:05");
    expect(formatCountdown(90_061)).toBe("1d 01:01:01");
  });
});

describe("truncateMiddle", () => {
  it("keeps head and tail", () => {
    expect(truncateMiddle("u1abcdefghijklmnopqrstuvwxyz")).toBe("u1abcdef…uvwxyz");
    expect(truncateMiddle("u1abcdefghijklmnopqrstuvwxyz", 5, 4)).toBe("u1abc…wxyz");
    expect(truncateMiddle("short")).toBe("short");
  });
});
