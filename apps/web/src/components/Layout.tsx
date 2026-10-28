import { useEffect } from "react";
import { Link, NavLink, Outlet, useLocation } from "react-router";
import { isMock } from "../api";
import { useInfo } from "../info";
import { Wordmark } from "./Logo";
import { NoticeBanners } from "./NoticeBanners";
import { cx, NetworkBadge } from "./ui";

const NAV = [
  { to: "/orders", label: "Orders", match: (p: string) => p === "/orders" || p === "/new" || p.startsWith("/orders/") },
  { to: "/payroll", label: "Payroll", match: (p: string) => p === "/payroll" || p.startsWith("/payroll/") },
  { to: "/subscribe", label: "Subscribe link", match: (p: string) => p === "/subscribe" || p === "/s" },
  { to: "/switch", label: "Switch", match: (p: string) => p === "/switch" },
  {
    to: "/allowances",
    label: "Agent allowances",
    match: (p: string) => p === "/allowances" || p === "/requests" || p.startsWith("/requests/"),
  },
  { to: "/payee", label: "Payee status", match: (p: string) => p === "/payee" },
  { to: "/receipt", label: "Receipts", match: (p: string) => p === "/receipt" },
  { to: "/privacy", label: "Who sees what", match: (p: string) => p === "/privacy" },
];

export function Layout() {
  const { data: info } = useInfo();
  const { pathname } = useLocation();
  // A block body: newer browsers return a Promise from scrollTo, and an effect's return value is
  // called as its cleanup.
  useEffect(() => {
    window.scrollTo(0, 0);
  }, [pathname]);

  return (
    <div className="flex min-h-dvh flex-col">
      <a
        href="#main"
        className="sr-only focus:not-sr-only focus:fixed focus:left-4 focus:top-4 focus:z-50 focus:rounded-xl focus:bg-surface focus:px-3 focus:py-2 focus:shadow"
      >
        Skip to content
      </a>
      <header className="sticky top-0 z-30 border-b border-border/70 bg-surface/75 backdrop-blur-md supports-[backdrop-filter]:bg-surface/60">
        <div className="mx-auto flex max-w-5xl flex-wrap items-center gap-x-6 gap-y-2 px-4 py-3 sm:px-6">
          <Link to="/" aria-label="Promissory home" className="rounded-xl focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
            <Wordmark />
          </Link>
          <div className="ml-auto flex items-center gap-2">
            {isMock && (
              <span className="inline-flex h-7 items-center rounded-full border border-dashed border-border-strong px-3 text-xs text-muted">
                mock data
              </span>
            )}
            {info && <NetworkBadge network={info.network} />}
          </div>
          <nav aria-label="Main" className="-mx-3 flex w-[calc(100%+1.5rem)] gap-1 overflow-x-auto pb-0.5">
            {NAV.map((n) => (
              <NavLink
                key={n.to}
                to={n.to}
                aria-current={n.match(pathname) ? "page" : undefined}
                className={() =>
                  cx(
                    "inline-flex h-9 items-center rounded-xl px-3 text-[15px] whitespace-nowrap transition-colors",
                    "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                    n.match(pathname) ? "bg-accent-soft font-semibold text-fg" : "text-muted hover:bg-surface-2/70 hover:text-fg",
                  )
                }
              >
                {n.label}
              </NavLink>
            ))}
          </nav>
        </div>
      </header>
      <main id="main" className="mx-auto w-full max-w-5xl flex-1 px-4 py-8 sm:px-6 sm:py-12">
        {pathname !== "/payee" && pathname !== "/s" && pathname !== "/receipt" && <NoticeBanners />}
        <Outlet />
      </main>
      <footer className="border-t border-border">
        <div className="mx-auto flex max-w-5xl flex-wrap items-center justify-between gap-3 px-4 py-6 text-sm text-muted sm:px-6">
          <p>
            Runs on your device. Keys stay in <code className="font-mono text-[0.92em]">promissory-app</code>; this page only talks to it.
          </p>
          <Link to="/" className="font-medium text-dawn-ink underline-offset-4 hover:underline">
            About Promissory
          </Link>
        </div>
      </footer>
    </div>
  );
}
