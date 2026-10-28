/** Mock `promissory-app` routes for agent allowances and agent requests (see `mock.ts`). */

import type { AgentRequest, Allowance, CreateOrderRequest, Network, Order } from "@promissory/client";

const AGENT = {
  regtest: "uregtest1ynahn0vg27j478s8azkuzthxhfagywduxzgx3ymx5armlml42rtxeeamhlywpj5p4fp469a5zck46tc7rfd79rd0nmps3lhauugm2y9q",
  main: "u10gsry8xrghdry83eg87jfjepxfft8jefk5wg6yjkwavzk9x8ksvhz0xagf7tk33r840qye5zqvpup0xhten3de9wj6pje3cmru93g4mv",
} as const;
const PAYEE = {
  regtest: "uregtest13543hcxsg0xcd7he0nsux3kmz2rqdg905n69x86mlyth67pesmxjpkfg6mwesu9chlpggf6clg58n7zth7uhd4ud9n655l8hkykuwwxj",
  main: AGENT.main,
} as const;

interface Spec {
  id: number;
  order_id: number;
  agent_ua: string;
  label: string;
  cap_zat: number;
  period_secs: number;
  chips: number;
  periods: number;
  first_due: number;
  created_at: number;
  stopped_at: number | null;
  /** Chips released at stop time, frozen. */
  frozen?: number;
}

let allowances: Spec[] = [];
let requests: AgentRequest[] = [];
let nextAllowance = 1;
let nextRequest = 1;

export function seedAgents(network: Network, now: number): void {
  allowances = [
    {
      id: 1,
      order_id: 2,
      agent_ua: AGENT[network],
      label: "Research agent",
      cap_zat: 3_000_000,
      period_secs: 3_600,
      chips: 3,
      periods: 24,
      first_due: now - 1_500,
      created_at: now - 2_000,
      stopped_at: null,
    },
  ];
  const base = { status: "pending", decided_at: null, order_id: null, allowance_id_new: null, error: null } as const;
  requests = [
    {
      ...base,
      id: 2,
      kind: "oneoff",
      payee: PAYEE[network],
      amount_zat: 2_500_000,
      memo: "Invoice 7: GPU hours",
      reason: "Renting compute for the batch job you asked for",
      created_at: now - 90,
    },
    { ...base, id: 1, kind: "allowance_change", allowance_id: 1, new_cap_zat: 6_000_000, reason: "API prices doubled", created_at: now - 600 },
  ];
  nextAllowance = 2;
  nextRequest = 3;
}

function released(s: Spec, now: number): number {
  if (s.frozen !== undefined) return s.frozen;
  const every = s.period_secs / s.chips;
  return Math.max(0, Math.min(s.chips * s.periods, Math.floor((now - s.first_due) / every) + 1));
}

function render(s: Spec, now: number): Allowance {
  const chip = s.cap_zat / s.chips;
  const every = s.period_secs / s.chips;
  const p = Math.max(0, Math.min(s.periods - 1, Math.floor((now - s.first_due) / s.period_secs)));
  const out = released(s, now);
  const inPeriod = Math.max(0, Math.min(s.chips, out - p * s.chips));
  const done = out >= s.chips * s.periods;
  const stopped = s.stopped_at !== null;
  const start = s.first_due + p * s.period_secs;
  return {
    id: s.id,
    order_id: s.order_id,
    status: stopped ? "stopped" : done ? "completed" : now < s.first_due - 60 ? "awaiting_funds" : "active",
    agent_ua: s.agent_ua,
    label: s.label,
    cap_zat: s.cap_zat,
    period_secs: s.period_secs,
    chips: s.chips,
    chip_zat: chip,
    every_secs: every,
    periods: s.periods,
    first_due: s.first_due,
    created_at: s.created_at,
    stopped_at: s.stopped_at,
    replaced_by: null,
    current_period: p + 1,
    period_start: start,
    period_end: start + s.period_secs,
    spent_this_period_zat: inPeriod * chip,
    remaining_this_period_zat: stopped ? 0 : (s.chips - inPeriod) * chip,
    released_total_zat: out * chip,
    next_release_unix: stopped || done ? null : s.first_due + out * every,
    funding: null,
    topup: null,
    keeper: "http://127.0.0.1:8737",
  };
}

