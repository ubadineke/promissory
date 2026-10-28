import { describe, expect, it } from "vitest";
import { AllowanceClient, type AgentRequest, type Allowance } from "@promissory/client";
import { canStop, fundingZat, periodPct, requestTitle, sortRequests } from "./agents";
import { createMockFetch } from "./mock";

const base = { status: "pending", created_at: 1, decided_at: null, order_id: null, allowance_id_new: null, error: null } as const;
const oneoff: AgentRequest = { ...base, id: 1, kind: "oneoff", payee: "uregtest1p", amount_zat: 2_500_000 };
const change: AgentRequest = { ...base, id: 2, kind: "allowance_change", allowance_id: 3, new_cap_zat: 6_000_000 };

describe("agent display rules", () => {
  it("titles requests", () => {
    expect(requestTitle(oneoff)).toBe("Pay 0.025 ZEC once");
    expect(requestTitle(change)).toBe("Change allowance #3 to 0.06 ZEC per period");
  });

  it("puts pending requests first, newest first", () => {
    const done: AgentRequest = { ...oneoff, id: 5, status: "approved" };
    expect(sortRequests([oneoff, done, change]).map((r) => r.id)).toEqual([2, 1, 5]);
  });

  it("computes progress, stoppability and funding", () => {
    const a = { cap_zat: 3_000_000, spent_this_period_zat: 1_000_000, status: "active" } as Allowance;
    expect(periodPct(a)).toBeCloseTo(33.33, 1);
    expect(canStop(a)).toBe(true);
    expect(canStop({ ...a, status: "stopped" })).toBe(false);
    expect(canStop({ ...a, status: "replaced" })).toBe(false);
    // 3 chips of 0.01 ZEC: 3 × 1,010,000 + the split's 15,000.
    expect(fundingZat(1_000_000, 3)).toBe(3_045_000);
    expect(fundingZat(1_000_000, 1)).toBe(1_020_000);
  });
});

describe("mock promissory-app: agents", () => {
  it("serves allowances and approves or rejects requests once", async () => {
    const agents = new AllowanceClient("", { fetch: createMockFetch() });
    const [a] = await agents.list();
    expect(a).toMatchObject({ status: "active", chips: 3 });
    const pending = (await agents.listRequests()).filter((r) => r.status === "pending");
    expect(pending.length).toBeGreaterThanOrEqual(2);
    const oneoffReq = pending.find((r) => r.kind === "oneoff")!;
    const approved = await agents.approveRequest(oneoffReq.id);
    expect(approved.request.status).toBe("approved");
    expect(approved.order?.status).toBe("awaiting_funds");
    await expect(agents.approveRequest(oneoffReq.id)).rejects.toMatchObject({ status: 409 });
    const changeReq = pending.find((r) => r.kind === "allowance_change")!;
    expect((await agents.rejectRequest(changeReq.id)).status).toBe("rejected");
    const stopped = await agents.stop(a!.id);
    expect(stopped.allowance.status).toBe("stopped");
  });
});
