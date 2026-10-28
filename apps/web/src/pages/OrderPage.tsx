import { useState, type ReactNode } from "react";
import { Link, useParams } from "react-router";
import { ArrowLeft, Check, ExternalLink, TriangleAlert } from "lucide-react";
import {
  ApiError,
  formatCadence,
  formatCountdown,
  formatZec,
  type CancelResult,
  type Funding as FundingInfo,
  type OrderDetail,
} from "@promissory/client";
import { api } from "../api";
import { CancelDialog, CancelResultView } from "../components/CancelDialog";
import { ErrorPanel } from "../components/ErrorPanel";
import { QrCode } from "../components/QrCode";
import { ShareProof } from "../components/ShareProof";
import { StatusChip } from "../components/StatusChip";
import { Timeline } from "../components/Timeline";
import { Address, Button, buttonClass, Card, CardHeader, CopyButton, cx, Dial, Notice, Skeleton, Zec } from "../components/ui";
import { useNow, usePoll } from "../hooks";
import { useInfo } from "../info";
import { canCancel, installmentPhase, orderSteps, type StepState } from "../lib/phase";
import { readSession, writeSession } from "../lib/storage";
import { formatDateTime, hostOf } from "../lib/time";
import { Naira, RateNote } from "../lib/fiat";
import { SwitchNotice } from "./Switch";

const STEPS = [
  { key: "funded", label: "Funded", hint: "Treasury received the funds" },
  { key: "split", label: "Split", hint: "One note reserved per payment" },
  { key: "armed", label: "Armed", hint: "Signed, timelocked, at the keeper" },
] as const;

function Steps({ order }: { order: OrderDetail }) {
  const s = orderSteps(order);
  return (
    <ol className="grid grid-cols-3 gap-2 px-5 py-4 sm:px-6" aria-label="Setup progress">
      {STEPS.map((step, i) => {
        const state: StepState = s[step.key];
        return (
          <li key={step.key} className="flex min-w-0 flex-col gap-2" aria-current={state === "current" ? "step" : undefined}>
            <div className="flex items-center gap-2">
              <span
                className={cx(
                  "flex size-6 shrink-0 items-center justify-center rounded-full text-xs font-semibold transition-colors duration-300",
                  state === "done" && "bg-accent text-accent-fg",
                  state === "current" && "border-2 border-accent text-accent",
                  state === "todo" && "border border-border-strong text-muted",
                )}
              >
                {state === "done" ? <Check className="size-3.5" aria-hidden /> : i + 1}
              </span>
              {i < STEPS.length - 1 && (
                <span aria-hidden className={cx("h-px flex-1", state === "done" ? "bg-accent" : "bg-border")} />
              )}
            </div>
            <div className="min-w-0 pr-2">
              <p className={cx("text-sm font-medium", state === "todo" && "text-muted")}>
                {step.label}
                <span className="sr-only">: {state === "done" ? "done" : state === "current" ? "in progress" : "not started"}</span>
              </p>
              <p className="hidden text-xs text-muted sm:block">{step.hint}</p>
            </div>
          </li>
        );
      })}
    </ol>
  );
}

/** "the next payment" / "the next 3 payments" */
function nextPayments(n: number): string {
  return n === 1 ? "the next payment" : `the next ${n} payments`;
}

/** Each installment note carries its own 0.0001 ZEC fee; the rest of a request is the split's. */
const INSTALLMENT_FEE_ZAT = 10_000;

function Funding({ order, funding, topup = false }: { order: OrderDetail; funding: FundingInfo; topup?: boolean }) {
  const n = funding.installments ?? Math.max(1, Math.floor(funding.zat / (order.amount_zat + INSTALLMENT_FEE_ZAT)));
  const fees = funding.zat - order.amount_zat * n;
  return (
    <Card className="overflow-hidden">
      {topup ? (
        <CardHeader title="Top up to keep payments going">
          This order keeps only {nextPayments(order.window ?? n)} funded, and your treasury is short for{" "}
          {nextPayments(n)}. Pay this and {n === 1 ? "it is" : "they are"} reserved and armed automatically.
        </CardHeader>
      ) : (
        <CardHeader title="Fund this order">Waiting for your payment. The app checks every 15 seconds.</CardHeader>
      )}
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
            {topup ? (
              <p className="mt-1 text-sm text-muted text-pretty">
                With what your treasury already holds, this funds {nextPayments(n)} of <Zec zat={order.amount_zat} />{" "}
                plus fees.
              </p>
            ) : fees > 0 && (
              <p className="mt-1 text-sm text-muted">
                <span className="tabular-nums">
                  {n} × {formatZec(order.amount_zat)}
                </span>{" "}
                plus <Zec zat={fees} /> in network fees
              </p>
            )}
          </div>
          <p className="text-sm text-pretty">
            Pay this once from Zodl or any Zcash wallet. You can close this app after the order is armed.
          </p>
          <div className="flex flex-wrap gap-2">
            <FundingCopy uri={funding.uri} />
            <a href={funding.uri} className={buttonClass("ghost")}>
              <ExternalLink className="size-4" aria-hidden />
              Open in wallet
            </a>
          </div>
          <p className="flex items-center gap-1 text-sm text-muted">
            To <Address value={funding.address} label="Copy treasury address" />
          </p>
          <p className="text-xs text-muted text-pretty">
            Pay from a shielded Ironwood balance if you can. Funds that cross from Orchard reveal the amount on chain.
          </p>
        </div>
      </div>
    </Card>
  );
}

