/**
 * Subscribe links (docs/api.md §1):
 * `https://<host>/s#v=1&net=main&to=<UA>&amt=<zat>&every=<secs>&n=<count>[&memo=…][&start=…][&keeper=…][&hook=…]`
 *
 * Parameters live in the fragment, so the host serving the page never sees them.
 */

import { AddressError, checkPayeeAddress, type Network } from "./address.js";

export const LINK_VERSION = 1;
export const MEMO_MAX_BYTES = 400;
export const EVERY_MIN_SECS = 60;
export const COUNT_MIN = 1;
export const COUNT_MAX = 120;

/** Suggested values for `every`. */
export const EVERY_PRESETS = {
  day: 86_400,
  week: 604_800,
  days30: 2_592_000,
} as const;

/** A Subscribe link's content. Field names are descriptive; the link keys are in the docs. */
export interface SubscribeParams {
  network: Network;
  /** Payee unified address; must carry an Orchard receiver. (`to`) */
  payee: string;
  /** Zatoshis per installment. (`amt`) */
  amountZat: number;
  /** Seconds between installments, at least 60. (`every`) */
  everySecs: number;
  /** Number of installments, 1–120. (`n`) */
  count: number;
  /** At most 400 bytes of UTF-8. The payer app appends ` (k/n)`. */
  memo?: string;
  /** Unix time of the first installment; defaults to when the payer arms the order. */
  start?: number;
  /** Suggested keeper base URL. */
  keeper?: string;
  /** Webhook for notices; only honoured by keepers that allow webhooks. */
  hook?: string;
}

/** Parameter names as they appear in the link. */
export type LinkParam = "v" | "net" | "to" | "amt" | "every" | "n" | "memo" | "start" | "keeper" | "hook";

/** A missing or invalid parameter. `param` names it as it appears in the link. */
export class LinkError extends Error {
  override name = "LinkError";
  constructor(
    readonly param: LinkParam,
    message: string,
  ) {
    super(message);
  }
}

const FIELD_OF: Record<Exclude<LinkParam, "v">, keyof SubscribeParams> = {
  net: "network",
  to: "payee",
  amt: "amountZat",
  every: "everySecs",
  n: "count",
  memo: "memo",
  start: "start",
  keeper: "keeper",
  hook: "hook",
};

const ORDER: Exclude<LinkParam, "v">[] = ["net", "to", "amt", "every", "n", "memo", "start", "keeper", "hook"];
const REQUIRED = new Set<LinkParam>(["v", "net", "to", "amt", "every", "n"]);

/** UTF-8 length of `s`. */
export function utf8Length(s: string): number {
  return new TextEncoder().encode(s).length;
}

function isHttpUrl(s: string): boolean {
  try {
    const u = new URL(s);
    return (u.protocol === "http:" || u.protocol === "https:") && u.hostname !== "";
  } catch {
    return false;
  }
}

/** Per-parameter checks. Each returns an error message, or `null` if the value is fine. */
export const check = {
  network(v: unknown): string | null {
    return v === "main" || v === "regtest" ? null : 'net must be "main" or "regtest"';
  },
  payee(v: unknown, network: Network | undefined): string | null {
    if (typeof v !== "string" || v.trim() === "") return "to must be a unified address";
    if (network === undefined) return null;
    try {
      checkPayeeAddress(v, network);
      return null;
    } catch (e) {
      if (e instanceof AddressError) return `to: ${e.message}`;
      throw e;
    }
  },
  amountZat(v: unknown): string | null {
    return Number.isSafeInteger(v) && (v as number) > 0
      ? null
      : "amt must be a whole number of zatoshis greater than 0";
  },
  everySecs(v: unknown): string | null {
    return Number.isSafeInteger(v) && (v as number) >= EVERY_MIN_SECS
      ? null
      : `every must be a whole number of seconds, at least ${EVERY_MIN_SECS}`;
  },
  count(v: unknown): string | null {
    return Number.isInteger(v) && (v as number) >= COUNT_MIN && (v as number) <= COUNT_MAX
      ? null
      : `n must be a whole number from ${COUNT_MIN} to ${COUNT_MAX}`;
  },
  memo(v: unknown): string | null {
    if (v === undefined) return null;
    if (typeof v !== "string") return "memo must be text";
    const len = utf8Length(v);
    return len <= MEMO_MAX_BYTES ? null : `memo is ${len} bytes; the limit is ${MEMO_MAX_BYTES} bytes of UTF-8`;
  },
  start(v: unknown): string | null {
    if (v === undefined) return null;
    return Number.isSafeInteger(v) && (v as number) > 0 ? null : "start must be a unix time in seconds";
  },
  keeper(v: unknown): string | null {
    if (v === undefined) return null;
    return typeof v === "string" && isHttpUrl(v) ? null : "keeper must be an http(s) URL";
  },
  hook(v: unknown): string | null {
    if (v === undefined) return null;
    return typeof v === "string" && isHttpUrl(v) ? null : "hook must be an http(s) URL";
  },
} as const;

