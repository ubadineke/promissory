import { useMemo, useState, type FormEvent } from "react";
import { Link, useNavigate } from "react-router";
import { Bot, ChevronRight, CircleCheck, TriangleAlert } from "lucide-react";
import {
  allowanceErrors,
  checkPayeeAddress,
  formatCountdown,
  formatDuration,
  formatZec,
  parseZec,
  planAllowance,
  ALLOWANCE_LABEL_MAX_CHARS,
  type Allowance,
  type Network,
  type StopAllowanceResult,
} from "@promissory/client";
import { agents } from "../api";
import { ErrorPanel } from "../components/ErrorPanel";
import { Address, Button, ButtonLink, Card, CardHeader, Chip, cx, describedBy, Field, inputClass, Notice, PageHeader, Skeleton, Zec } from "../components/ui";
import { useNow, usePoll } from "../hooks";
import { useInfo } from "../info";
import { ALLOWANCE_STATUS, canStop, fundingZat, periodPct } from "../lib/agents";
import { formatWhen, fromLocalInput, toLocalInput } from "../lib/time";

const PERIODS = [
  { secs: 600, label: "10 minutes" },
  { secs: 3_600, label: "Hour" },
  { secs: 86_400, label: "Day" },
  { secs: 604_800, label: "Week" },
  { secs: 2_592_000, label: "30 days" },
];

function StatusBadge({ a }: { a: Allowance }) {
  const s = ALLOWANCE_STATUS[a.status];
  return (
    <Chip tone={s.tone}>
      <span aria-hidden className={s.live ? "size-1.5 rounded-full bg-current" : "size-1.5 rounded-full border border-current"} />
      {s.label}
    </Chip>
  );
}