const json = (data: unknown, status = 200) =>
  new Response(JSON.stringify(data), { status, headers: { "Content-Type": "application/json" } });
const err = (status: number, error: string) => json({ error }, status);

/** Answers an agent route, or returns null for any other path. */
export function agentRoute(
  method: string,
  path: string,
  body: Record<string, unknown>,
  now: number,
  createOrder: (req: CreateOrderRequest) => Order,
): Response | null {
  if (path === "/allowances" && method === "GET") return json(allowances.map((s) => render(s, now)));
  if (path === "/allowances" && method === "POST") {
    const cap = Number(body.cap_zat);
    const chips = Number(body.chips);
    const period = Number(body.period_secs);
    const periods = Number(body.periods);
    const first = Number(body.first_due ?? now + 600);
    if (!(cap > 0 && chips > 0 && cap % chips === 0 && period % chips === 0 && periods > 0)) {
      return err(400, "cap_zat and period_secs must be multiples of chips");
    }
    const order = createOrder({
      payee: String(body.agent_ua),
      amount_zat: cap / chips,
      every_secs: period / chips,
      count: chips * periods,
      memo: String(body.label || "Agent allowance"),
      first_due: first,
      window: chips,
    });
    const s: Spec = {
      id: nextAllowance++,
      order_id: order.id,
      agent_ua: String(body.agent_ua),
      label: String(body.label ?? ""),
      cap_zat: cap,
      period_secs: period,
      chips,
      periods,
      first_due: first,
      created_at: now,
      stopped_at: null,
    };
    allowances.unshift(s);
    return json({ ...render(s, now), status: "awaiting_funds", funding: order.funding });
  }
  const a = /^\/allowances\/(\d+)(\/stop)?$/.exec(path);
  if (a) {
    const s = allowances.find((x) => x.id === Number(a[1]));
    if (!s) return err(404, `no allowance ${a[1]}`);
    if (method === "GET" && !a[2]) return json(render(s, now));
    if (method === "POST" && a[2]) {
      if (s.stopped_at !== null) return err(400, `allowance ${s.id} is already stopped`);
      s.frozen = released(s, now);
      s.stopped_at = now;
      return json({ allowance: render(s, now), sweep_txid: "9f".repeat(32), cancelled: [s.frozen], may_still_pay: [] });
    }
  }
  if (path === "/requests" && method === "GET") return json(requests);
  if (path === "/requests" && method === "POST") {
    if (requests.filter((r) => r.status === "pending").length >= 10) return err(429, "10 requests are already waiting");
    const r = { ...body, id: nextRequest++, status: "pending", created_at: now, decided_at: null, order_id: null, allowance_id_new: null, error: null } as unknown as AgentRequest;
    requests.unshift(r);
    return json(r);
  }
  const m = /^\/requests\/(\d+)(?:\/(approve|reject))?$/.exec(path);
  if (m) {
    const r = requests.find((x) => x.id === Number(m[1]));
    if (!r) return err(404, `no request ${m[1]}`);
    if (method === "GET" && !m[2]) return json(r);
    if (method === "POST" && m[2]) {
      if (r.status !== "pending") return err(409, `request ${r.id} is ${r.status}, not pending`);
      r.decided_at = now;
      if (m[2] === "reject") {
        r.status = "rejected";
        return json(r);
      }
      r.status = "approved";
      if (r.kind === "oneoff") {
        const req: CreateOrderRequest = { payee: r.payee, amount_zat: r.amount_zat, every_secs: 60, count: 1, first_due: Math.max(r.due ?? 0, now + 600) };
        if (r.memo !== undefined) req.memo = r.memo;
        const order = createOrder(req);
        r.order_id = order.id;
        return json({ request: r, order });
      }
      return json({ request: r });
    }
  }
  return null;
}