/**
 * Every problem with `p`, keyed by link parameter. Empty when `p` is a valid set of
 * Subscribe parameters. Suits forms that show all errors at once.
 */
export function subscribeParamErrors(p: Partial<SubscribeParams>): Partial<Record<LinkParam, string>> {
  const errors: Partial<Record<LinkParam, string>> = {};
  const net = check.network(p.network) === null ? p.network : undefined;
  for (const param of ORDER) {
    const field = FIELD_OF[param];
    const value = p[field];
    if (REQUIRED.has(param) && value === undefined) {
      errors[param] = `missing required parameter "${param}"`;
      continue;
    }
    const msg = param === "to" ? check.payee(value, net) : (check[field] as (v: unknown) => string | null)(value);
    if (msg) errors[param] = msg;
  }
  return errors;
}

/** Throws a `LinkError` for the first problem with `p`, in link parameter order. */
export function validateSubscribeParams(p: Partial<SubscribeParams>): asserts p is SubscribeParams {
  const errors = subscribeParamErrors(p);
  const first = ORDER.find((k) => errors[k] !== undefined);
  if (first) throw new LinkError(first, errors[first]!);
}

/**
 * Builds a Subscribe link. `base` is the page that opens it, e.g. `https://pay.example/s`;
 * any fragment already on it is replaced. Throws `LinkError` on invalid parameters.
 */
export function encodeSubscribeLink(base: string, params: SubscribeParams): string {
  validateSubscribeParams(params);
  const pairs: [LinkParam, string | number][] = [
    ["v", LINK_VERSION],
    ["net", params.network],
    ["to", params.payee.trim()],
    ["amt", params.amountZat],
    ["every", params.everySecs],
    ["n", params.count],
  ];
  if (params.memo !== undefined && params.memo !== "") pairs.push(["memo", params.memo]);
  if (params.start !== undefined) pairs.push(["start", params.start]);
  if (params.keeper !== undefined) pairs.push(["keeper", params.keeper]);
  if (params.hook !== undefined) pairs.push(["hook", params.hook]);
  const fragment = pairs.map(([k, v]) => `${k}=${encodeURIComponent(String(v))}`).join("&");
  const hash = base.indexOf("#");
  return `${hash >= 0 ? base.slice(0, hash) : base}#${fragment}`;
}

function parseInteger(s: string): number {
  return /^[0-9]+$/.test(s) ? Number(s) : Number.NaN;
}

/**
 * Parses a Subscribe link. Accepts a full URL, a bare fragment (`#v=1&…`) or the
 * fragment's content (`v=1&…`). Unknown parameters are ignored. Throws `LinkError`
 * naming the first missing or invalid parameter.
 */
export function parseSubscribeLink(urlOrFragment: string): SubscribeParams {
  const input = urlOrFragment.trim();
  const hash = input.indexOf("#");
  const fragment = hash >= 0 ? input.slice(hash + 1) : input.includes("://") ? "" : input;

  const raw = new Map<string, string>();
  for (const part of fragment.split("&")) {
    if (part === "") continue;
    const eq = part.indexOf("=");
    const key = eq >= 0 ? part.slice(0, eq) : part;
    const known = key === "v" || Object.hasOwn(FIELD_OF, key);
    let value: string;
    try {
      // A raw "+" is a space, as URLSearchParams writes it; encodeURIComponent never emits one.
      value = decodeURIComponent((eq >= 0 ? part.slice(eq + 1) : "").replace(/\+/g, " "));
    } catch {
      if (known) throw new LinkError(key as LinkParam, `${key} is not valid percent-encoding`);
      continue;
    }
    if (!known) continue;
    if (raw.has(key)) throw new LinkError(key as LinkParam, `${key} appears more than once`);
    raw.set(key, value);
  }

  const v = raw.get("v");
  if (v === undefined) throw new LinkError("v", 'missing required parameter "v"');
  if (v !== String(LINK_VERSION)) throw new LinkError("v", `unsupported link version "${v}"; expected 1`);

  const p: Partial<SubscribeParams> = {};
  const net = raw.get("net");
  if (net !== undefined) p.network = net as Network;
  const to = raw.get("to");
  if (to !== undefined) p.payee = to;
  for (const [param, field] of [
    ["amt", "amountZat"],
    ["every", "everySecs"],
    ["n", "count"],
    ["start", "start"],
  ] as const) {
    const s = raw.get(param);
    if (s !== undefined) p[field] = parseInteger(s);
  }
  for (const key of ["memo", "keeper", "hook"] as const) {
    const s = raw.get(key);
    if (s !== undefined && s !== "") p[key] = s;
  }
  validateSubscribeParams(p);
  return p;
}

/** Total paid over the whole order, before fees. */
export function subscribeTotalZat(p: Pick<SubscribeParams, "amountZat" | "count">): number {
  return p.amountZat * p.count;
}
