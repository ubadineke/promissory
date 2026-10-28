import { useEffect, useRef, useState, type ReactNode } from "react";
import { Link, useParams } from "react-router";
import { ArrowLeft, CircleCheck, ExternalLink, TriangleAlert } from "lucide-react";
import {
  ApiError,
  formatCadence,
  formatZec,
  type Batch,
  type BatchCancelResult,
  type Funding,
} from "@promissory/client";
import { api } from "../api";
import { ErrorPanel } from "../components/ErrorPanel";
import { QrCode } from "../components/QrCode";
import { StatusChip } from "../components/StatusChip";
import { Address, Button, buttonClass, Card, CardHeader, CopyButton, cx, Notice, Skeleton, Zec } from "../components/ui";
import { usePoll } from "../hooks";
import { Naira, RateNote } from "../lib/fiat";
import { formatDateTime } from "../lib/time";

const LIVE = ["awaiting_funds", "splitting", "active", "cancelling"];

function Stat({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="min-w-0">
      <dt className="text-sm font-medium text-muted">{label}</dt>
      <dd className="mt-1 truncate text-lg font-bold">{children}</dd>
    </div>
  );
}

function FundingCard({ batch, funding }: { batch: Batch; funding: Funding }) {
  const waiting = batch.orders.filter((o) => o.status === "awaiting_funds").length;
  return (
    <Card className="mb-6 overflow-hidden">
      <CardHeader title="Fund this batch">
        One payment funds {waiting === batch.order_count ? "every order" : `the ${waiting} orders still waiting`}. The app checks every 15
        seconds.
      </CardHeader>
      <div className="grid gap-6 px-5 py-6 sm:grid-cols-[auto_minmax(0,1fr)] sm:px-6">
        <div className="mx-auto w-full max-w-56 self-start rounded-2xl border border-border bg-white p-2.5 shadow-sm sm:mx-0">
          <QrCode value={funding.uri} label={`Payment request for ${formatZec(funding.zat)} ZEC`} size={208} />
        </div>
        <div className="min-w-0 space-y-4">
          <div>
            <p className="text-sm text-muted">Send exactly</p>
            <p className="font-display text-4xl font-extrabold tracking-tight">
              <Zec zat={funding.zat} />
            </p>
            <p className="mt-1 text-sm text-muted">
              {waiting} {waiting === 1 ? "order" : "orders"}, network fees included
              <Naira zat={funding.zat} />
            </p>
          </div>
          <p className="text-sm text-pretty">
            Each order is split and armed on its own as soon as the treasury can cover it, one after another while change confirms. You
            can close this app once every order is armed.
          </p>
          <div className="flex flex-wrap gap-2">
            <span className={cx(buttonClass("secondary"), "gap-1 pr-1")}>
              Payment request
              <CopyButton text={funding.uri} label="Copy payment request (ZIP-321 URI)" className="size-8" />
            </span>
            <a href={funding.uri} className={buttonClass("ghost")}>
              <ExternalLink className="size-4" aria-hidden />
              Open in wallet
            </a>
          </div>
          <p className="flex items-center gap-1 text-sm text-muted">
            To <Address value={funding.address} label="Copy treasury address" />
          </p>
        </div>
      </div>
    </Card>
  );
}