function StopPanel({ a, onDone }: { a: Allowance; onDone: () => void }) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [result, setResult] = useState<StopAllowanceResult>();

  async function stop() {
    setBusy(true);
    setError(undefined);
    try {
      setResult(await agents.stop(a.id));
      setConfirming(false);
      onDone();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  if (result) {
    return (
      <Notice tone={result.may_still_pay.length > 0 ? "warn" : "ok"} icon={<CircleCheck className="size-4" aria-hidden />}>
        <strong>Stopped.</strong> {result.cancelled.length} unreleased {result.cancelled.length === 1 ? "chip" : "chips"} swept back to
        your treasury{result.sweep_txid ? <> in <span className="font-mono">{result.sweep_txid.slice(0, 10)}…</span></> : null}.
        {result.may_still_pay.length > 0 && (
          <> {result.may_still_pay.length} {result.may_still_pay.length === 1 ? "chip was" : "chips were"} too close to release and may still reach the agent.</>
        )}
      </Notice>
    );
  }
  if (!canStop(a)) return null;
  return confirming ? (
    <div className="space-y-3 rounded-2xl border border-danger/40 bg-danger-soft/40 p-4">
      <p className="text-sm text-pretty">
        <strong className="font-semibold">Stop this allowance?</strong> Every chip not yet released goes back to your treasury in one
        sweep. Chips already released are the agent’s; a chip due within 10 minutes may still be released.
      </p>
      {error && <p className="text-sm text-danger" role="alert">{error}</p>}
      <div className="flex flex-wrap gap-2">
        <Button variant="danger" size="sm" disabled={busy} onClick={() => void stop()}>
          {busy ? "Stopping…" : "Stop and sweep"}
        </Button>
        <Button size="sm" disabled={busy} onClick={() => setConfirming(false)}>
          Keep it running
        </Button>
      </div>
    </div>
  ) : (
    <Button size="sm" onClick={() => setConfirming(true)}>
      Stop allowance
    </Button>
  );
}

function AllowanceCard({ a, now, onChange }: { a: Allowance; now: number; onChange: () => void }) {
  const pct = periodPct(a);
  const live = a.status === "active" || a.status === "starting";
  return (
    <Card>
      <CardHeader
        title={
          <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <span>{a.label || `Allowance #${a.id}`}</span>
            <StatusBadge a={a} />
          </span>
        }
        action={
          <Link to={`/orders/${a.order_id}`} className="inline-flex items-center gap-1 text-sm text-muted hover:text-fg">
            Order #{a.order_id}
            <ChevronRight className="size-4" aria-hidden />
          </Link>
        }
      >
        Up to <Zec zat={a.cap_zat} /> per {formatDuration(a.period_secs)}: {a.chips} {a.chips === 1 ? "chip" : "chips"} of{" "}
        <Zec zat={a.chip_zat} />, one every {formatDuration(a.every_secs)}.
      </CardHeader>
      <div className="space-y-4 px-5 py-5 sm:px-6">
        <div className="flex flex-wrap items-center gap-x-2 text-sm">
          <span className="text-muted">Agent wallet</span>
          <Address value={a.agent_ua} head={12} tail={8} />
        </div>
        <div className="space-y-1.5">
          <div className="flex flex-wrap items-baseline justify-between gap-2 text-sm">
            <span>
              Period {a.current_period} of {a.periods}
              <span className="text-muted"> · {formatWhen(a.period_start)} to {formatWhen(a.period_end)}</span>
            </span>
            <span className="tabular-nums">
              <Zec zat={a.spent_this_period_zat} unit={false} /> of <Zec zat={a.cap_zat} /> released
            </span>
          </div>
          <div className="h-2 overflow-hidden rounded-full bg-surface-2" role="progressbar" aria-valuenow={Math.round(pct)} aria-valuemin={0} aria-valuemax={100} aria-label="Released this period">
            <div className="h-full rounded-full bg-accent transition-[width]" style={{ width: `${pct}%` }} />
          </div>
        </div>
        <dl className="grid grid-cols-2 gap-3 text-sm sm:grid-cols-3">
          <div>
            <dt className="text-muted">Remaining this period</dt>
            <dd className="mt-0.5 font-medium"><Zec zat={a.remaining_this_period_zat} /></dd>
          </div>
          <div>
            <dt className="text-muted">Next chip</dt>
            <dd className="mt-0.5 font-medium tabular-nums">
              {a.next_release_unix === null ? "—" : a.next_release_unix > now ? `in ${formatCountdown(a.next_release_unix - now)}` : "releasing now"}
            </dd>
          </div>
          <div>
            <dt className="text-muted">Released in total</dt>
            <dd className="mt-0.5 font-medium"><Zec zat={a.released_total_zat} /></dd>
          </div>
        </dl>
        {a.funding && (
          <Notice tone="warn" icon={<TriangleAlert className="size-4" aria-hidden />}>
            <strong>Fund the first period:</strong> <Zec zat={a.funding.zat} /> to your treasury.{" "}
            <Link className="font-medium underline underline-offset-2" to={`/orders/${a.order_id}`}>Show the QR code</Link>
          </Notice>
        )}
        {a.topup && live && (
          <Notice tone="warn">
            <strong>Top up for the next period:</strong> <Zec zat={a.topup.zat} />.{" "}
            <Link className="font-medium underline underline-offset-2" to={`/orders/${a.order_id}`}>Show the QR code</Link>
          </Notice>
        )}
        {a.replaced_by !== null && (
          <p className="text-sm text-muted">Replaced by allowance #{a.replaced_by} after a cap change.</p>
        )}
        <StopPanel a={a} onDone={onChange} />
      </div>
    </Card>
  );
}

function CreateForm({ network }: { network: Network }) {
  const navigate = useNavigate();
  const [agent, setAgent] = useState("");
  const [cap, setCap] = useState("0.03");
  const [period, setPeriod] = useState(network === "regtest" ? 600 : 86_400);
  const [chips, setChips] = useState("3");
  const [periods, setPeriods] = useState(network === "regtest" ? "2" : "30");
  const [first, setFirst] = useState(() => toLocalInput(Math.floor(Date.now() / 1000) + 900));
  const [label, setLabel] = useState("");
  const [touched, setTouched] = useState(false);
  const [busy, setBusy] = useState(false);
  const [apiError, setApiError] = useState<string>();

  const capZat = useMemo(() => {
    try {
      return parseZec(cap);
    } catch {
      return NaN;
    }
  }, [cap]);
  const nChips = Number(chips);
  const nPeriods = Number(periods);
  const shape = allowanceErrors(capZat, period, nChips, nPeriods);
  let agentError: string | undefined;
  try {
    checkPayeeAddress(agent, network);
  } catch (e) {
    agentError = agent === "" ? "Paste the agent’s receiving address" : e instanceof Error ? e.message : "Invalid address";
  }
  const firstDue = fromLocalInput(first);
  const firstError = firstDue === null || firstDue < Math.floor(Date.now() / 1000) + 60 ? "Pick a time at least a minute from now" : undefined;
  const capError = Number.isNaN(capZat) ? "Enter an amount in ZEC" : shape.cap_zat;
  const errors = { agent: agentError, cap: capError, chips: shape.chips, periods: shape.periods, period: shape.period_secs, first: firstError };
  const ok = Object.values(errors).every((e) => e === undefined);
  const plan = planAllowance(capZat, period, nChips, nPeriods);
  const show = (e: string | undefined) => (touched ? e : undefined);

  async function submit(ev: FormEvent) {
    ev.preventDefault();
    setTouched(true);
    if (!ok || firstDue === null) return;
    setBusy(true);
    setApiError(undefined);
    try {
      const a = await agents.create({
        agent_ua: agent,
        cap_zat: capZat,
        period_secs: period,
        chips: nChips,
        periods: nPeriods,
        first_due: firstDue,
        ...(label.trim() ? { label: label.trim() } : {}),
      });
      navigate(`/orders/${a.order_id}`);
    } catch (e) {
      setApiError(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  }

  return (
    <form onSubmit={(e) => void submit(e)} noValidate className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_18rem]">
      <Card className="min-w-0 space-y-5 px-5 py-5 sm:px-6 sm:py-6">
        <Field id="al-agent" label="Agent wallet" hint="The agent’s own unified address. It receives the chips." error={show(errors.agent)}>
          <textarea
            id="al-agent"
            rows={2}
            className={cx(inputClass, "resize-none py-2 font-mono break-all")}
            placeholder={network === "main" ? "u1…" : "uregtest1…"}
            autoComplete="off"
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
            value={agent}
            onChange={(e) => setAgent(e.target.value.replace(/\s+/g, ""))}
            {...describedBy("al-agent", show(errors.agent))}
          />
        </Field>
        <div className="grid gap-5 sm:grid-cols-2">
          <Field id="al-cap" label="Cap per period" hint="The most the agent gets each period." error={show(errors.cap)}>
            <div className="relative">
              <input
                id="al-cap"
                className={cx(inputClass, "h-11 pr-14 tabular-nums")}
                inputMode="decimal"
                autoComplete="off"
                value={cap}
                onChange={(e) => setCap(e.target.value)}
                {...describedBy("al-cap", show(errors.cap))}
              />
              <span className="pointer-events-none absolute inset-y-0 right-3 flex items-center text-sm text-muted">ZEC</span>
            </div>
          </Field>
          <Field id="al-period" label="Period" hint="A new cap starts each period." error={show(errors.period)}>
            <select
              id="al-period"
              className={cx(inputClass, "h-11")}
              value={period}
              onChange={(e) => setPeriod(Number(e.target.value))}
              {...describedBy("al-period", show(errors.period))}
            >
              {PERIODS.filter((p) => network === "regtest" || p.secs >= 3_600).map((p) => (
                <option key={p.secs} value={p.secs}>
                  {p.label}
                </option>
              ))}
            </select>
          </Field>
          <Field id="al-chips" label="Chips per period" hint="Released one at a time, evenly spaced." error={show(errors.chips)}>
            <input
              id="al-chips"
              type="number"
              min={1}
              max={30}
              className={cx(inputClass, "h-11 tabular-nums")}
              value={chips}
              onChange={(e) => setChips(e.target.value)}
              {...describedBy("al-chips", show(errors.chips))}
            />
          </Field>
          <Field id="al-periods" label="Periods" hint="How long the allowance runs." error={show(errors.periods)}>
            <input
              id="al-periods"
              type="number"
              min={1}
              className={cx(inputClass, "h-11 tabular-nums")}
              value={periods}
              onChange={(e) => setPeriods(e.target.value)}
              {...describedBy("al-periods", show(errors.periods))}
            />
          </Field>
          <Field id="al-first" label="First chip" hint="Your local time." error={show(errors.first)}>
            <input
              id="al-first"
              type="datetime-local"
              className={cx(inputClass, "h-11 tabular-nums")}
              value={first}
              onChange={(e) => setFirst(e.target.value)}
              {...describedBy("al-first", show(errors.first))}
            />
          </Field>
          <Field id="al-label" label={<>Label <span className="font-normal text-muted">(optional)</span></>} hint="Also the memo the agent sees.">
            <input
              id="al-label"
              className={cx(inputClass, "h-11")}
              autoComplete="off"
              maxLength={ALLOWANCE_LABEL_MAX_CHARS}
              placeholder="Research agent"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              {...describedBy("al-label", undefined)}
            />
          </Field>
        </div>
      </Card>

      <aside className="space-y-4 lg:sticky lg:top-6 lg:self-start">
        <Card className="px-5 py-5 sm:px-6">
          <h2 className="text-base font-bold">Summary</h2>
          {plan ? (
            <dl className="mt-3 space-y-2.5 text-sm">
              <div className="flex justify-between gap-3">
                <dt className="text-muted">Each chip</dt>
                <dd><Zec zat={plan.chipZat} /></dd>
              </div>
              <div className="flex justify-between gap-3">
                <dt className="text-muted">One chip every</dt>
                <dd className="tabular-nums">{formatDuration(plan.everySecs)}</dd>
              </div>
              <div className="flex justify-between gap-3">
                <dt className="text-muted">Over {nPeriods} {nPeriods === 1 ? "period" : "periods"}</dt>
                <dd><Zec zat={plan.chipZat * plan.count} /></dd>
              </div>
              <div className="flex justify-between gap-3 border-t border-border pt-2.5 font-medium">
                <dt>Fund now</dt>
                <dd><Zec zat={fundingZat(plan.chipZat, plan.window)} /></dd>
              </div>
              <p className="text-xs text-muted text-pretty">
                One period at a time, fees included. Later periods are split from your treasury as chips are paid.
              </p>
            </dl>
          ) : (
            <p className="mt-2 text-sm text-muted">Fix the highlighted fields to see the plan.</p>
          )}
        </Card>
        {apiError && (
          <Notice tone="danger">
            <strong>Couldn’t create the allowance.</strong> {apiError}
          </Notice>
        )}
        <Button type="submit" variant="primary" className="w-full" disabled={busy || (touched && !ok)}>
          {busy ? "Creating…" : "Create allowance"}
        </Button>
        <p className="text-xs text-muted text-pretty">
          Chips are pre-signed and released by the keeper while this app is closed. The agent can never take more than the
          chips already released.
        </p>
      </aside>
    </form>
  );
}

export function Allowances() {
  const info = useInfo();
  const now = useNow();
  const list = usePoll(() => agents.list(), 10_000);

  return (
    <div>
      <PageHeader
        title="Agent allowances"
        subtitle="Give an AI agent’s own wallet up to a fixed amount per period, released in small chips. Stop it any time."
        action={
          <ButtonLink to="/requests" size="sm">
            <Bot className="size-4" aria-hidden />
            Agent requests
          </ButtonLink>
        }
      />
      {list.error && !list.data ? (
        <div className="mb-8">
          <ErrorPanel error={list.error} onRetry={() => void list.refresh()} />
        </div>
      ) : !list.data ? (
        <Skeleton className="mb-8 h-48 w-full" />
      ) : list.data.length > 0 ? (
        <div className="mb-8 space-y-4">
          {list.data.map((a) => (
            <AllowanceCard key={a.id} a={a} now={now} onChange={() => void list.refresh()} />
          ))}
        </div>
      ) : null}

      <h2 className="mb-3 font-display text-lg font-bold">New allowance</h2>
      {info.data ? (
        <CreateForm network={info.data.network} />
      ) : info.error ? (
        <ErrorPanel error={info.error} onRetry={() => void info.refresh()} />
      ) : (
        <Skeleton className="h-64 w-full" />
      )}
      <p className="mt-6 text-xs text-muted text-pretty">
        Each chip pays its own network fee ({formatZec(10_000)} ZEC): fewer, larger chips cost less, while more, smaller
        chips mean less is already in the agent’s hands when you stop.
      </p>
    </div>
  );
}
