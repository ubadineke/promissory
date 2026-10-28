/**
 * Canned `promissory-app` responses for building and screenshotting the UI without a backend.
 * Enabled by `?mock=1` (or `?mock=main` for mainnet data) or `VITE_MOCK=1`.
 * State moves with the clock: installments release, broadcast and mine on schedule.
 */

import {
  parseSubscribeLink,
  type CancelResult,
  type CreateOrderRequest,
  type FetchLike,
  type Info,
  type Installment,
  type Network,
  type OrderDetail,
} from "@promissory/client";
import { agentRoute, seedAgents } from "./mockAgents";

const ADDR = {
  regtest: {
    payee: "uregtest13543hcxsg0xcd7he0nsux3kmz2rqdg905n69x86mlyth67pesmxjpkfg6mwesu9chlpggf6clg58n7zth7uhd4ud9n655l8hkykuwwxj",
    payee2: "uregtest1ynahn0vg27j478s8azkuzthxhfagywduxzgx3ymx5armlml42rtxeeamhlywpj5p4fp469a5zck46tc7rfd79rd0nmps3lhauugm2y9q",
    treasury: "uregtest1ynahn0vg27j478s8azkuzthxhfagywduxzgx3ymx5armlml42rtxeeamhlywpj5p4fp469a5zck46tc7rfd79rd0nmps3lhauugm2y9q",
  },
  main: {
    payee: "u10gsry8xrghdry83eg87jfjepxfft8jefk5wg6yjkwavzk9x8ksvhz0xagf7tk33r840qye5zqvpup0xhten3de9wj6pje3cmru93g4mv",
    payee2: "u10gsry8xrghdry83eg87jfjepxfft8jefk5wg6yjkwavzk9x8ksvhz0xagf7tk33r840qye5zqvpup0xhten3de9wj6pje3cmru93g4mv",
    treasury: "u10gsry8xrghdry83eg87jfjepxfft8jefk5wg6yjkwavzk9x8ksvhz0xagf7tk33r840qye5zqvpup0xhten3de9wj6pje3cmru93g4mv",
  },
} as const;

const FEE = 10_000;
const KEEPER = "http://127.0.0.1:8737";
const CANCEL_MARGIN = 60;
const QUICKNET_GENESIS = 1_692_803_367;
const BLOCK_SECS = 75;

const unix = () => Math.floor(Date.now() / 1000);
const roundAt = (t: number) => Math.ceil((t - QUICKNET_GENESIS) / 3) + 1;

function hexId(seed: number, len = 64): string {
  let x = (seed * 2654435761) >>> 0;
  let s = "";
  while (s.length < len) {
    x = (x ^ (x << 13)) >>> 0;
    x = (x ^ (x >>> 17)) >>> 0;
    x = (x ^ (x << 5)) >>> 0;
    s += x.toString(16).padStart(8, "0");
  }
  return s.slice(0, len);
}

function fundingZat(amount: number, count: number): number {
  return count * (amount + FEE) + 5_000 * Math.max(2, count);
}

function zecString(zat: number): string {
  const whole = Math.floor(zat / 1e8);
  const frac = String(zat % 1e8).padStart(8, "0").replace(/0+$/, "");
  return frac ? `${whole}.${frac}` : String(whole);
}

interface Spec {
  id: number;
  payee: string;
  amount_zat: number;
  memo: string;
  every_secs: number;
  count: number;
  first_due: number;
  created_at: number;
  /** Seconds after creation at which funds arrive and the order arms. Infinity: never. */
  fundedAfter: number;
  armedAfter: number;
  cancelledAt?: number;
  /** Fixed per-installment overrides, for static scenarios. */
  fixed?: Partial<Installment>[];
}

let network: Network = "regtest";
let tip0 = 684;
const t0 = unix();
const specs: Spec[] = [];
let nextId = 8;

function tipHeight(now: number): number {
  return tip0 + Math.floor((now - t0) / BLOCK_SECS);
}

