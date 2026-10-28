import { useEffect, useMemo, useState, type FormEvent } from "react";
import { useNavigate, useSearchParams } from "react-router";
import { ChevronDown } from "lucide-react";
import { formatCadence, formatZec, type CreateOrderRequest } from "@promissory/client";
import { api } from "../api";
import { ErrorPanel } from "../components/ErrorPanel";
import { ScheduleFields } from "../components/ScheduleFields";
import { SendHomeAmount } from "../components/SendHome";
import { Button, Card, cx, describedBy, Field, inputClass, Notice, PageHeader, Skeleton, Zec } from "../components/ui";
import { useInfo } from "../info";
import { Naira, setNairaEnabled } from "../lib/fiat";
import { checkSchedule, emptySchedule, visibleErrors, type ScheduleForm } from "../lib/schedule";
import { formatDateTime, fromLocalInput, toLocalInput } from "../lib/time";

export function NewOrder() {
  const info = useInfo();
  const navigate = useNavigate();
  const [form, setForm] = useState<ScheduleForm>(emptySchedule);
  const [first, setFirst] = useState(() => toLocalInput(Math.floor(Date.now() / 1000) + 600));
  const [jitter, setJitter] = useState("");
  /** Installments funded ahead; 0 funds every payment now. */
  const [fundAhead, setFundAhead] = useState(0);
  const [submitted, setSubmitted] = useState(false);
  const [busy, setBusy] = useState(false);
  const [apiError, setApiError] = useState<string>();
  const [search, setSearch] = useSearchParams();
  const home = search.get("mode") === "home";

  useEffect(() => {
    if (!home) return;
    // Remittance preset: weekly. The naira rate is display only.
    setNairaEnabled(true);
    setForm((f) => ({ ...f, cadence: { ...f.cadence, preset: 604_800 }, memo: f.memo || "Send home" }));
  }, [home]);

  function setMode(next: "standard" | "home") {
    setSearch(next === "home" ? { mode: "home" } : {}, { replace: true });
  }

  const network = info.data?.network;
  const checked = useMemo(() => (network ? checkSchedule(form, network) : { errors: {} }), [form, network]);

  const firstDue = fromLocalInput(first);
  const firstError =
    firstDue === null ? "Pick a date and time" : firstDue <= Math.floor(Date.now() / 1000) + 60 ? "Pick a time at least a minute from now" : undefined;
  const jitterSecs = jitter.trim() === "" ? undefined : /^\d+$/.test(jitter.trim()) ? Number(jitter.trim()) : Number.NaN;
  const jitterError = jitterSecs !== undefined && !Number.isSafeInteger(jitterSecs) ? "Enter a whole number of seconds" : undefined;
  const errors = visibleErrors(form, checked.errors, submitted);

  if (!info.data) {
    return info.error ? (
      <ErrorPanel error={info.error} onRetry={() => void info.refresh()} />
    ) : (
      <div className="space-y-4" aria-busy="true">
        <Skeleton className="h-8 w-64" />
        <Skeleton className="h-96 w-full" />
      </div>
    );
  }

  const p = checked.params;
  const last = p && firstDue !== null ? firstDue + (p.count - 1) * p.everySecs : null;

  async function submit(e: FormEvent) {
    e.preventDefault();
    setSubmitted(true);
    setApiError(undefined);
    if (!p || firstError || jitterError || firstDue === null) return;
    const req: CreateOrderRequest = {
      payee: p.payee,
      amount_zat: p.amountZat,
      every_secs: p.everySecs,
      count: p.count,
      first_due: firstDue,
    };
    if (p.memo) req.memo = p.memo;
    if (p.keeper) req.keeper = p.keeper;
    if (jitterSecs !== undefined) req.jitter_max_secs = jitterSecs;
    if (fundAhead > 0 && fundAhead < p.count) req.window = fundAhead;
    setBusy(true);
    try {
      const order = await api.createOrder(req);
      navigate(`/orders/${order.id}`);
    } catch (err) {
      setApiError(err instanceof Error ? err.message : String(err));
      setBusy(false);
    }
  }

  return (
    <div>
      {home ? (
        <section className="mb-8 rounded-[28px] p-6 sm:p-8" style={{ background: "var(--hero)" }}>
          <h1 className="text-[2.1rem] leading-[1.05] font-extrabold tracking-[-0.03em] text-balance sm:text-[2.6rem]">Send home</h1>
          <p className="mt-2 max-w-prose text-base text-fg/80 text-pretty">
            A weekly remittance, priced in naira when you set it up and paid in ZEC. Fund it once; every payment is signed now.
          </p>
          {p && (
            <div className="mt-6">
              <p className="text-sm text-fg/80">Each payment</p>
              <p className="font-display text-4xl font-extrabold tracking-tight">
                <Zec zat={p.amountZat} />
                <Naira zat={p.amountZat} className="text-base font-semibold text-fg/80 tabular-nums" />
              </p>
              <p className="mt-1 text-sm text-fg/80">
                {formatCadence(p.everySecs)}, {p.count} {p.count === 1 ? "payment" : "payments"}
              </p>
            </div>
          )}
        </section>
      ) : (
        <PageHeader title="New standing order" subtitle="You fund it once. Every payment is signed now and released on its date." />
      )}
      <div role="radiogroup" aria-label="Order type" className="mb-5 inline-grid grid-cols-2 gap-1 rounded-2xl border border-border bg-surface p-1">
        {(["standard", "home"] as const).map((m) => (
          <button
            key={m}
            type="button"
            role="radio"
            aria-checked={home === (m === "home")}
            onClick={() => setMode(m)}
            className={cx(
              "h-9 rounded-xl px-3.5 text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
              home === (m === "home") ? "bg-accent-soft font-medium text-accent" : "text-muted hover:text-fg",
            )}
          >
            {m === "home" ? "Send home (₦)" : "Standard"}
          </button>
        ))}
      </div>
      <form onSubmit={(e) => void submit(e)} noValidate className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_18rem]">
        <Card className="space-y-6 px-5 py-5 sm:px-6 sm:py-6">
          {home && <SendHomeAmount idPrefix="new" amount={form.amount} setAmount={(amount) => setForm((f) => ({ ...f, amount }))} />}
          <ScheduleFields form={form} set={(patch) => setForm((f) => ({ ...f, ...patch }))} errors={errors} idPrefix="new" />

          <Field id="new-first" label="First payment" hint="Your local time." error={submitted || first ? firstError : undefined}>
            <input
              id="new-first"
              type="datetime-local"
              className={cx(inputClass, "h-11 max-w-64 tabular-nums")}
              value={first}
              onChange={(e) => setFirst(e.target.value)}
              {...describedBy("new-first", firstError)}
            />
          </Field>

          <Field
            id="new-window"
            label="Fund ahead"
            hint={
              fundAhead === 0
                ? "You pay for every payment now; nothing else to do."
                : `You pay for the next ${fundAhead} now. As each is paid, the app reserves the next from your treasury and asks you to top up when it runs short.`
            }
          >
            <select
              id="new-window"
              className={cx(inputClass, "h-11 max-w-72")}
              value={fundAhead}
              onChange={(e) => setFundAhead(Number(e.target.value))}
            >
              <option value={0}>All payments now</option>
              {[1, 2, 3, 6, 12].map((n) => (
                <option key={n} value={n}>
                  Next {n} payment{n === 1 ? "" : "s"} (rolling)
                </option>
              ))}
            </select>
          </Field>

          <details className="group rounded-2xl border border-border">
            <summary className="flex h-11 cursor-pointer list-none items-center justify-between rounded-2xl px-4 text-sm font-semibold focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
              Advanced
              <ChevronDown className="size-4 text-muted transition-transform group-open:rotate-180" aria-hidden />
            </summary>
            <div className="space-y-5 border-t border-border px-4 py-5">
              <Field id="new-keeper" label="Keeper URL" hint={`Releases each payment on time. Default: ${info.data.default_keeper}`} error={errors.keeper}>
                <input
                  id="new-keeper"
                  type="url"
                  className={cx(inputClass, "h-11 font-mono")}
                  placeholder={info.data.default_keeper}
                  autoComplete="url"
                  spellCheck={false}
                  value={form.keeper}
                  onChange={(e) => setForm((f) => ({ ...f, keeper: e.target.value }))}
                  {...describedBy("new-keeper", errors.keeper)}
                />
              </Field>
              <Field
                id="new-jitter"
                label="Release jitter"
                hint="Random extra delay, up to this many seconds, so payments don’t land on round times. Leave empty for the app default; 0 for exact timing."
                error={jitterError}
              >
                <div className="flex items-center gap-2">
                  <input
                    id="new-jitter"
                    className={cx(inputClass, "h-11 max-w-40 tabular-nums")}
                    inputMode="numeric"
                    autoComplete="off"
                    placeholder="default"
                    value={jitter}
                    onChange={(e) => setJitter(e.target.value)}
                    {...describedBy("new-jitter", jitterError)}
                  />
                  <span className="text-sm text-muted">seconds</span>
                </div>
              </Field>
            </div>
          </details>
        </Card>

        <aside className="space-y-4 lg:sticky lg:top-6 lg:self-start">
          <Card className="px-5 py-5 sm:px-6">
            <h2 className="text-base font-bold">Summary</h2>
            {p ? (
              <dl className="mt-3 space-y-2.5 text-sm">
                <div className="flex justify-between gap-3">
                  <dt className="text-muted">Each payment</dt>
                  <dd className="text-right tabular-nums">
                    {formatZec(p.amountZat, { unit: true })}
                    {home && <Naira zat={p.amountZat} className="block text-xs text-muted" />}
                  </dd>
                </div>
                <div className="flex justify-between gap-3">
                  <dt className="text-muted">Schedule</dt>
                  <dd className="text-right">
                    {p.count} payments, {formatCadence(p.everySecs)}
                  </dd>
                </div>
                {firstDue !== null && (
                  <div className="flex justify-between gap-3">
                    <dt className="text-muted">First</dt>
                    <dd className="text-right tabular-nums">{formatDateTime(firstDue)}</dd>
                  </div>
                )}
                {last !== null && p.count > 1 && (
                  <div className="flex justify-between gap-3">
                    <dt className="text-muted">Last</dt>
                    <dd className="text-right tabular-nums">{formatDateTime(last)}</dd>
                  </div>
                )}
                <div className="flex justify-between gap-3 border-t border-border pt-2.5 font-medium">
                  <dt>Total</dt>
                  <dd className="text-right tabular-nums">
                    {formatZec(p.amountZat * p.count, { unit: true })}
                    {home && <Naira zat={p.amountZat * p.count} className="block text-xs font-normal text-muted" />}
                  </dd>
                </div>
                <p className="text-xs text-muted">Plus network fees. The exact amount to fund is shown after you create the order.</p>
              </dl>
            ) : (
              <p className="mt-2 text-sm text-muted">Fill in the payee, amount and schedule to see the total.</p>
            )}
          </Card>
          {apiError && (
            <Notice tone="danger">
              <strong>Couldn’t create the order.</strong> {apiError}
            </Notice>
          )}
          <Button type="submit" variant="primary" className="w-full" disabled={busy}>
            {busy ? "Creating…" : "Create order"}
          </Button>
          <p className="text-xs text-muted text-pretty">
            Nothing is signed or sent yet. Next you’ll get a QR code to fund the order from any Zcash wallet.
          </p>
        </aside>
      </form>
    </div>
  );
}
