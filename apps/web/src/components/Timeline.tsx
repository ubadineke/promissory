import { Ban, Check, CircleAlert, Clock, Radio, Sparkles, TriangleAlert } from "lucide-react";
import { useEffect, useRef, type ReactNode } from "react";
import { formatCountdown, formatDuration, type Installment, type OrderDetail } from "@promissory/client";
import { hasFreshCopy, installmentPhase, isUnlocked, releaseAt, type InstallmentPhase } from "../lib/phase";
import { formatWhen } from "../lib/time";
import { ReceiptButton } from "./ReceiptButton";
import { Address, cx, Ring } from "./ui";

type Tone = "muted" | "accent" | "ok" | "warn" | "danger";

const ICON_TONE: Record<Tone, string> = {
  muted: "bg-surface-2 text-muted border-border",
  accent: "bg-dawn-soft text-dawn-ink border-transparent",
  ok: "bg-ok-soft text-ok border-transparent",
  warn: "bg-warn-soft text-warn border-transparent",
  danger: "bg-danger-soft text-danger border-transparent",
};

interface View {
  icon: ReactNode;
  tone: Tone;
  title: ReactNode;
  detail?: ReactNode;
}

const ic = "size-[18px]";

/** A padlock whose shackle can swing open; the `unlock` class animates it. */
function Padlock({ open = false }: { open?: boolean }) {
  return (
    <svg viewBox="0 0 24 24" className={ic} fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="5" y="11" width="14" height="10" rx="2.5" />
      <path className="shackle" d={open ? "M8 11V7a4 4 0 0 1 7.8-1.3" : "M8 11V7a4 4 0 0 1 8 0v4"} />
    </svg>
  );
}

function view(p: InstallmentPhase): View {
  switch (p.kind) {
    case "reserved":
      return { icon: <Padlock />, tone: "muted", title: "Reserved", detail: "Signed and locked when the order is armed" };
    case "locked":
      return { icon: <Padlock />, tone: "accent", title: "Locked", detail: "Encrypted to its drand round; nobody can open it early" };
    case "releasing":
      return { icon: <Padlock open />, tone: "accent", title: "Unlocking", detail: "Its drand round is out; waiting for the keeper" };
    case "released":
      return { icon: <Padlock open />, tone: "accent", title: "Released by the keeper", detail: "Broadcasting now" };
    case "broadcast":
      return { icon: <Radio className={ic} />, tone: "accent", title: "Broadcast", detail: "Waiting to be mined" };
    case "paid":
      return {
        icon: <Check className={ic} strokeWidth={2.4} />,
        tone: "ok",
        title: p.height !== null ? `Paid at block ${p.height.toLocaleString()}` : "Paid",
      };
    case "cancelled":
      return { icon: <Ban className={ic} />, tone: "muted", title: "Cancelled", detail: "Its funds were swept back; the keeper’s copy fails as a double-spend" };
    case "may_still_pay":
      return p.why === "released"
        ? {
            icon: <Padlock open />,
            tone: "warn",
            title: "May still pay: already released by the keeper",
            detail: "Once the keeper has broadcast an installment, cancel cannot stop it",
          }
        : {
            icon: <TriangleAlert className={ic} />,
            tone: "warn",
            title: "May still pay: inside the race window",
            detail: "Due too soon for the sweep to be sure to land first",
          };
    case "expired":
      return { icon: <Clock className={ic} />, tone: "muted", title: "Expired", detail: "The chain no longer accepts this copy" };
    case "failed":
      return { icon: <CircleAlert className={ic} />, tone: "danger", title: "Failed", detail: "The keeper could not broadcast it" };
    case "withdrawn":
      return { icon: <Ban className={ic} />, tone: "muted", title: "Withdrawn from the keeper" };
  }
}