function seed(): void {
  const a = ADDR[network];
  specs.length = 0;
  // Live order: two paid, one in flight, three still locked.
  specs.push({
    id: 2,
    payee: a.payee,
    amount_zat: 1_000_000,
    memo: "SO gate",
    every_secs: 180,
    count: 6,
    first_due: t0 - 400,
    created_at: t0 - 1_200,
    fundedAfter: 0,
    armedAfter: 0,
  });
  specs.push({
    id: 7,
    payee: a.payee2,
    amount_zat: 1_000_000,
    memo: "Newsletter, weekly",
    every_secs: 604_800,
    count: 12,
    first_due: t0 + 600,
    created_at: t0 - 30,
    fundedAfter: Infinity,
    armedAfter: Infinity,
  });
  specs.push({
    id: 6,
    payee: a.payee,
    amount_zat: 25_000_000,
    memo: "Rent",
    every_secs: 2_592_000,
    count: 3,
    first_due: t0 + 86_400,
    created_at: t0 - 300,
    fundedAfter: 0,
    armedAfter: Infinity,
  });
  // Cancelled mid-flight: one released by the keeper, one inside the race window.
  specs.push({
    id: 3,
    payee: a.payee2,
    amount_zat: 2_000_000,
    memo: "Allowance",
    every_secs: 180,
    count: 5,
    first_due: t0 - 200,
    created_at: t0 - 1_000,
    fundedAfter: 0,
    armedAfter: 0,
    cancelledAt: t0 - 5,
    fixed: [
      { state: "paid", keeper_state: "mined", mined_height: tip0 - 2, cancellable: false },
      { state: "armed", keeper_state: "released", cancellable: false },
      { state: "armed", keeper_state: "locked", cancellable: false },
      { state: "cancelled", keeper_state: "deleted", cancellable: false },
      { state: "cancelled", keeper_state: "deleted", cancellable: false },
    ],
  });
  // The engine gate: three paid by the keeper, the fourth rejected after cancel.
  specs.push({
    id: 1,
    payee: a.payee,
    amount_zat: 1_000_000,
    memo: "SO gate",
    every_secs: 180,
    count: 4,
    first_due: t0 - 3_600,
    created_at: t0 - 4_400,
    fundedAfter: 0,
    armedAfter: 0,
    cancelledAt: t0 - 3_200,
    fixed: [
      { state: "paid", keeper_state: "mined", mined_height: 224 },
      { state: "paid", keeper_state: "mined", mined_height: 235 },
      { state: "paid", keeper_state: "mined", mined_height: 247 },
      { state: "cancelled", keeper_state: "cancelled", mined_height: null },
    ],
  });
  specs.push({
    id: 4,
    payee: a.payee2,
    amount_zat: 50_000_000,
    memo: "Grant milestone",
    every_secs: 86_400,
    count: 2,
    first_due: t0 - 200_000,
    created_at: t0 - 201_000,
    fundedAfter: 0,
    armedAfter: 0,
    fixed: [
      { state: "paid", keeper_state: "mined", mined_height: 120, cancellable: false },
      { state: "paid", keeper_state: "mined", mined_height: 131, cancellable: false },
    ],
  });
}

function installment(s: Spec, k: number, now: number): Installment {
  const due = s.first_due + k * s.every_secs;
  const release = due + 1;
  const armed = now >= s.created_at + s.armedAfter;
  const base: Installment = {
    k,
    due_unix: due,
    round: roundAt(release),
    release_unix: release,
    state: armed ? "armed" : "reserved",
    keeper_state: armed ? "locked" : null,
    txids: armed ? [hexId(s.id * 100 + k)] : [],
    mined_height: null,
    blob_id: armed ? hexId(s.id * 1000 + k, 32) : null,
    cancellable: armed ? release - now > CANCEL_MARGIN : true,
  };
  const fixed = s.fixed?.[k];
  if (fixed) return { ...base, ...fixed };
  if (s.cancelledAt !== undefined && release > s.cancelledAt + CANCEL_MARGIN) {
    return { ...base, state: "cancelled", keeper_state: armed ? "deleted" : null, cancellable: false };
  }
  if (!armed) return base;
  const since = now - release;
  if (since < 0) return base;
  // Keeper timeline after the round: released (+3 s), broadcast (+8 s), mined (+40 s).
  if (since < 3) return { ...base, cancellable: false };
  if (since < 8) return { ...base, keeper_state: "released", cancellable: false };
  if (since < 40) return { ...base, keeper_state: "broadcast", cancellable: false };
  const height = Math.max(1, tipHeight(release + 40));
  return { ...base, state: "paid", keeper_state: "mined", mined_height: height, cancellable: false };
}

function render(s: Spec, now: number): OrderDetail {
  const installments = Array.from({ length: s.count }, (_, k) => installment(s, k, now));
  const paid = installments.filter((i) => i.state === "paid").length;
  const funded = now >= s.created_at + s.fundedAfter;
  const armed = now >= s.created_at + s.armedAfter;
  let status: OrderDetail["status"];
  if (s.cancelledAt !== undefined) {
    const open = installments.some((i) => i.state === "armed");
    status = open ? "cancelling" : "cancelled";
  } else if (!funded) status = "awaiting_funds";
  else if (!armed) status = "splitting";
  else status = paid === s.count ? "completed" : "active";
  const treasury = ADDR[network].treasury;
  const zat = fundingZat(s.amount_zat, s.count);
  return {
    id: s.id,
    status,
    payee: s.payee,
    amount_zat: s.amount_zat,
    memo: s.memo,
    every_secs: s.every_secs,
    count: s.count,
    first_due: s.first_due,
    keeper: KEEPER,
    split_txid: funded ? hexId(s.id * 7) : null,
    created_at: s.created_at,
    funding: status !== "awaiting_funds" ? null : { zat, address: treasury, uri: `zcash:${treasury}?amount=${zecString(zat)}&message=Standing%20order%20${s.id}` },
    sweep_txid: s.cancelledAt !== undefined ? hexId(s.id * 13) : null,
    cancelled_at: s.cancelledAt ?? null,
    paid_zat: paid * s.amount_zat,
    installments,
  };
}

