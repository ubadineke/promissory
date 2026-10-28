import { useState, type ReactNode } from "react";
import { Link } from "react-router";
import { ChevronRight, Plus, RefreshCw } from "lucide-react";
import { formatCadence, formatCountdown, truncateMiddle, type Order } from "@promissory/client";
import { api } from "../api";
import { ErrorPanel } from "../components/ErrorPanel";
import { StatusChip } from "../components/StatusChip";
import { Button, ButtonLink, Card, cx, PageHeader, Ring, Skeleton, Zec } from "../components/ui";
import { useNow, usePoll } from "../hooks";
import { useInfo } from "../info";
import { Naira, RateNote } from "../lib/fiat";
import { formatWhen } from "../lib/time";

function Stat({ label, children, sub }: { label: string; children: ReactNode; sub?: ReactNode }) {
  return (
    <div className="min-w-0">
      <dt className="text-sm font-medium text-fg/75">{label}</dt>
      <dd className="mt-1 truncate font-display text-[2rem] leading-tight font-extrabold tracking-[-0.02em] tabular-nums">{children}</dd>
      {sub && <dd className="mt-1 truncate text-sm text-fg/75">{sub}</dd>}
    </div>
  );
}

function Overview() {
  const { data: info, error, refresh } = useInfo();
  const [syncing, setSyncing] = useState(false);
  const [syncError, setSyncError] = useState<string>();

  async function sync() {
    setSyncing(true);
    setSyncError(undefined);
    try {
      await api.sync();
      await refresh();
    } catch (e) {
      setSyncError(e instanceof Error ? e.message : String(e));
    } finally {
      setSyncing(false);
    }
  }

  if (!info && error) return null;
  const behind = info ? info.tip_height - info.synced_height : 0;
  return (
    <section className="mb-10 rounded-[28px] p-6 sm:p-8" style={{ background: "var(--hero)" }}>
      <div className="flex flex-wrap items-end justify-between gap-6">
        <dl>
          <Stat
            label="Treasury"
            sub={
              info && info.treasury_spendable_zat !== info.treasury_zat ? (
                <>
                  <Zec zat={info.treasury_spendable_zat} /> spendable now
                </>
              ) : (
                "Funds orders before they are split"
              )
            }
          >
            {info ? (
              <>
                <Zec zat={info.treasury_zat} />
                <Naira zat={info.treasury_zat} className="ml-2 font-sans text-base font-medium tracking-normal text-fg/75" />
              </>
            ) : (
              <Skeleton className="h-10 w-40" />
            )}
          </Stat>
        </dl>
        <div className="glass flex items-center gap-4 rounded-3xl px-5 py-3.5">
          <dl className="min-w-0">
            <dt className="text-sm text-muted">Synced to block</dt>
            <dd className="font-mono text-xl font-medium tabular-nums">
              {info ? info.synced_height.toLocaleString() : <Skeleton className="h-7 w-24" />}
            </dd>
            {info && (
              <dd className="text-xs text-muted">{behind > 0 ? `${behind} behind tip ${info.tip_height}` : "Up to date with the chain"}</dd>
            )}
          </dl>
          <Button size="sm" onClick={() => void sync()} disabled={syncing || !info}>
            <RefreshCw className={cx("size-4", syncing && "animate-spin motion-reduce:animate-none")} aria-hidden />
            {syncing ? "Syncing" : "Sync"}
          </Button>
        </div>
      </div>
      {syncError && <p className="mt-4 text-sm font-medium text-danger" role="alert">Sync failed: {syncError}</p>}
    </section>
  );
}

/** Due time of the next unpaid installment, estimated from the schedule. */
function nextDue(o: Order, now: number): number | null {
  if (o.status !== "active" && o.status !== "splitting" && o.status !== "awaiting_funds") return null;
  const paid = Math.round(o.paid_zat / o.amount_zat);
  if (paid >= o.count) return null;
  let k = paid;
  while (k < o.count - 1 && o.first_due + k * o.every_secs < now) k++;
  return o.first_due + k * o.every_secs;
}