function FundingCopy({ uri }: { uri: string }) {
  return (
    <span className={cx(buttonClass("secondary"), "gap-1 pr-1")}>
      Payment request
      <CopyButton text={uri} label="Copy payment request (ZIP-321 URI)" className="size-8" />
    </span>
  );
}

function Stat({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="min-w-0">
      <dt className="text-sm font-medium text-muted">{label}</dt>
      <dd className="mt-1 truncate text-lg font-bold">{children}</dd>
    </div>
  );
}

function Body({ order, now, marginSecs }: { order: OrderDetail; now: number; marginSecs: number }) {
  const storageKey = `promissory-cancel-${order.id}`;
  const [dialogOpen, setDialogOpen] = useState(false);
  const [lastCancel, setLastCancel] = useState<CancelResult | null>(() => {
    const raw = readSession(storageKey);
    try {
      return raw ? (JSON.parse(raw) as CancelResult) : null;
    } catch {
      return null;
    }
  });

  const next = order.installments.find((i) => installmentPhase(i, order.status, now).kind === "locked");
  const paidCount = order.installments.filter((i) => installmentPhase(i, order.status, now).kind === "paid").length;
  const mayStill = order.installments.filter((i) => installmentPhase(i, order.status, now).kind === "may_still_pay").length;

  const total = order.amount_zat * order.count;
  const prev = next ? order.installments.find((i) => i.k === next.k - 1) : undefined;
  const span = next ? next.release_unix - (prev ? prev.release_unix : next.release_unix - (order.every_secs || 86_400)) : 1;

  return (
    <>
      <section className="mb-8 flex flex-wrap items-center gap-6 rounded-[28px] p-6 sm:p-8" style={{ background: "var(--hero)" }}>
        <div className="min-w-0 flex-[1_1_22rem]">
          <div className="flex flex-wrap items-center gap-3">
            {order.memo && <p className="font-mono text-sm font-medium text-fg/75">Order #{order.id}</p>}
            <StatusChip status={order.status} renewed={!!order.renewed_to} />
          </div>
          <h1 className="mt-2 text-[2.2rem] leading-[1.02] font-extrabold tracking-[-0.03em] text-balance sm:text-[2.9rem]">
            {order.memo || `Order #${order.id}`}
          </h1>
          <p className="mt-3 flex flex-wrap items-center gap-x-1.5 text-base text-fg/80">
            <span>
              <Zec zat={order.amount_zat} className="font-semibold text-fg" />
              <Naira zat={order.amount_zat} />{" "}
              {order.count === 1 ? "once" : `${formatCadence(order.every_secs)}, ${order.count} payments`}, to
            </span>
            <Address value={order.payee} label="Copy payee address" className="text-fg" />
          </p>
        </div>
        {next ? (
          <Dial
            label={paidCount === 0 ? "First payment opens in" : "Next payment opens in"}
            value={formatCountdown(next.release_unix - now)}
            progress={1 - (next.release_unix - now) / Math.max(1, span)}
            dateTime={new Date(next.release_unix * 1000).toISOString()}
          />
        ) : order.status === "completed" ? (
          <Dial label="All paid" value={`${paidCount} of ${order.count}`} progress={1} />
        ) : null}
      </section>

      <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_20rem]">
        <div className="min-w-0 space-y-6">
          {order.status === "cancelling" && (
            <Notice tone="warn" icon={<TriangleAlert className="size-4" aria-hidden />}>
              <strong>Cancelling.</strong> The sweep is on its way.{" "}
              {mayStill > 0
                ? `${mayStill} payment${mayStill > 1 ? "s" : ""} may still pay. Once the keeper has broadcast an installment, cancel cannot stop it.`
                : "Remaining payments will fail as double-spends."}
            </Notice>
          )}
          {order.status === "cancelled" && (
            <Notice>
              <strong>Cancelled.</strong> {paidCount} of {order.count} payments were made; the rest can never pay.
            </Notice>
          )}
          {order.status === "completed" && (
            <Notice tone="ok">
              <strong>Completed.</strong> {order.count === 1 ? "The payment was made." : `All ${order.count} payments were made.`}
            </Notice>
          )}

          {order.status === "awaiting_funds" && order.funding && <Funding order={order} funding={order.funding} />}
          {order.status === "active" && order.topup && <Funding order={order} funding={order.topup} topup />}

          {lastCancel && (
            <Card>
              <CardHeader
                title="Cancel result"
                action={
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => {
                      writeSession(storageKey, null);
                      setLastCancel(null);
                    }}
                  >
                    Dismiss
                  </Button>
                }
              />
              <div className="px-5 pb-5 sm:px-6">
                <CancelResultView result={lastCancel} />
              </div>
            </Card>
          )}

          {order.kind === "inheritance" && <SwitchNotice order={order} />}

          <Card className="overflow-hidden">
            <CardHeader title="Payments">
              First due {formatDateTime(order.first_due)}. Each opens when its drand round is published.
            </CardHeader>
            <Steps order={order} />
            <div className="border-t border-border">
              <Timeline order={order} now={now} />
            </div>
          </Card>
        </div>

        <aside className="min-w-0 space-y-6" aria-label="Order details">
          <Card>
            <dl className="space-y-4 px-5 py-5 sm:px-6">
              <Stat label="Paid">
                <Zec zat={order.paid_zat} /> <span className="text-sm font-normal text-muted">of {formatZec(total)}</span>
                <Naira zat={order.paid_zat} className="block text-xs font-normal text-muted" />
              </Stat>
              <Stat label="Payments">
                <span className="tabular-nums">
                  {paidCount} of {order.count}
                </span>{" "}
                <span className="text-sm font-normal text-muted">made</span>
              </Stat>
              <Stat label="Keeper">
                <span className="font-mono text-sm font-medium" title={order.keeper}>
                  {hostOf(order.keeper)}
                </span>
              </Stat>
            </dl>
            <RateNote className="border-t border-border px-5 py-2.5 text-xs text-muted sm:px-6" />
          </Card>

          {order.status !== "awaiting_funds" && <ShareProof order={order} />}

          {canCancel(order) && (
            <Card className="border-danger/25">
              <div className="space-y-3 px-5 py-5 sm:px-6">
                <h2 className="text-base font-bold">Stop this order</h2>
                <p className="text-sm text-muted text-pretty">
                  Sweeps the reserved funds back to your treasury. Payments the keeper has already released still pay.
                </p>
                <Button variant="danger-soft" className="w-full" onClick={() => setDialogOpen(true)}>
                  Cancel order…
                </Button>
              </div>
            </Card>
          )}
        </aside>
      </div>

      <CancelDialog
        order={order}
        marginSecs={marginSecs}
        open={dialogOpen}
        onClose={() => setDialogOpen(false)}
        onDone={(r) => {
          writeSession(storageKey, JSON.stringify(r));
          setLastCancel(r);
        }}
      />
    </>
  );
}

