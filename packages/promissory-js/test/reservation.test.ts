import { describe, expect, it } from "vitest";
import type { FetchLike } from "../src/http.js";
import { NoticesClient } from "../src/notices.js";
import {
  decodeReservationProof,
  encodeReservationProof,
  RESERVATION_PREFIX,
  ReservationClient,
  ReservationError,
  type ReservationProof,
} from "../src/reservation.js";

const proof: ReservationProof = {
  v: 1,
  net: "regtest",
  ufvk: "uviewregtest1abc",
  birthday: 952,
  payee: "uregtest1payee",
  amount_zat: 1_000_000,
  note_zat: 1_010_000,
  count: 3,
  every_secs: 600,
  first_due: 1_793_012_788,
  memo: "Café ✓",
  keeper: "http://127.0.0.1:8737",
  blobs: [{ k: 0, round: 33403141, blob_id: "ab12" }],
};

describe("reservation proof", () => {
  it("round-trips, including non-ASCII memos", () => {
    const s = encodeReservationProof(proof);
    expect(s.startsWith(RESERVATION_PREFIX)).toBe(true);
    expect(s).not.toMatch(/[+/=]/);
    expect(decodeReservationProof(`  ${s}\n`)).toEqual(proof);
  });

  it("decodes what promissory-app produces (base64url of the JSON, no padding)", () => {
    const json = JSON.stringify(proof);
    const b64 = Buffer.from(json, "utf8").toString("base64url");
    expect(decodeReservationProof(RESERVATION_PREFIX + b64)).toEqual(proof);
  });

  it("rejects other strings", () => {
    expect(() => decodeReservationProof("promissory-reserve:v2:abc")).toThrow(ReservationError);
    expect(() => decodeReservationProof(RESERVATION_PREFIX + "!!")).toThrow(/base64url/);
    expect(() => decodeReservationProof(RESERVATION_PREFIX + "bm90IGpzb24")).toThrow(/JSON/);
    const v2 = encodeReservationProof({ ...proof, v: 2 as 1 });
    expect(() => decodeReservationProof(v2)).toThrow(/version/);
  });
});

describe("clients", () => {
  it("sends the CSRF header on writes and hits the documented paths", async () => {
    const calls: { url: string; method: string; headers: Headers; body?: string }[] = [];
    const fetch: FetchLike = async (url, init = {}) => {
      calls.push({ url, method: init.method ?? "GET", headers: new Headers(init.headers), body: init.body as string });
      const body = url.endsWith("/reservation") ? { order_id: 1, proof: "promissory-reserve:v1:x" } : url.includes("notices") && init.method === "GET" ? [] : { ok: true };
      return new Response(JSON.stringify(body), { status: 200 });
    };
    const r = new ReservationClient("http://app", { fetch });
    expect(await r.exportProof(1)).toBe("promissory-reserve:v1:x");
    await r.verify(" promissory-reserve:v1:x ");
    await r.keeperStatus("http://k", ["a", "b"]);
    const n = new NoticesClient("http://app", { fetch });
    await n.list();
    await n.dismiss("paid:1:0");
    expect(calls.map((c) => `${c.method} ${c.url}`)).toEqual([
      "GET http://app/api/v1/orders/1/reservation",
      "POST http://app/api/v1/reservations/verify",
      "POST http://app/api/v1/reservations/keeper-status",
      "GET http://app/api/v1/notices",
      "POST http://app/api/v1/notices/paid%3A1%3A0/dismiss",
    ]);
    expect(JSON.parse(calls[1]!.body!)).toEqual({ proof: "promissory-reserve:v1:x" });
    expect(JSON.parse(calls[2]!.body!)).toEqual({ keeper: "http://k", blob_ids: ["a", "b"] });
    for (const c of calls.filter((c) => c.method === "POST")) expect(c.headers.get("x-promissory-app")).toBe("1");
  });
});
