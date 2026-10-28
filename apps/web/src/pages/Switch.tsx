import { useState, type FormEvent } from "react";
import { Link, useNavigate } from "react-router";
import { Fingerprint, HeartPulse, Hourglass, RotateCw, ShieldCheck, TriangleAlert } from "lucide-react";
import {
  checkPayeeAddress,
  formatCountdown,
  formatZec,
  parseZec,
  type Order,
  type RenewResult,
} from "@promissory/client";
import { api } from "../api";
import { ErrorPanel } from "../components/ErrorPanel";
import { StatusChip } from "../components/StatusChip";
import {
  Address,
  Button,
  Card,
  CardHeader,
  cx,
  describedBy,
  Field,
  inputClass,
  Notice,
  PageHeader,
  Skeleton,
  Zec,
} from "../components/ui";
import { useNow, usePoll } from "../hooks";
import { useInfo } from "../info";
import { DAY, liveSwitches, pastSwitches, RENEW_PRESETS, renewError, renewTarget, switchPhase, tooLateToRenew, type SwitchPhase } from "../lib/switch";
import { formatDateTime, fromLocalInput, toLocalInput } from "../lib/time";

const HOW = [
  {
    icon: <ShieldCheck className="size-4" aria-hidden />,
    title: "You reserve it",
    body: "The amount moves into an account of its own and one payment to your heir is signed now, then locked to a date with drand.",
  },
  {
    icon: <HeartPulse className="size-4" aria-hidden />,
    title: "You check in",
    body: "Before the date, press Renew. The reserved funds move to a new switch with a later date in one transaction, and the old signed payment can never pay.",
  },
  {
    icon: <Hourglass className="size-4" aria-hidden />,
    title: "If you don’t",
    body: "On the date the keeper opens the payment and broadcasts it. Your heir is paid with nobody’s help: no guardians, no custodian, your wallet can be closed for good.",
  },
];

function Fingerprinted() {
  return (
    <Notice tone="warn" icon={<Fingerprint className="size-4" aria-hidden />}>
      <strong>This payment can be recognised on chain as pre-signed.</strong> A switch may fire months from now with you
      not there to re-sign it, so its transaction never expires (expiry height 0). Ordinary wallets always set an expiry,
      so this one stands out. Sender, heir and amount stay hidden.
    </Notice>
  );
}

const PHASE: Record<SwitchPhase, { label: string; tone: string }> = {
  unfunded: { label: "Waiting for funds", tone: "text-warn" },
  arming: { label: "Reserving and signing", tone: "text-dawn-ink" },
  armed: { label: "Armed", tone: "text-ok" },
  due_soon: { label: "Check in soon", tone: "text-warn" },
  firing: { label: "Due: the keeper is releasing it", tone: "text-danger" },
};