function Row({ inst, order, now, isNext }: { inst: Installment; order: OrderDetail; now: number; isNext: boolean }) {
  const phase = installmentPhase(inst, order.status, now);
  const v = view(phase);
  const release = releaseAt(inst);
  const remaining = release - now;
  const showCountdown =
    phase.kind === "locked" ||
    (phase.kind === "reserved" && remaining < 86_400) || (phase.kind === "may_still_pay" && phase.why === "race_window");
  // Show the copy that actually went out: the fresh one when it is ahead.
  const txid = inst.fresh_txid && inst.fresh_keeper_state && inst.fresh_keeper_state !== "deleted" && inst.fresh_keeper_state !== "expired" ? inst.fresh_txid : inst.txids[0];
  const showTx = txid && (phase.kind === "paid" || phase.kind === "broadcast" || phase.kind === "released" || phase.kind === "may_still_pay");

  // Animate the padlock only when it opens while the page is on screen, not on every load.
  const unlocked = isUnlocked(phase);
  const wasUnlocked = useRef(unlocked);
  const justOpened = unlocked && !wasUnlocked.current;
  useEffect(() => {
    wasUnlocked.current = unlocked;
  }, [unlocked]);

  const period = order.every_secs > 0 ? order.every_secs : 86_400;
  const current = isNext && remaining > 0;

  return (
    <li
      className={cx(
        "flex gap-4 px-5 py-4 sm:px-6",
        current && "bg-gradient-to-r from-dawn-soft to-transparent",
      )}
    >
      {current ? (
        <Ring value={1 - remaining / period} size={40} thickness={3} className="z-10 text-fg">
          <Padlock />
        </Ring>
      ) : (
        <span
          aria-hidden
          className={cx(
            "relative z-10 flex size-10 shrink-0 items-center justify-center rounded-full border transition-colors duration-300",
            phase.kind === "reserved" ? "border-2 border-dashed border-border-strong bg-surface text-muted" : ICON_TONE[v.tone],
            justOpened && "unlock glow",
          )}
        >
          {v.icon}
        </span>
      )}
      <div className="min-w-0 flex-1 pt-1.5">
        <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
          <h3 className="text-[15px] font-bold">
            <span className="mr-2 font-mono font-medium text-muted tabular-nums">#{inst.k + 1}</span>
            {v.title}
          </h3>
          {showCountdown && remaining > 0 ? (
            <span className="text-sm text-muted">
              opens in{" "}
              <time
                className="inline-block min-w-[5ch] text-right font-mono text-base font-medium text-fg tabular-nums"
                dateTime={new Date(release * 1000).toISOString()}
                aria-label={`opens in ${formatCountdown(remaining)}`}
              >
                {formatCountdown(remaining)}
              </time>
            </span>
          ) : null}
        </div>
        <p className="mt-0.5 text-sm text-muted text-pretty">
          {v.detail ? <>{v.detail} · </> : null}
          due <span className="tabular-nums">{formatWhen(inst.due_unix)}</span>
        </p>
        {hasFreshCopy(inst) && (phase.kind === "locked" || phase.kind === "releasing") && (
          <p className="mt-1 flex items-center gap-1.5 text-xs font-medium text-dawn-ink">
            <Sparkles aria-hidden className="size-3.5" />
            Fresh copy signed while you were online: it pays {formatDuration(inst.release_unix - releaseAt(inst))} early and looks like a normal send
          </p>
        )}
        {showTx && (
          <p className="mt-1 flex items-center gap-1 text-xs text-muted">
            tx <Address value={txid} head={10} tail={8} label={`Copy transaction ID of payment ${inst.k + 1}`} />
          </p>
        )}
        {phase.kind === "paid" && <ReceiptButton orderId={order.id} k={inst.k} />}
      </div>
    </li>
  );
}

export function Timeline({ order, now }: { order: OrderDetail; now: number }) {
  // Only the next payment to open gets the ring and the highlight.
  const next = order.installments.find((i) => installmentPhase(i, order.status, now).kind === "locked");
  return (
    <ol className="divide-y divide-border/60 py-1" aria-label="Payments">
      {order.installments.map((inst) => (
        <Row key={inst.k} inst={inst} order={order} now={now} isNext={inst.k === next?.k} />
      ))}
    </ol>
  );
}