export function OrderPage() {
  const { id } = useParams();
  const orderId = Number(id);
  const valid = Number.isSafeInteger(orderId) && orderId >= 0;
  const poll = usePoll(() => (valid ? api.getOrder(orderId) : Promise.reject(new ApiError(404, "no such order"))), 5_000, [orderId]);
  const now = useNow();
  const info = useInfo();

  return (
    <div>
      <Link
        to="/orders"
        className="mb-4 inline-flex h-9 items-center gap-1.5 rounded-xl text-[15px] font-medium text-muted hover:text-fg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <ArrowLeft className="size-4" aria-hidden />
        All orders
      </Link>
      {poll.data ? (
        <Body order={poll.data} now={now} marginSecs={info.data?.cancel_margin_secs ?? 600} />
      ) : poll.error ? (
        poll.error instanceof ApiError && (poll.error.status === 404 || /^no (such )?order/i.test(poll.error.message)) ? (
          <ErrorPanel title="Order not found" error={new Error(`There is no order #${id} in this app.`)} />
        ) : (
          <ErrorPanel error={poll.error} onRetry={() => void poll.refresh()} />
        )
      ) : (
        <div className="space-y-4" aria-busy="true" aria-label="Loading order">
          <Skeleton className="h-8 w-48" />
          <Skeleton className="h-5 w-80 max-w-full" />
          <Skeleton className="h-36 w-full" />
          <Skeleton className="h-72 w-full" />
        </div>
      )}
    </div>
  );
}