function RenewPanel({ order, now, onRenewed }: { order: Order; now: number; onRenewed: (r: RenewResult) => void }) {
  const [custom, setCustom] = useState(false);
  const [value, setValue] = useState(() => toLocalInput(renewTarget(order, now, 30 * DAY)));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const id = `renew-${order.id}`;

  async function renew(at: number | null) {
    const local = renewError(order, at, now);
    if (local || at === null) {
      setError(local ?? "Choose a date");
      return;
    }
    setBusy(true);
    setError(undefined);
    try {
      onRenewed(await api.renewSwitch(order.id, { at }));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  const funded = order.status !== "awaiting_funds";
  const late = tooLateToRenew(order, now);
  return (
    <div className="space-y-3">
      {late && (
        <p className="text-sm text-warn text-pretty">
          Too late to renew: it pays in under 10 minutes, and a renewal could race the payment.
        </p>
      )}
      <Button
        variant="primary"
        className="h-14 w-full text-base"
        disabled={busy || late}
        onClick={() => void renew(renewTarget(order, now, 30 * DAY))}
      >
        <RotateCw className={cx("size-5", busy && "animate-spin motion-reduce:animate-none")} aria-hidden />
        {busy ? "Renewing…" : "I’m still here: renew for 30 days"}
      </Button>
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <span className="text-muted">Or renew for</span>
        {RENEW_PRESETS.slice(1).map((p) => (
          <Button key={p.label} size="sm" disabled={busy} onClick={() => void renew(renewTarget(order, now, p.secs))}>
            {p.label}
          </Button>
        ))}
        <Button size="sm" variant="ghost" aria-expanded={custom} aria-controls={id} onClick={() => setCustom((c) => !c)}>
          Pick a date…
        </Button>
      </div>
      {custom && (
        <div id={id} className="flex flex-wrap items-end gap-2">
          <div className="min-w-0 flex-1">
            <Field id={`${id}-at`} label="New date" error={undefined}>
              <input
                id={`${id}-at`}
                type="datetime-local"
                className={cx(inputClass, "h-11")}
                value={value}
                onChange={(e) => setValue(e.currentTarget.value)}
              />
            </Field>
          </div>
          <Button disabled={busy} onClick={() => void renew(fromLocalInput(value))}>
            Renew to this date
          </Button>
        </div>
      )}
      <p className="text-xs text-muted text-pretty">
        {funded
          ? "One transaction moves the reserved funds to a new switch (the network fee, 0.0001 ZEC, comes from your treasury). The old signed payment then fails as a double-spend."
          : "Not funded yet, so nothing moves on chain: the switch is simply re-dated."}
      </p>
      {error && (
        <Notice tone="danger" icon={<TriangleAlert className="size-4" aria-hidden />}>
          <strong>Could not renew.</strong> {error}
        </Notice>
      )}
    </div>
  );
}

function SwitchCard({ order, now, onRenewed }: { order: Order; now: number; onRenewed: (r: RenewResult) => void }) {
  const phase = switchPhase(order, now);
  const left = order.first_due - now;
  return (
    <Card className="mb-6 overflow-hidden">
      <CardHeader
        title={
          <span className="flex flex-wrap items-center gap-2">
            <Link to={`/orders/${order.id}`} className="hover:underline">
              Switch #{order.id}
            </Link>
            <StatusChip status={order.status} renewed={!!order.renewed_to} />
          </span>
        }
        action={
          <Link to={`/orders/${order.id}`} className="text-sm font-medium text-dawn-ink underline-offset-2 hover:underline">
            Details
          </Link>
        }
      >
        <span className="inline-flex flex-wrap items-center gap-x-1.5">
          Pays <Zec zat={order.amount_zat} className="font-medium text-fg" /> to
          <Address value={order.payee} label="Copy heir address" className="text-fg" />
        </span>
      </CardHeader>
      <div className="grid gap-6 px-5 py-6 sm:grid-cols-[minmax(0,1fr)_minmax(0,1.3fr)] sm:px-6">
        <div className="min-w-0">
          <p className={cx("text-sm font-medium", PHASE[phase].tone)}>{PHASE[phase].label}</p>
          <p className="mt-2 text-sm text-muted">Pays your heir in</p>
          <p className="font-mono text-4xl font-medium tracking-tight tabular-nums">
            <time dateTime={new Date(order.first_due * 1000).toISOString()}>{left > 0 ? formatCountdown(left) : "now"}</time>
          </p>
          <p className="mt-1 text-sm text-muted">on {formatDateTime(order.first_due)}</p>
          {order.renewed_from != null && (
            <p className="mt-2 text-xs text-muted">
              Renewed from{" "}
              <Link to={`/orders/${order.renewed_from}`} className="underline underline-offset-2">
                switch #{order.renewed_from}
              </Link>
            </p>
          )}
          {phase === "unfunded" && (
            <p className="mt-3 text-sm">
              <Link to={`/orders/${order.id}`} className="font-medium text-dawn-ink underline underline-offset-2">
                Fund it
              </Link>{" "}
              to arm it.
            </p>
          )}
        </div>
        {phase === "firing" ? (
          <Notice tone="danger">
            <strong>Too late to renew.</strong> Its date has passed; the keeper releases the payment to your heir.
          </Notice>
        ) : (
          <RenewPanel order={order} now={now} onRenewed={onRenewed} />
        )}
      </div>
    </Card>
  );
}

function CreateSwitch() {
  const navigate = useNavigate();
  const info = useInfo();
  const [heir, setHeir] = useState("");
  const [amount, setAmount] = useState("");
  const [at, setAt] = useState(() => toLocalInput(Math.floor(Date.now() / 1000) + 90 * DAY));
  const [remindDays, setRemindDays] = useState("7");
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [failure, setFailure] = useState<string>();
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    const errs: Record<string, string> = {};
    const net = info.data?.network ?? "main";
    try {
      checkPayeeAddress(heir.trim(), net);
    } catch (err) {
      errs.heir = err instanceof Error ? err.message : "Not a valid address";
    }
    let zat = 0;
    try {
      zat = parseZec(amount);
      if (zat <= 0) errs.amount = "Enter an amount above zero";
    } catch (err) {
      errs.amount = err instanceof Error ? err.message : "Not a ZEC amount";
    }
    const when = fromLocalInput(at);
    const now = Math.floor(Date.now() / 1000);
    if (when === null || when < now + 600) errs.at = "Choose a date at least 10 minutes from now";
    const days = Number(remindDays);
    if (!Number.isFinite(days) || days < 0 || days > 365) errs.remind = "Between 0 and 365 days";
    setErrors(errs);
    if (Object.keys(errs).length > 0 || when === null) return;
    setBusy(true);
    setFailure(undefined);
    try {
      const o = await api.createSwitch({
        heir: heir.trim(),
        amount_zat: zat,
        at: when,
        remind_secs: Math.round(days * DAY),
        memo: "Inheritance",
      });
      navigate(`/orders/${o.id}`);
    } catch (err) {
      setFailure(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card className="mb-6">
      <CardHeader title="New switch">One payment to your heir on a date, unless you check in first.</CardHeader>
      <form className="space-y-5 px-5 py-5 sm:px-6" onSubmit={(e) => void submit(e)} noValidate>
        <Field id="heir" label="Heir’s address" hint="A unified address with an Orchard receiver (Zodl gives one)." error={errors.heir}>
          <input
            id="heir"
            className={cx(inputClass, "h-11 font-mono")}
            value={heir}
            onChange={(e) => setHeir(e.currentTarget.value)}
            autoComplete="off"
            spellCheck={false}
            {...describedBy("heir", errors.heir)}
          />
        </Field>
        <div className="grid gap-5 sm:grid-cols-3">
          <Field id="amount" label="Amount (ZEC)" error={errors.amount} hint="Reserved now, until it pays or you cancel.">
            <input
              id="amount"
              inputMode="decimal"
              className={cx(inputClass, "h-11 tabular-nums")}
              value={amount}
              onChange={(e) => setAmount(e.currentTarget.value)}
              placeholder="1.5"
              {...describedBy("amount", errors.amount)}
            />
          </Field>
          <Field id="at" label="Pays on" error={errors.at} hint="Unless you renew before then.">
            <input
              id="at"
              type="datetime-local"
              className={cx(inputClass, "h-11")}
              value={at}
              onChange={(e) => setAt(e.currentTarget.value)}
              {...describedBy("at", errors.at)}
            />
          </Field>
          <Field id="remind" label="Remind me (days before)" error={errors.remind} hint="A “check in” notice in this app.">
            <input
              id="remind"
              inputMode="numeric"
              className={cx(inputClass, "h-11 tabular-nums")}
              value={remindDays}
              onChange={(e) => setRemindDays(e.currentTarget.value)}
              {...describedBy("remind", errors.remind)}
            />
          </Field>
        </div>
        <Fingerprinted />
        {failure && (
          <Notice tone="danger">
            <strong>Could not create the switch.</strong> {failure}
          </Notice>
        )}
        <div className="flex justify-end">
          <Button type="submit" variant="primary" disabled={busy}>
            {busy ? "Creating…" : "Create switch"}
          </Button>
        </div>
      </form>
    </Card>
  );
}

export function Switch() {
  const poll = usePoll(() => api.listOrders(), 5_000);
  const now = useNow();
  const [renewed, setRenewed] = useState<RenewResult | null>(null);
  const live = poll.data ? liveSwitches(poll.data) : [];
  const past = poll.data ? pastSwitches(poll.data) : [];

  return (
    <div>
      <PageHeader
        title="Dead-man’s switch"
        subtitle="Leave ZEC to someone if you stop checking in. No guardians, no custodian, no contract."
      />

      <Card className="mb-6">
        <ol className="grid gap-5 px-5 py-5 sm:grid-cols-3 sm:px-6">
          {HOW.map((h, i) => (
            <li key={h.title} className="min-w-0">
              <p className="flex items-center gap-2 text-sm font-semibold">
                <span className="flex size-7 items-center justify-center rounded-full bg-accent-soft text-accent">{h.icon}</span>
                <span>
                  <span className="sr-only">Step {i + 1}: </span>
                  {h.title}
                </span>
              </p>
              <p className="mt-1.5 text-sm text-muted text-pretty">{h.body}</p>
            </li>
          ))}
        </ol>
      </Card>

      {renewed && (
        <div className="mb-6">
          <Notice tone="ok" icon={<ShieldCheck className="size-4" aria-hidden />}>
            <strong>Renewed.</strong> Switch #{renewed.old_id} is replaced by{" "}
            <Link to={`/orders/${renewed.new_id}`} className="font-medium underline underline-offset-2">
              switch #{renewed.new_id}
            </Link>
            , due {formatDateTime(renewed.order.first_due)}.{" "}
            {renewed.txid
              ? "The old signed payment can no longer pay: its funds moved in the renewal transaction."
              : "Nothing had been funded, so nothing moved on chain."}
          </Notice>
        </div>
      )}

      {poll.error && !poll.data ? (
        <div className="mb-6">
          <ErrorPanel error={poll.error} onRetry={() => void poll.refresh()} />
        </div>
      ) : !poll.data ? (
        <Skeleton className="mb-6 h-56 w-full" />
      ) : (
        live.map((o) => (
          <SwitchCard
            key={o.id}
            order={o}
            now={now}
            onRenewed={(r) => {
              setRenewed(r);
              void poll.refresh();
            }}
          />
        ))
      )}

      <CreateSwitch />

      {past.length > 0 && (
        <Card>
          <CardHeader title="Finished switches" />
          <ul className="divide-y divide-border">
            {past.map((o) => (
              <li key={o.id} className="flex flex-wrap items-center justify-between gap-3 px-5 py-3 text-sm sm:px-6">
                <Link to={`/orders/${o.id}`} className="font-medium hover:underline">
                  Switch #{o.id}
                </Link>
                <span className="text-muted">
                  {formatZec(o.amount_zat)} ZEC · {formatDateTime(o.first_due)}
                </span>
                <StatusChip status={o.status} renewed={!!o.renewed_to} />
              </li>
            ))}
          </ul>
        </Card>
      )}
    </div>
  );
}

/** Shown on a switch's order page. */
export function SwitchNotice({ order }: { order: Order }) {
  return (
    <div className="space-y-2">
      <Notice tone="accent" icon={<HeartPulse className="size-4" aria-hidden />}>
        <strong>Dead-man’s switch.</strong>{" "}
        {order.status === "completed" ? (
          <>It paid your heir on {formatDateTime(order.first_due)}.</>
        ) : order.renewed_to == null && (order.status === "cancelled" || order.status === "cancelling") ? (
          <>Cancelled: its funds were swept back to your treasury.</>
        ) : order.renewed_to != null ? (
          <>
            Renewed as{" "}
            <Link to={`/orders/${order.renewed_to}`} className="font-medium underline underline-offset-2">
              switch #{order.renewed_to}
            </Link>
            ; this one can no longer pay.
          </>
        ) : (
          <>
            Pays your heir on {formatDateTime(order.first_due)} unless you{" "}
            <Link to="/switch" className="font-medium underline underline-offset-2">
              renew it
            </Link>
            .
          </>
        )}
      </Notice>
      {order.renewed_to == null && order.status !== "completed" && order.status !== "cancelled" && <Fingerprinted />}
    </div>
  );
}