function Payees({ batch }: { batch: Batch }) {
  return (
    <Card className="mb-6 overflow-hidden">
      <CardHeader title="Payees">Each payee has their own standing order, signed and timelocked like any other.</CardHeader>
      <ul className="divide-y divide-border sm:hidden">
        {batch.orders.map((o) => {
          const paid = o.installments?.filter((i) => i.state === "paid").length ?? Math.round(o.paid_zat / o.amount_zat);
          return (
            <li key={o.id}>
              <Link to={`/orders/${o.id}`} className="block space-y-1.5 px-5 py-3.5 hover:bg-surface-2/60 focus-visible:bg-surface-2 focus-visible:outline-none">
                <div className="flex items-center justify-between gap-3">
                  <span className="truncate font-medium">{o.name}</span>
                  <StatusChip status={o.status} />
                </div>
                <p className="text-sm text-muted">
                  <Zec zat={o.amount_zat} className="text-fg" /> {formatCadence(o.every_secs)}
                </p>
                <p className="text-xs text-muted tabular-nums">
                  #{o.id} · {paid} of {o.count} paid · {formatZec(o.paid_zat)} ZEC
                  <Naira zat={o.paid_zat} />
                </p>
              </Link>
            </li>
          );
        })}
      </ul>
      <div className="hidden overflow-x-auto sm:block">
        <table className="w-full min-w-[42rem] text-left text-sm">
          <thead className="text-sm text-muted">
            <tr className="border-b border-border">
              <th scope="col" className="py-2.5 pl-5 pr-2 font-medium sm:pl-6">Name</th>
              <th scope="col" className="px-2 py-2.5 font-medium">Each</th>
              <th scope="col" className="px-2 py-2.5 font-medium">Status</th>
              <th scope="col" className="px-2 py-2.5 font-medium">Payments</th>
              <th scope="col" className="py-2.5 pl-2 pr-5 text-right font-medium sm:pr-6">Paid</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-border">
            {batch.orders.map((o) => {
              const paid = o.installments?.filter((i) => i.state === "paid").length ?? Math.round(o.paid_zat / o.amount_zat);
              return (
                <tr key={o.id}>
                  <td className="py-3 pl-5 pr-2 sm:pl-6">
                    <Link to={`/orders/${o.id}`} className="font-medium hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
                      {o.name}
                    </Link>
                    <span className="mt-0.5 flex items-center gap-1 text-xs text-muted">
                      #{o.id} · <Address value={o.payee} head={10} tail={6} label={`Copy ${o.name}’s address`} />
                    </span>
                  </td>
                  <td className="px-2 py-3">
                    <Zec zat={o.amount_zat} />
                    <span className="block text-xs text-muted">{formatCadence(o.every_secs)}</span>
                  </td>
                  <td className="px-2 py-3">
                    <StatusChip status={o.status} />
                  </td>
                  <td className="px-2 py-3">
                    <div className="flex items-center gap-2 text-xs text-muted">
                      <div className="h-1.5 w-16 overflow-hidden rounded-full bg-surface-2" aria-hidden>
                        <div className="h-full rounded-full bg-accent" style={{ width: `${Math.min(100, (paid / o.count) * 100)}%` }} />
                      </div>
                      <span className="tabular-nums">
                        {paid} of {o.count}
                      </span>
                    </div>
                  </td>
                  <td className="py-3 pl-2 pr-5 text-right sm:pr-6">
                    <Zec zat={o.paid_zat} />
                    <Naira zat={o.paid_zat} className="block text-xs text-muted" />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </Card>
  );
}

function CancelBatch({ batch, onDone }: { batch: Batch; onDone: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [result, setResult] = useState<BatchCancelResult>();
  const live = batch.orders.filter((o) => LIVE.includes(o.status));

  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) d.showModal();
    else if (!open && d.open) d.close();
  }, [open]);

  async function confirm() {
    setBusy(true);
    setError(undefined);
    try {
      setResult(await api.cancelBatch(batch.id));
      onDone();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      {live.length > 0 && (
        <Card>
          <div className="flex flex-wrap items-center justify-between gap-4 px-5 py-5 sm:px-6">
            <div className="min-w-0 max-w-prose">
              <h2 className="text-base font-bold">Cancel the whole batch</h2>
              <p className="mt-0.5 text-sm text-muted text-pretty">
                Cancels all {live.length} running orders: each one’s reserved funds are swept back to your treasury. Payments the keeper
                has already released still pay.
              </p>
            </div>
            <Button variant="danger-soft" onClick={() => setOpen(true)}>
              Cancel batch…
            </Button>
          </div>
        </Card>
      )}
      <dialog
        ref={ref}
        aria-labelledby="batch-cancel-title"
        onCancel={(e) => {
          if (busy) e.preventDefault();
          else setOpen(false);
        }}
        className="m-auto w-[calc(100%-2rem)] max-w-lg rounded-3xl border border-border bg-surface p-0 text-fg shadow-2xl"
      >
        <div className="anim-in px-6 py-6">
          {!result ? (
            <>
              <h2 id="batch-cancel-title" className="font-display text-xl font-extrabold tracking-tight">
                Cancel {live.length} standing {live.length === 1 ? "order" : "orders"}?
              </h2>
              <p className="mt-3 text-sm text-muted text-pretty">
                Each order sends its own sweep transaction. Payments inside the race window, or already released by the keeper, may
                still pay.
              </p>
              <ul className="mt-4 divide-y divide-border rounded-2xl border border-border px-4 text-sm">
                {live.map((o) => (
                  <li key={o.id} className="flex justify-between gap-3 py-2">
                    <span className="font-medium">{o.name}</span>
                    <span className="text-muted">
                      <Zec zat={o.amount_zat} /> {formatCadence(o.every_secs)}
                    </span>
                  </li>
                ))}
              </ul>
              {error && (
                <div className="mt-3">
                  <Notice tone="danger">
                    <strong>Cancel failed.</strong> {error}
                  </Notice>
                </div>
              )}
              <div className="mt-5 flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
                <Button onClick={() => setOpen(false)} disabled={busy} autoFocus>
                  Keep batch
                </Button>
                <Button variant="danger" onClick={() => void confirm()} disabled={busy}>
                  {busy ? "Sending sweeps…" : "Cancel batch"}
                </Button>
              </div>
            </>
          ) : (
            <>
              <h2 id="batch-cancel-title" className="flex items-center gap-2 font-display text-xl font-extrabold tracking-tight">
                <CircleCheck className="size-5 text-ok" aria-hidden />
                Cancel sent
              </h2>
              <ul className="mt-3 divide-y divide-border rounded-2xl border border-border px-4 text-sm">
                {result.results.map((r) => (
                  <li key={r.order_id} className="py-2.5">
                    <p className="font-medium">
                      {r.name} <span className="font-normal text-muted">#{r.order_id}</span>
                    </p>
                    {r.error ? (
                      <p className="text-danger">{r.error}</p>
                    ) : (
                      <p className="text-muted">
                        {r.cancelled?.length ?? 0} cancelled
                        {r.may_still_pay && r.may_still_pay.length > 0 && (
                          <span className="text-warn">
                            {" "}
                            · {r.may_still_pay.map((m) => `#${m.k + 1} may still pay (${m.reason})`).join(", ")}
                          </span>
                        )}
                        {r.sweep_txid && <> · sweep {r.sweep_txid.slice(0, 10)}…</>}
                      </p>
                    )}
                  </li>
                ))}
                {result.results.length === 0 && <li className="py-2.5 text-muted">Nothing was left to cancel.</li>}
              </ul>
              <div className="mt-5 flex justify-end">
                <Button variant="primary" onClick={() => setOpen(false)} autoFocus>
                  Done
                </Button>
              </div>
            </>
          )}
        </div>
      </dialog>
    </>
  );
}

function Body({ batch, refresh }: { batch: Batch; refresh: () => void }) {
  const payments = batch.orders.reduce((s, o) => s + o.count, 0);
  const paidCount = batch.orders.reduce((s, o) => s + Math.round(o.paid_zat / o.amount_zat), 0);
  return (
    <>
      <section className="mb-8 min-w-0 rounded-[28px] p-6 sm:p-8" style={{ background: "var(--hero)" }}>
        <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
          <h1 className="min-w-0 text-[2.1rem] leading-[1.05] font-extrabold tracking-[-0.03em] text-balance sm:text-[2.6rem]">
            {batch.name || `Batch #${batch.id}`}
          </h1>
          <StatusChip status={batch.status} />
        </div>
        <p className="mt-3 text-base text-fg/80">
          Payroll batch #{batch.id} · {batch.order_count} payees · {payments} payments · first {formatDateTime(batch.first_due)}
        </p>
      </section>

      {batch.status === "cancelling" && (
        <div className="mb-6">
          <Notice tone="warn" icon={<TriangleAlert className="size-4" aria-hidden />}>
            <strong>Cancelling.</strong> Some payments were inside the race window or already released and may still pay.
          </Notice>
        </div>
      )}
      {batch.status === "cancelled" && (
        <div className="mb-6">
          <Notice>
            <strong>Cancelled.</strong> {paidCount} of {payments} payments were made; the rest can never pay.
          </Notice>
        </div>
      )}

      <Card className="mb-6">
        <dl className="grid grid-cols-2 gap-4 px-5 py-5 sm:grid-cols-3 sm:px-6">
          <Stat label="Paid">
            <Zec zat={batch.paid_zat} /> <span className="text-sm font-normal text-muted">of {formatZec(batch.total_zat)}</span>
          </Stat>
          <Stat label="Payments made">
            <span className="tabular-nums">
              {paidCount} <span className="text-sm font-normal text-muted">of {payments}</span>
            </span>
          </Stat>
          <Stat label="Total">
            <Zec zat={batch.total_zat} />
            <Naira zat={batch.total_zat} className="block text-xs font-normal text-muted" />
          </Stat>
        </dl>
        <RateNote className="border-t border-border px-5 py-2.5 text-xs text-muted sm:px-6" />
      </Card>

      {batch.funding && <FundingCard batch={batch} funding={batch.funding} />}
      <Payees batch={batch} />
      <CancelBatch batch={batch} onDone={refresh} />
    </>
  );
}

export function BatchPage() {
  const { id } = useParams();
  const batchId = Number(id);
  const valid = Number.isSafeInteger(batchId) && batchId >= 0;
  const poll = usePoll(() => (valid ? api.getBatch(batchId) : Promise.reject(new ApiError(404, "no such batch"))), 10_000, [batchId]);

  return (
    <div>
      <Link
        to="/payroll"
        className="mb-4 inline-flex h-9 items-center gap-1.5 rounded-xl text-[15px] font-medium text-muted hover:text-fg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <ArrowLeft className="size-4" aria-hidden />
        Payroll
      </Link>
      {poll.data ? (
        <Body batch={poll.data} refresh={() => void poll.refresh()} />
      ) : poll.error ? (
        poll.error instanceof ApiError && poll.error.status === 404 ? (
          <ErrorPanel title="Batch not found" error={new Error(`There is no batch #${id} in this app.`)} />
        ) : (
          <ErrorPanel error={poll.error} onRetry={() => void poll.refresh()} />
        )
      ) : (
        <div className="space-y-4" aria-busy="true" aria-label="Loading batch">
          <Skeleton className="h-8 w-48" />
          <Skeleton className="h-24 w-full" />
          <Skeleton className="h-72 w-full" />
        </div>
      )}
    </div>
  );
}
