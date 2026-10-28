import { useState } from "react";
import { Link } from "react-router";
import { CircleCheck, PlugZap, TriangleAlert, X } from "lucide-react";
import type { AppNotice } from "@promissory/client";
import { notices } from "../api";
import { usePoll } from "../hooks";
import { cx } from "./ui";

const TONE = {
  ok: "bg-ok-soft text-ok",
  warn: "bg-warn-soft text-warn",
  danger: "bg-danger-soft text-danger",
} as const;

function Icon({ n }: { n: AppNotice }) {
  const c = "size-4";
  if (n.kind === "keeper_unreachable") return <PlugZap className={c} aria-hidden />;
  if (n.kind === "renewal_paid") return <CircleCheck className={c} aria-hidden />;
  return <TriangleAlert className={c} aria-hidden />;
}

/** Payer-side banners from `GET /notices`. Silent when promissory-app is unreachable or older. */
export function NoticeBanners() {
  const poll = usePoll(() => notices.list(), 10_000);
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const list = (poll.data ?? []).filter((n) => !hidden.has(n.id)).slice(0, 4);
  if (list.length === 0) return null;

  return (
    <div className="mb-6 space-y-2" role="region" aria-label="Notices">
      {list.map((n) => (
        <div
          key={n.id}
          role={n.tone === "danger" ? "alert" : "status"}
          className={cx("anim-in flex items-start gap-3 rounded-2xl px-4 py-3 text-sm", TONE[n.tone])}
        >
          <span className="mt-0.5 shrink-0">
            <Icon n={n} />
          </span>
          <div className="min-w-0 flex-1 text-pretty">
            <strong className="font-semibold">{n.title}.</strong> <span className="text-fg/80">{n.message}</span>
            {n.kind === "switch_due" ? (
              <>
                {" "}
                <Link to="/switch" className="font-medium underline underline-offset-2">
                  Renew now
                </Link>
              </>
            ) : n.order_id !== null && (
              <>
                {" "}
                <Link to={`/orders/${n.order_id}`} className="font-medium underline underline-offset-2">
                  View order
                </Link>
              </>
            )}
          </div>
          {n.dismissible && (
            <button
              type="button"
              aria-label={`Dismiss: ${n.title}`}
              onClick={() => {
                setHidden((h) => new Set(h).add(n.id));
                void notices.dismiss(n.id).catch(() => undefined);
              }}
              className="-my-1 -mr-1.5 inline-flex size-7 shrink-0 items-center justify-center rounded-xl opacity-70 hover:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <X className="size-4" aria-hidden />
            </button>
          )}
        </div>
      ))}
    </div>
  );
}
