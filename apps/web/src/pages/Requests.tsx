import { useState, type ReactNode } from "react";
import { Link, useParams } from "react-router";
import { Bot, ChevronLeft, ShieldCheck } from "lucide-react";
import { formatZec, type AgentRequest, type ApprovedRequest } from "@promissory/client";
import { agents } from "../api";
import { ErrorPanel } from "../components/ErrorPanel";
import { Address, Button, Card, CardHeader, Chip, Notice, PageHeader, Skeleton, Zec } from "../components/ui";
import { usePoll } from "../hooks";
import { REQUEST_STATUS, requestTitle, sortRequests } from "../lib/agents";
import { formatDateTime, formatWhen } from "../lib/time";

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-0.5 py-2">
      <dt className="text-sm text-muted">{label}</dt>
      <dd className="min-w-0 text-sm">{children}</dd>
    </div>
  );
}

function RequestCard({ r, onChange, focus }: { r: AgentRequest; onChange: () => void; focus?: boolean }) {
  const [busy, setBusy] = useState<"approve" | "reject">();
  const [error, setError] = useState<string>();
  const [approved, setApproved] = useState<ApprovedRequest>();
  const s = REQUEST_STATUS[r.status];

  async function decide(how: "approve" | "reject") {
    setBusy(how);
    setError(undefined);
    try {
      if (how === "approve") setApproved(await agents.approveRequest(r.id));
      else await agents.rejectRequest(r.id);
      onChange();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(undefined);
    }
  }

  return (
    <Card className={focus ? "ring-2 ring-ring/40" : undefined}>
      <CardHeader
        title={
          <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <span>{requestTitle(r)}</span>
            <Chip tone={s.tone}>{s.label}</Chip>
          </span>
        }
        action={
          !focus && (
            <Link to={`/requests/${r.id}`} className="text-sm text-muted hover:text-fg">
              #{r.id}
            </Link>
          )
        }
      >
        Asked by an agent {formatWhen(r.created_at)}
      </CardHeader>
      <div className="px-5 py-3 sm:px-6">
        <dl className="divide-y divide-border">
          {r.kind === "oneoff" ? (
            <>
              <Row label="Pay to"><Address value={r.payee} head={12} tail={8} /></Row>
              <Row label="Amount"><Zec zat={r.amount_zat} /></Row>
              <Row label="When">{r.due ? formatDateTime(r.due) : "10 minutes after you approve"}</Row>
              {r.memo && <Row label="Memo"><span className="break-words">{r.memo}</span></Row>}
            </>
          ) : (
            <>
              <Row label="Allowance"><Link className="underline underline-offset-2" to="/allowances">#{r.allowance_id}</Link></Row>
              <Row label="New cap per period"><Zec zat={r.new_cap_zat} /></Row>
              <Row label="Takes effect">From the allowance’s next period</Row>
            </>
          )}
          {r.reason && (
            <Row label="Agent’s reason">
              <span className="break-words italic">“{r.reason}”</span>
            </Row>
          )}
          {r.decided_at && <Row label="Decided">{formatDateTime(r.decided_at)}</Row>}
          {r.order_id !== null && (
            <Row label="Order"><Link className="font-medium underline underline-offset-2" to={`/orders/${r.order_id}`}>#{r.order_id}</Link></Row>
          )}
          {r.allowance_id_new !== null && (
            <Row label="New allowance"><Link className="font-medium underline underline-offset-2" to="/allowances">#{r.allowance_id_new}</Link></Row>
          )}
        </dl>
        {r.status === "pending" && (
          <div className="mt-3 space-y-3">
            <p className="text-sm text-muted text-pretty">
              {r.kind === "oneoff"
                ? `Approving creates a one-payment order of ${formatZec(r.amount_zat)} ZEC from your treasury. Rejecting tells the agent no.`
                : "Approving stops this allowance (unreleased chips go back to your treasury) and starts a new one with the new cap from the next period."}
            </p>
            {(error ?? r.error) && <Notice tone="danger">{error ?? `Last attempt failed: ${r.error}`}</Notice>}
            <div className="flex flex-wrap gap-2">
              <Button variant="primary" size="sm" disabled={busy !== undefined} onClick={() => void decide("approve")}>
                {busy === "approve" ? "Approving…" : "Approve"}
              </Button>
              <Button size="sm" disabled={busy !== undefined} onClick={() => void decide("reject")}>
                {busy === "reject" ? "Rejecting…" : "Reject"}
              </Button>
            </div>
          </div>
        )}
        {approved?.order && (
          <div className="mt-3">
            <Notice tone="ok">
              <strong>Approved.</strong> Order #{approved.order.id} is{" "}
              {approved.order.funding ? "waiting for the treasury to cover it" : "funded"}.{" "}
              <Link className="font-medium underline underline-offset-2" to={`/orders/${approved.order.id}`}>Open the order</Link>
            </Notice>
          </div>
        )}
      </div>
    </Card>
  );
}

function Explainer() {
  return (
    <Notice tone="accent" icon={<ShieldCheck className="size-4" aria-hidden />}>
      AI agents connected through <code className="font-mono">promissory-mcp</code> cannot spend: they can only ask. Nothing is paid or
      changed until you approve here. At most 10 requests can wait at once; undecided ones expire after 7 days.
    </Notice>
  );
}

export function Requests() {
  const list = usePoll(() => agents.listRequests(), 5_000);
  const sorted = list.data ? sortRequests(list.data) : undefined;
  const pending = sorted?.filter((r) => r.status === "pending").length ?? 0;
  return (
    <div>
      <PageHeader
        title="Agent requests"
        subtitle={pending > 0 ? `${pending} waiting for your decision.` : "Payments and changes your agents asked for."}
      />
      <div className="mb-6">
        <Explainer />
      </div>
      {list.error && !sorted ? (
        <ErrorPanel error={list.error} onRetry={() => void list.refresh()} />
      ) : !sorted ? (
        <Skeleton className="h-48 w-full" />
      ) : sorted.length === 0 ? (
        <Card className="px-5 py-10 text-center text-sm text-muted sm:px-6">
          <Bot className="mx-auto mb-2 size-6" aria-hidden />
          No requests yet. See <code className="font-mono">docs/mcp.md</code> to connect an agent.
        </Card>
      ) : (
        <div className="space-y-4">
          {sorted.map((r) => (
            <RequestCard key={r.id} r={r} onChange={() => void list.refresh()} />
          ))}
        </div>
      )}
    </div>
  );
}

export function RequestPage() {
  const { id } = useParams();
  const n = Number(id);
  const req = usePoll(() => agents.getRequest(n), 5_000, [n]);
  return (
    <div>
      <Link
        to="/requests"
        className="mb-4 inline-flex h-9 items-center gap-1.5 rounded-xl text-[15px] font-medium text-muted hover:text-fg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <ChevronLeft className="size-4" aria-hidden />
        All requests
      </Link>
      <PageHeader title={`Request #${id}`} subtitle="An agent asked for this. Review it before you decide." />
      <div className="mb-6">
        <Explainer />
      </div>
      {req.error && !req.data ? (
        <ErrorPanel error={req.error} onRetry={() => void req.refresh()} />
      ) : !req.data ? (
        <Skeleton className="h-48 w-full" />
      ) : (
        <RequestCard r={req.data} focus onChange={() => void req.refresh()} />
      )}
    </div>
  );
}
