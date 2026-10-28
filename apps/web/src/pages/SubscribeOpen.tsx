import { useMemo, useState, type ReactNode } from "react";
import { Link, useLocation, useNavigate } from "react-router";
import { CircleAlert, ShieldCheck } from "lucide-react";
import {
  formatCadence,
  formatZec,
  LinkError,
  NetworkError,
  parseSubscribeLink,
  PAYER_DEFAULT_URL,
  subscribeTotalZat,
  truncateMiddle,
  type SubscribeParams,
} from "@promissory/client";
import { api } from "../api";
import { Address, Button, ButtonLink, buttonClass, Card, CopyButton, NetworkBadge, Notice, PageHeader, Skeleton } from "../components/ui";
import { useInfo } from "../info";
import { formatDateTime, hostOf } from "../lib/time";

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid gap-1 py-3 sm:grid-cols-[9rem_minmax(0,1fr)] sm:gap-4">
      <dt className="text-sm text-muted">{label}</dt>
      <dd className="min-w-0 text-sm">{children}</dd>
    </div>
  );
}

function Setup({ p }: { p: SubscribeParams }) {
  const info = useInfo();
  const navigate = useNavigate();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  // Off by default: sending the payee's webhook tells the keeper who gets paid.
  const [acceptHook, setAcceptHook] = useState(false);

  if (!info.data && !info.error) return <Skeleton className="h-10 w-56" />;

  if (!info.data) {
    const offline = info.error instanceof NetworkError;
    const local = `${PAYER_DEFAULT_URL}/s${window.location.hash}`;
    return (
      <Card className="px-5 py-5 sm:px-6">
        <h2 className="text-base font-bold">{offline ? "Open this link in your Promissory app" : "The Promissory app reported an error"}</h2>
        {offline ? (
          <ol className="mt-3 list-decimal space-y-2 pl-5 text-sm text-muted marker:text-muted">
            <li>
              Start Promissory on this device (<code className="font-mono text-fg">promissory-app</code>). It holds your keys and signs
              the payments.
            </li>
            <li>
              Open this link there:
              <div className="mt-2 flex items-start gap-1 rounded-2xl border border-border bg-surface-2 py-1 pr-1 pl-3.5">
                <code className="min-w-0 flex-1 py-1.5 font-mono text-xs break-all text-fg">{local}</code>
                <CopyButton text={local} label="Copy local link" />
              </div>
            </li>
          </ol>
        ) : (
          <p className="mt-2 text-sm text-muted">{info.error instanceof Error ? info.error.message : String(info.error)}</p>
        )}
        <div className="mt-4 flex flex-wrap gap-2">
          {offline && (
            <a className={buttonClass("primary")} href={local}>
              Open in local app
            </a>
          )}
          <Button onClick={() => void info.refresh()}>Check again</Button>
        </div>
      </Card>
    );
  }

  if (info.data.network !== p.network) {
    return (
      <Notice tone="danger" icon={<CircleAlert className="size-4" aria-hidden />}>
        <strong>Wrong network.</strong> This link is for {p.network === "main" ? "mainnet" : "regtest"}, but your app runs on{" "}
        {info.data.network === "main" ? "mainnet" : "regtest"}.
      </Notice>
    );
  }

  async function setup() {
    setBusy(true);
    setError(undefined);
    try {
      const order = await api.createOrderFromLink({ link: window.location.href, accept_hook: acceptHook });
      navigate(`/orders/${order.id}`);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  }

  return (
    <div className="space-y-3">
      {error && (
        <Notice tone="danger">
          <strong>Couldn’t set it up.</strong> {error}
        </Notice>
      )}
      {p.hook && (
        <label className="flex items-start gap-2.5 text-sm text-pretty">
          <input
            type="checkbox"
            className="mt-0.5 size-4 accent-[var(--color-accent)]"
            checked={acceptHook}
            onChange={(e) => setAcceptHook(e.target.checked)}
          />
          <span>
            Send the payee notices through the keeper.{" "}
            <span className="text-muted">
              The keeper then learns the payee's notice address and which payments are theirs. Leave off unless the payee
              needs it.
            </span>
          </span>
        </label>
      )}
      <div className="flex flex-wrap items-center gap-3">
        <Button variant="primary" onClick={() => void setup()} disabled={busy}>
          {busy ? "Setting up…" : "Set up this standing order"}
        </Button>
        <span className="text-sm text-muted">Next: fund it once with a QR code.</span>
      </div>
    </div>
  );
}

export function SubscribeOpen() {
  const { hash } = useLocation();
  const parsed = useMemo(() => {
    if (!hash || hash === "#") return null;
    try {
      return { ok: true as const, p: parseSubscribeLink(hash) };
    } catch (e) {
      if (e instanceof LinkError) return { ok: false as const, error: e };
      throw e;
    }
  }, [hash]);

  if (!parsed) {
    return (
      <div>
        <PageHeader title="Subscribe link" subtitle="This page opens Promissory subscribe links. This one has no details in it." />
        <ButtonLink to="/subscribe" variant="primary">
          Make a subscribe link
        </ButtonLink>
      </div>
    );
  }

  if (!parsed.ok) {
    return (
      <div>
        <PageHeader title="This link can’t be used" subtitle="Ask the payee for a new one." />
        <Notice tone="danger" icon={<CircleAlert className="size-4" aria-hidden />}>
          <strong>
            Problem with <code className="font-mono">{parsed.error.param}</code>:
          </strong>{" "}
          {parsed.error.message}
        </Notice>
      </div>
    );
  }

  const p = parsed.p;
  const total = subscribeTotalZat(p);
  return (
    <div>
      <section className="mb-6 rounded-[28px] p-6 sm:p-8" style={{ background: "var(--hero)" }}>
        <div className="mb-3 flex items-center gap-2">
          <span className="text-sm font-medium text-fg/80">Subscribe request</span>
          <NetworkBadge network={p.network} />
        </div>
        <h1 className="text-[2.1rem] leading-[1.05] font-extrabold tracking-[-0.03em] text-balance sm:text-[2.6rem]">
          Pay <span className="tabular-nums">{formatZec(p.amountZat, { unit: true })}</span> to{" "}
          <span className="font-mono text-[0.8em] font-medium" title={p.payee}>
            {truncateMiddle(p.payee, 6, 4)}
          </span>{" "}
          {formatCadence(p.everySecs)}, {p.count} {p.count === 1 ? "time" : "times"}
        </h1>
        <p className="mt-4 text-base text-fg/80">
          Total{" "}
          <span className="font-display text-2xl font-extrabold tracking-tight text-fg tabular-nums">
            {formatZec(total, { unit: true })}
          </span>{" "}
          + fees
        </p>
      </section>

      <Card className="mb-6 px-5 sm:px-6">
        <dl className="divide-y divide-border">
          <Row label="Payee">
            <Address value={p.payee} head={12} tail={10} label="Copy payee address" />
          </Row>
          <Row label="Each payment">{formatZec(p.amountZat, { unit: true })}</Row>
          <Row label="Schedule">
            {p.count} payments, {formatCadence(p.everySecs)}
          </Row>
          <Row label="First payment">{p.start ? formatDateTime(p.start) : "When you fund and arm the order"}</Row>
          {p.memo && <Row label="Memo">“{p.memo}”</Row>}
          {p.keeper && (
            <Row label="Suggested keeper">
              <span className="font-mono">{hostOf(p.keeper)}</span>
            </Row>
          )}
          {p.hook && (
            <Row label="Notices">
              The payee asks for notices at <span className="font-mono">{hostOf(p.hook)}</span>. Only keepers that allow webhooks
              send them.
            </Row>
          )}
        </dl>
      </Card>

      <Setup p={p} />

      <div className="mt-8 flex gap-3 text-sm text-muted">
        <ShieldCheck className="mt-0.5 size-4 shrink-0" aria-hidden />
        <p className="text-pretty">
          You stay in control: payments come from your own wallet, and you can cancel at any time.{" "}
          <Link to="/privacy" className="font-medium text-dawn-ink underline-offset-2 hover:underline">
            Who sees what
          </Link>
        </p>
      </div>
    </div>
  );
}