/** What the row says about the next payment. */
function nextNote(order: Order, due: number | null, now: number): { text: string; tone?: "warn" } | null {
  if (due === null) return null;
  if (order.status !== "active") return { text: `first ${formatWhen(order.first_due)}` };
  if (due > now) return { text: `next in ${formatCountdown(due - now)}` };
  // Past due and unpaid: either it is on its way, or the rolling window ran out of funds.
  return order.topup ? { text: "waiting for a top-up", tone: "warn" } : { text: "paying now" };
}

function OrderRow({ order, now }: { order: Order; now: number }) {
  const paid = Math.round(order.paid_zat / order.amount_zat);
  const note = nextNote(order, nextDue(order, now), now);
  return (
    <li>
      <Link
        to={`/orders/${order.id}`}
        className="group flex items-center gap-4 px-5 py-4 transition-colors hover:bg-surface-2/60 focus-visible:bg-surface-2 focus-visible:outline-none sm:px-6"
      >
        <Ring value={paid / order.count} size={48} thickness={4}>
          <span className="font-mono text-[11px] font-medium tabular-nums">
            {paid}/{order.count}
          </span>
        </Ring>
        <div className="min-w-0 flex-1 space-y-1">
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <span className="text-[15px] font-bold">{order.memo || `Order #${order.id}`}</span>
            <StatusChip status={order.status} renewed={!!order.renewed_to} />
          </div>
          <p className="truncate text-sm text-muted">
            <Zec zat={order.amount_zat} className="font-semibold text-fg" />
            <Naira zat={order.amount_zat} /> {order.count === 1 ? "once" : formatCadence(order.every_secs)} · to{" "}
            <span className="font-mono">{truncateMiddle(order.payee, 10, 6)}</span>
            {order.memo && <> · #{order.id}</>}
          </p>
          <p className="text-sm text-muted">
            <span className="tabular-nums">
              {paid} of {order.count} paid
            </span>
            {note && (
              <span className={cx("tabular-nums", note.tone === "warn" && "font-medium text-warn")}> · {note.text}</span>
            )}
          </p>
        </div>
        <ChevronRight className="size-5 shrink-0 text-muted transition-transform group-hover:translate-x-0.5" aria-hidden />
      </Link>
    </li>
  );
}

export function Dashboard() {
  const orders = usePoll(() => api.listOrders(), 5_000);
  const now = useNow();

  return (
    <div>
      <PageHeader
        title="Your orders"
        subtitle="Signed once, paid on the day, even with this app closed. Cancel any time."
        action={
          <div className="flex flex-wrap gap-2">
            <ButtonLink to="/new?mode=home">Send money home</ButtonLink>
            <ButtonLink to="/new" variant="primary">
              <Plus className="size-4" aria-hidden />
              New standing order
            </ButtonLink>
          </div>
        }
      />
      <Overview />

      <h2 className="mb-4 font-display text-xl font-bold tracking-tight">Orders</h2>
      {orders.loading && !orders.data ? (
        <Card>
          <ul className="divide-y divide-border" aria-busy="true" aria-label="Loading orders">
            {[0, 1, 2].map((i) => (
              <li key={i} className="space-y-2 px-6 py-4">
                <Skeleton className="h-5 w-48" />
                <Skeleton className="h-4 w-72 max-w-full" />
                <Skeleton className="h-3 w-40" />
              </li>
            ))}
          </ul>
        </Card>
      ) : orders.error && !orders.data ? (
        <ErrorPanel error={orders.error} onRetry={() => void orders.refresh()} />
      ) : orders.data && orders.data.length === 0 ? (
        <Card className="px-6 py-12 text-center">
          <h3 className="font-display text-xl font-bold">No orders yet</h3>
          <p className="mx-auto mt-1 max-w-sm text-sm text-muted text-pretty">
            Authorize a series of payments once. Each one is pre-signed and released on its date, even with this app closed.
          </p>
          <div className="mt-5 flex flex-wrap justify-center gap-2">
            <ButtonLink to="/new" variant="primary">
              New standing order
            </ButtonLink>
            <ButtonLink to="/subscribe">Make a subscribe link</ButtonLink>
          </div>
        </Card>
      ) : (
        <Card>
          <ul className="divide-y divide-border">
            {orders.data?.map((o) => <OrderRow key={o.id} order={o} now={now} />)}
          </ul>
        </Card>
      )}
      {orders.data && orders.data.length > 0 && (
        <RateNote className="mt-3 text-xs text-muted" />
      )}
    </div>
  );
}