function info(now: number): Info {
  return {
    network,
    tip_height: tipHeight(now),
    synced_height: tipHeight(now),
    treasury_address: ADDR[network].treasury,
    treasury_zat: 15_230_000,
    treasury_spendable_zat: 15_230_000,
    default_keeper: KEEPER,
    cancel_margin_secs: CANCEL_MARGIN,
  };
}

function cancel(s: Spec, now: number): CancelResult {
  const before = render(s, now).installments;
  const result: CancelResult = { sweep_txid: hexId(s.id * 13), cancelled: [], may_still_pay: [] };
  for (const i of before) {
    if (i.state === "paid" || i.state === "cancelled") continue;
    if (i.keeper_state === "released" || i.keeper_state === "broadcast") {
      result.may_still_pay.push({ k: i.k, code: "released", reason: "already released by the keeper" });
    } else if (!i.cancellable) {
      result.may_still_pay.push({ k: i.k, code: "race_window", reason: "inside the race window" });
    } else {
      result.cancelled.push(i.k);
    }
  }
  // Freeze what the keeper already had; everything later was swept.
  s.fixed = before.map((i) =>
    result.cancelled.includes(i.k)
      ? { state: "cancelled", keeper_state: i.keeper_state ? "deleted" : null, cancellable: false }
      : i.state === "paid" || i.state === "cancelled"
        ? { ...i }
        : { ...i, cancellable: false },
  );
  s.cancelledAt = now;
  return result;
}

function create(req: CreateOrderRequest, now: number): OrderDetail {
  const s: Spec = {
    id: nextId++,
    payee: req.payee,
    amount_zat: req.amount_zat,
    memo: req.memo ?? "",
    every_secs: req.every_secs,
    count: req.count,
    first_due: req.first_due ?? now + 600,
    created_at: now,
    fundedAfter: 20,
    armedAfter: 35,
  };
  specs.unshift(s);
  return render(s, now);
}

const json = (data: unknown, status = 200) =>
  new Response(status === 204 ? null : JSON.stringify(data), {
    status,
    headers: { "Content-Type": "application/json" },
  });

const err = (status: number, error: string) => json({ error }, status);

export function createMockFetch(net: Network = "regtest"): FetchLike {
  network = net;
  tip0 = net === "main" ? 3_141_592 : 684;
  seed();
  seedAgents(net, unix());
  return async (url, init = {}) => {
    await new Promise((r) => setTimeout(r, 120));
    const method = init.method ?? "GET";
    const path = new URL(url, "http://mock").pathname.replace(/^\/api\/v1/, "");
    const body = typeof init.body === "string" ? (JSON.parse(init.body) as Record<string, unknown>) : {};
    const now = unix();
    if (method !== "GET" && new Headers(init.headers).get("X-Promissory-App") !== "1") {
      return err(403, "missing X-Promissory-App header");
    }

    const agent = agentRoute(method, path, body, now, (req) => create(req, now));
    if (agent) return agent;
    if (method === "GET" && path === "/info") return json(info(now));
    if (method === "POST" && path === "/sync") return json(info(now));
    // Display-only rate; batches are not simulated in mock mode.
    if (method === "GET" && path === "/rates") {
      return json({ vs: "ngn", rate: 512_340, zec_ngn: 512_340, fetched_at: now - 120, source: "mock" });
    }
    if (method === "GET" && path === "/batches") return json([]);
    if (path.startsWith("/batches")) return err(400, "payroll batches need a running promissory-app");
    if (method === "GET" && path === "/orders") {
      return json(specs.map((s) => {
        const { installments: _, ...order } = render(s, now);
        return order;
      }));
    }
    if (method === "POST" && path === "/orders") {
      return json(create(body as unknown as CreateOrderRequest, now), 201);
    }
    if (method === "POST" && path === "/orders/from-link") {
      try {
        const p = parseSubscribeLink(String(body.link ?? ""));
        if (p.network !== network) return err(400, `link is for ${p.network}; this app runs on ${network}`);
        const req: CreateOrderRequest = {
          payee: p.payee,
          amount_zat: p.amountZat,
          every_secs: p.everySecs,
          count: p.count,
        };
        if (p.memo !== undefined) req.memo = p.memo;
        const first = (body.first_due as number | undefined) ?? p.start;
        if (first !== undefined) req.first_due = first;
        return json(create(req, now), 201);
      } catch (e) {
        return err(400, e instanceof Error ? e.message : "invalid link");
      }
    }
    const m = /^\/orders\/(\d+)(\/cancel)?$/.exec(path);
    if (m) {
      const s = specs.find((x) => x.id === Number(m[1]));
      if (!s) return err(404, "no such order");
      if (method === "GET" && !m[2]) return json(render(s, now));
      if (method === "POST" && m[2]) {
        const status = render(s, now).status;
        if (status === "cancelled" || status === "cancelling" || status === "completed") {
          return err(409, `order is ${status}`);
        }
        return json(cancel(s, now));
      }
    }
    return err(404, "not found");
  };
}
