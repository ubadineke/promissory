/**
 * Payment receipts (docs/api.md, "Receipts"): `promissory-receipt:v1:<base64url(json)>`. A receipt
 * holds the outgoing cipher key of one output, which opens that output and nothing else.
 */

import type { Network } from "./address.js";

/** Prefix of every receipt string. */
export const RECEIPT_PREFIX = "promissory-receipt:v1:";

/** The decoded JSON inside a receipt. */
export interface Receipt {
  v: 1;
  net: Network | "test";
  /** Transaction ID (display byte order). */
  txid: string;
  pool: "ironwood" | "orchard";
  /** Index of the action in that pool's bundle. */
  action_index: number;
  /** Outgoing cipher key for that one action, 64 hex characters. */
  ock: string;
  /** Claimed payee. */
  payee: string;
  /** Claimed value in zatoshis. */
  value_zat: number;
}

/** `GET /orders/{id}/installments/{k}/receipt`. */
export interface MadeReceipt {
  order_id: number;
  k: number;
  txid: string;
  action_index: number;
  /** The shareable string. */
  receipt: string;
}

/** `POST /receipts/verify`: what the chain shows for a valid receipt. */
export interface VerifiedReceipt {
  txid: string;
  pool: "ironwood" | "orchard";
  action_index: number;
  value_zat: number;
  payee: string;
  memo: string | null;
  /** Block height that mined the transaction. */
  height: number;
}

export class ReceiptError extends Error {
  override name = "ReceiptError";
}

const HEX64 = /^[0-9a-f]{64}$/;

/**
 * Parses a receipt locally, without contacting anything: shows what it claims. Only
 * `PayerClient.verifyReceipt` checks the claim against the chain.
 */
export function decodeReceipt(text: string): Receipt {
  const s = text.trim();
  if (!s.startsWith(RECEIPT_PREFIX)) {
    throw new ReceiptError(`not a receipt (expected it to start with ${RECEIPT_PREFIX})`);
  }
  const body = s.slice(RECEIPT_PREFIX.length).replace(/=+$/, "");
  if (!/^[A-Za-z0-9_-]*$/.test(body)) throw new ReceiptError("receipt is not valid base64url");
  let r: Receipt;
  try {
    const b64 = body.replace(/-/g, "+").replace(/_/g, "/");
    const bin = atob(b64 + "=".repeat((4 - (b64.length % 4)) % 4));
    r = JSON.parse(new TextDecoder().decode(Uint8Array.from(bin, (c) => c.charCodeAt(0)))) as Receipt;
  } catch {
    throw new ReceiptError("receipt is not valid JSON");
  }
  if (r.v !== 1) throw new ReceiptError(`unsupported receipt version ${String(r.v)}`);
  if (r.pool !== "ironwood" && r.pool !== "orchard") throw new ReceiptError(`unsupported receipt pool ${String(r.pool)}`);
  if (typeof r.txid !== "string" || !HEX64.test(r.txid)) throw new ReceiptError("receipt txid must be 32 bytes of hex");
  if (typeof r.ock !== "string" || !HEX64.test(r.ock)) throw new ReceiptError("receipt key must be 32 bytes of hex");
  if (typeof r.payee !== "string" || !Number.isSafeInteger(r.value_zat) || !Number.isSafeInteger(r.action_index)) {
    throw new ReceiptError("receipt is missing fields");
  }
  return r;
}

/** Inverse of `decodeReceipt` (for tests and tools; promissory-app makes real receipts). */
export function encodeReceipt(r: Receipt): string {
  const bytes = new TextEncoder().encode(JSON.stringify(r));
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return RECEIPT_PREFIX + btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}
