/** Display helpers. Amount conversion is string and bigint based; no float math touches money. */

export const ZAT_PER_ZEC = 100_000_000;
const DECIMALS = 8;
/** 21 million ZEC: no valid amount exceeds the supply cap. */
export const MAX_MONEY_ZAT = 21_000_000 * ZAT_PER_ZEC;

export class FormatError extends Error {
  override name = "FormatError";
}

export interface FormatZecOptions {
  /** Fewest fraction digits to show; trailing zeros beyond this are dropped. Default 2. */
  minDecimals?: number;
  /** Group the whole part in thousands with commas. Default true. */
  grouping?: boolean;
  /** Append " ZEC". Default false. */
  unit?: boolean;
}

/** Formats zatoshis as ZEC: 1_000_000 → "0.01", 4_060_000 → "0.0406", 100_000_000 → "1.00". */
export function formatZec(zat: number | bigint, opts: FormatZecOptions = {}): string {
  const { minDecimals = 2, grouping = true, unit = false } = opts;
  if (typeof zat === "number" && !Number.isSafeInteger(zat)) {
    throw new FormatError(`not a whole number of zatoshis: ${zat}`);
  }
  let n = BigInt(zat);
  const negative = n < 0n;
  if (negative) n = -n;
  const whole = (n / BigInt(ZAT_PER_ZEC)).toString();
  let frac = (n % BigInt(ZAT_PER_ZEC)).toString().padStart(DECIMALS, "0");
  const keep = Math.max(0, Math.min(DECIMALS, minDecimals));
  while (frac.length > keep && frac.endsWith("0")) frac = frac.slice(0, -1);
  const grouped = grouping ? whole.replace(/\B(?=(\d{3})+(?!\d))/g, ",") : whole;
  const s = `${negative ? "-" : ""}${grouped}${frac ? `.${frac}` : ""}`;
  return unit ? `${s} ZEC` : s;
}

/**
 * Parses a ZEC amount typed by a person into zatoshis: "0.01" → 1_000_000.
 * Accepts digits with an optional "." and up to 8 decimals, surrounding spaces,
 * and a trailing "ZEC". Throws `FormatError`.
 */
export function parseZec(input: string): number {
  const s = input.trim().replace(/\s*zec$/i, "");
  const m = /^(\d*)(?:\.(\d*))?$/.exec(s);
  if (!m || (m[1] === "" && (m[2] ?? "") === "")) {
    throw new FormatError("enter an amount like 0.01");
  }
  const whole = m[1] ?? "";
  const frac = m[2] ?? "";
  if (frac.length > DECIMALS) throw new FormatError("at most 8 decimal places");
  const zat = BigInt(whole || "0") * BigInt(ZAT_PER_ZEC) + BigInt(frac.padEnd(DECIMALS, "0") || "0");
  if (zat > BigInt(MAX_MONEY_ZAT)) throw new FormatError("more than the 21 million ZEC supply");
  return Number(zat);
}

const UNITS = [
  { secs: 604_800, one: "week", many: "weeks" },
  { secs: 86_400, one: "day", many: "days" },
  { secs: 3_600, one: "hour", many: "hours" },
  { secs: 60, one: "min", many: "min" },
  { secs: 1, one: "s", many: "s" },
] as const;

function largestUnit(secs: number): { n: number; one: string; many: string } {
  if (!Number.isSafeInteger(secs) || secs <= 0) {
    throw new FormatError(`not a positive whole number of seconds: ${secs}`);
  }
  const u = UNITS.find((u) => secs % u.secs === 0) ?? UNITS[UNITS.length - 1]!;
  return { n: secs / u.secs, one: u.one, many: u.many };
}

/**
 * Labels a duration in the largest unit that divides it exactly:
 * 180 → "3 min", 604800 → "1 week", 2592000 → "30 days", 90 → "90 s".
 */
export function formatDuration(secs: number): string {
  const { n, one, many } = largestUnit(secs);
  return `${n} ${n === 1 ? one : many}`;
}

/**
 * Labels a cadence: 604800 → "every week", 180 → "every 3 min", 60 → "every minute".
 * 0 → "once": a one-off payment, such as a dead-man's switch, has no cadence.
 */
export function formatCadence(secs: number): string {
  if (secs === 0) return "once";
  const { n, one, many } = largestUnit(secs);
  if (n === 1) return `every ${one === "min" ? "minute" : one === "s" ? "second" : one}`;
  return `every ${n} ${many}`;
}

const pad2 = (n: number) => String(n).padStart(2, "0");

/**
 * A countdown with a stable shape for tabular figures:
 * 65 → "01:05", 3_725 → "1:02:05", 90_061 → "1d 01:01:01". Negative values clamp to "00:00".
 */
export function formatCountdown(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  const d = Math.floor(s / 86_400);
  const h = Math.floor((s % 86_400) / 3_600);
  const m = Math.floor((s % 3_600) / 60);
  const r = s % 60;
  if (d > 0) return `${d}d ${pad2(h)}:${pad2(m)}:${pad2(r)}`;
  if (h > 0) return `${h}:${pad2(m)}:${pad2(r)}`;
  return `${pad2(m)}:${pad2(r)}`;
}

/** Shortens a long string in the middle: "u1abcdef…wxyz". Short input is returned as is. */
export function truncateMiddle(s: string, head = 8, tail = 6): string {
  return s.length <= head + tail + 1 ? s : `${s.slice(0, head)}…${s.slice(-tail)}`;
}
