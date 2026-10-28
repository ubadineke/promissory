import { useEffect, useRef, useState, type ReactNode } from "react";
import { Link } from "react-router";
import { ArrowDown, ArrowRight, Bot, Briefcase, HeartPulse, Home, Repeat } from "lucide-react";
import { Wordmark } from "../components/Logo";
import { ButtonLink, cx } from "../components/ui";
import type { DialTick } from "../landing/dialScene";

function prefersReducedMotion(): boolean {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** The hero object. The CSS dial paints first; the WebGL one fades in over it when it is ready. */
function HeroDial() {
  const host = useRef<HTMLDivElement>(null);
  const [ready, setReady] = useState(false);
  const [tick, setTick] = useState<DialTick>({ remaining: 9, paid: false });

  useEffect(() => {
    let dispose: (() => void) | null = null;
    let cancelled = false;
    void import("../landing/dialScene").then(({ mountDial }) => {
      if (cancelled || !host.current) return;
      dispose = mountDial(host.current, { onTick: setTick, still: prefersReducedMotion() });
      if (dispose) setReady(true);
    });
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, []);

  return (
    <div role="img" aria-label="A glass dial counting down to a payment, over a rising sun" className="relative mx-auto aspect-square w-full max-w-[34rem]">
      <div aria-hidden className={cx("absolute inset-0 transition-opacity duration-700", ready && "opacity-0")}>
        <div className="absolute inset-x-[8%] top-[34%] aspect-square rounded-full bg-[radial-gradient(circle,#ffe2b8_0%,#f2a65a_42%,rgb(242_166_90/0)_70%)]" />
        <div
          className="absolute inset-[14%] rounded-full"
          style={{
            background: "conic-gradient(var(--fg) 0deg 252deg, color-mix(in srgb, var(--fg) 14%, transparent) 252deg 360deg)",
            mask: "radial-gradient(circle, transparent 69%, #000 70%, #000 73%, transparent 74%)",
            WebkitMask: "radial-gradient(circle, transparent 69%, #000 70%, #000 73%, transparent 74%)",
          }}
        />
        <div className="glass absolute inset-[24%] rounded-full shadow-[0_30px_60px_-20px_rgb(20_33_61_/_0.35)]" />
      </div>
      {/* Fade the canvas out at its edges so the sun's glow never ends in a hard line. */}
      <div
        ref={host}
        className={cx(
          "absolute inset-0 transition-opacity duration-700 [mask-image:radial-gradient(closest-side,#000_78%,transparent)]",
          ready ? "opacity-100" : "opacity-0",
        )}
      />
      <div aria-hidden className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-1">
        <span className={cx("font-mono text-[2.6rem] leading-none font-medium tabular-nums [text-shadow:0_1px_14px_var(--bg)] sm:text-5xl", tick.paid && "font-display font-extrabold")}>
          {tick.paid ? "Paid" : `00:${String(tick.remaining).padStart(2, "0")}`}
        </span>
        <span className="text-sm font-semibold [text-shadow:0_1px_10px_var(--bg)]">{tick.paid ? "right on time" : "until it pays"}</span>
      </div>
    </div>
  );
}

/** Opens its padlock once, when scrolled into view: the release step. */
function UnlockingPadlock() {
  const ref = useRef<HTMLSpanElement>(null);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const io = new IntersectionObserver(
      ([e]) => {
        if (e?.isIntersecting) {
          setOpen(true);
          io.disconnect();
        }
      },
      { threshold: 0.8 },
    );
    io.observe(el);
    return () => {
      io.disconnect();
    };
  }, []);
  return (
    <span
      ref={ref}
      aria-hidden
      className={cx(
        "flex size-12 items-center justify-center rounded-full transition-colors duration-500",
        open ? "unlock glow bg-dawn text-ink" : "bg-surface-2 text-fg",
      )}
    >
      <svg viewBox="0 0 24 24" className="size-6" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <rect x="5" y="11" width="14" height="10" rx="2.5" />
        <path className="shackle" d={open ? "M8 11V7a4 4 0 0 1 7.8-1.3" : "M8 11V7a4 4 0 0 1 8 0v4"} />
      </svg>
    </span>
  );
}

function StepIcon({ n }: { n: number }) {
  return (
    <span aria-hidden className="flex size-12 items-center justify-center rounded-full border border-border-strong bg-surface font-mono text-base font-medium">
      {n}
    </span>
  );
}

const STEPS: { title: string; body: string; icon?: ReactNode }[] = [
  { title: "Reserve", body: "Your app sets aside one shielded note for each payment, so the money is there when it is due." },
  { title: "Sign", body: "Every payment is signed now, on your device, while you hold the keys." },
  { title: "Lock", body: "Each one is sealed to a future moment in time. Nobody can open it early, not even us." },
  { title: "Release", body: "When its moment comes, a keeper opens it and sends it. Your phone can be off.", icon: <UnlockingPadlock /> },
  { title: "Cancel", body: "Spend the reserved notes and every payment still waiting can never pay. The chain enforces it." },
];

const USES = [
  { to: "/subscribe", icon: Repeat, title: "Subscriptions", body: "A link your customers fund once." },
  { to: "/payroll", icon: Briefcase, title: "Payroll", body: "A CSV in, every salary on time." },
  { to: "/new?mode=home", icon: Home, title: "Send home", body: "Weekly money for family, shown in naira.", warm: true },
  { to: "/switch", icon: HeartPulse, title: "Dead-man’s switch", body: "Pays your heir unless you check in." },
  { to: "/allowances", icon: Bot, title: "Agent allowances", body: "An AI agent spends within a cap you set." },
];

const PARTIES = [
  { party: "Everyone watching the chain", sees: "Ordinary shielded transactions.", never: "Who paid whom, or how much." },
  { party: "The person you pay", sees: "The amount, the schedule, your memo, and each payment arriving.", never: "Your other money, or your other payments." },
  { party: "The keeper", sees: "Sealed blobs until each due time, then a transaction about to be public anyway.", never: "Your keys, the amount, the recipient, or who you are." },
  { party: "drand, the clock", sees: "That someone checks the time.", never: "Anything about your payments." },
];

const PROOF = [
  { label: "App closed", value: "3 payments paid", detail: "mined at blocks 224, 235 and 247" },
  { label: "Cancel", value: "The 4th never paid", detail: "rejected by the node: duplicate nullifier" },
  { label: "Network upgrade", value: "Signed before, paid after", detail: "NU7, branch 0x77190ad9, mined at 304" },
  { label: "Payroll", value: "9 salaries, one batch", detail: "all paid at blocks 987 to 991" },
];

const SNIPPET = `import { encodeSubscribeLink, parseZec } from "@promissory/client";

const link = encodeSubscribeLink("https://pay.example.com/s", {
  network: "main",
  payee: "u1…",
  amountZat: parseZec("0.01"),
  everySecs: 604_800, // weekly
  count: 12,
});`;

function Section({ id, eyebrow, title, intro, children }: { id: string; eyebrow: string; title: string; intro?: ReactNode; children: ReactNode }) {
  return (
    <section id={id} aria-labelledby={`${id}-title`} className="scroll-mt-8 py-16 sm:py-24">
      <p className="text-sm font-semibold text-dawn-ink">{eyebrow}</p>
      <h2 id={`${id}-title`} className="mt-2 max-w-3xl font-display text-[2.1rem] leading-[1.05] font-extrabold tracking-[-0.03em] text-balance sm:text-5xl">
        {title}
      </h2>
      {intro && <p className="mt-4 max-w-2xl text-lg text-muted text-pretty">{intro}</p>}
      <div className="mt-10">{children}</div>
    </section>
  );
}

export function Landing() {
  useEffect(() => {
    document.title = "Promissory: private scheduled payments on Zcash";
    return () => {
      document.title = "Promissory";
    };
  }, []);

  return (
    <div className="min-h-dvh">
      <a
        href="#main"
        className="sr-only focus:not-sr-only focus:fixed focus:left-4 focus:top-4 focus:z-50 focus:rounded-xl focus:bg-surface focus:px-3 focus:py-2 focus:shadow"
      >
        Skip to content
      </a>
      <div style={{ background: "var(--sky-fade)" }}>
        <div className="mx-auto max-w-6xl px-4 sm:px-8">
          <header className="flex flex-wrap items-center justify-between gap-4 py-5">
            <Link to="/" aria-label="Promissory home" className="rounded-xl focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
              <Wordmark />
            </Link>
            <nav aria-label="Sections" className="order-last flex w-full gap-6 overflow-x-auto text-[15px] font-medium sm:order-none sm:w-auto">
              <a href="#how" className="whitespace-nowrap hover:text-dawn-ink">How it works</a>
              <a href="#made" className="whitespace-nowrap hover:text-dawn-ink">Made for</a>
              <a href="#privacy" className="whitespace-nowrap hover:text-dawn-ink">Privacy</a>
              <a href="#builders" className="whitespace-nowrap hover:text-dawn-ink">Builders</a>
            </nav>
            <ButtonLink to="/orders" size="sm" variant="primary">
              Open the app
            </ButtonLink>
          </header>

          <main id="main">
            <section className="grid items-center gap-8 pt-8 pb-16 lg:grid-cols-[minmax(0,1.05fr)_minmax(0,1fr)] lg:pt-14 lg:pb-24">
              <div className="rise">
                <p className="glass inline-flex h-8 items-center rounded-full px-3.5 text-sm font-semibold">Private scheduled payments on Zcash</p>
                <h1 className="mt-6 text-[3.3rem] leading-[0.95] font-extrabold tracking-[-0.04em] text-balance sm:text-7xl lg:text-[5.6rem]">
                  Signed tonight.
                  <br />
                  Paid at dawn.
                </h1>
                <p className="mt-6 max-w-xl text-lg leading-relaxed text-fg/80 text-pretty sm:text-xl">
                  Rent, salaries and money for home, paid on the day, with your phone off and nobody watching. Change your mind? Cancel
                  any time.
                </p>
                <div className="mt-8 flex flex-wrap items-center gap-x-6 gap-y-3">
                  <ButtonLink to="/new" variant="primary" size="lg">
                    Schedule a payment
                    <ArrowRight className="size-4" aria-hidden />
                  </ButtonLink>
                  <a href="#how" className="inline-flex items-center gap-1.5 text-base font-semibold underline-offset-4 hover:underline">
                    See how it works
                    <ArrowDown className="size-4" aria-hidden />
                  </a>
                </div>
                <ul className="mt-10 flex flex-wrap gap-x-6 gap-y-2 text-sm font-medium text-fg/75 [&>li]:flex [&>li]:items-center [&>li]:gap-2 [&>li]:before:size-1.5 [&>li]:before:rounded-full [&>li]:before:bg-dawn [&>li]:before:content-['']">
                  <li>No smart contract</li>
                  <li>No custodian</li>
                  <li>Shielded end to end</li>
                </ul>
              </div>
              <HeroDial />
            </section>
          </main>
        </div>
      </div>

      <div className="mx-auto max-w-6xl px-4 sm:px-8">
        <Section
          id="how"
          eyebrow="How it works"
          title="Promise once. The chain keeps it."
          intro="Promissory turns a series of payments into sealed, signed transactions that open on their own dates. There is no contract holding your money and nobody in the middle."
        >
          <ol className="grid gap-4 sm:grid-cols-2 lg:grid-cols-5">
            {STEPS.map((s, i) => (
              <li
                key={s.title}
                className={cx(
                  "flex flex-col gap-4 rounded-3xl border p-6",
                  s.icon ? "border-dawn/40 bg-dawn-soft" : "border-border bg-surface",
                )}
              >
                {s.icon ?? <StepIcon n={i + 1} />}
                <div>
                  <h3 className="font-display text-xl font-bold">
                    <span className="sr-only">Step {i + 1}: </span>
                    {s.title}
                  </h3>
                  <p className="mt-1.5 text-[15px] leading-relaxed text-muted text-pretty">{s.body}</p>
                </div>
              </li>
            ))}
          </ol>
        </Section>

        <Section id="made" eyebrow="Made for" title="The payments you keep promising">
          <ul className="grid gap-4 sm:grid-cols-2 lg:grid-cols-5">
            {USES.map((u) => (
              <li key={u.title}>
                <Link
                  to={u.to}
                  className={cx(
                    "group flex h-full flex-col gap-4 rounded-3xl border p-6 transition-[transform,box-shadow] duration-200 hover:-translate-y-0.5 hover:shadow-[0_18px_40px_-24px_rgb(20_33_61_/_0.45)]",
                    "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                    u.warm ? "border-dawn/40 bg-dawn-soft" : "border-border bg-surface",
                  )}
                >
                  <span aria-hidden className="flex size-11 items-center justify-center rounded-2xl bg-sky text-fg">
                    <u.icon className="size-5" />
                  </span>
                  <span>
                    <span className="block font-display text-xl font-bold">{u.title}</span>
                    <span className="mt-1 block text-[15px] text-muted">{u.body}</span>
                  </span>
                  <span className="mt-auto inline-flex items-center gap-1 text-sm font-semibold text-dawn-ink">
                    Open
                    <ArrowRight className="size-4 transition-transform group-hover:translate-x-0.5" aria-hidden />
                  </span>
                </Link>
              </li>
            ))}
          </ul>
        </Section>

        <Section
          id="privacy"
          eyebrow="Who sees what"
          title="Sender, recipient and amount stay hidden."
          intro="Every payment settles as an ordinary shielded Zcash transaction. Here is everything each party can learn."
        >
          <div className="overflow-hidden rounded-3xl border border-border bg-surface">
            <table className="w-full text-left text-[15px]">
              <caption className="sr-only">What each party can and cannot see</caption>
              <thead className="border-b border-border bg-surface-2/70 text-sm text-muted">
                <tr>
                  <th scope="col" className="px-5 py-3.5 font-semibold sm:px-6">Who</th>
                  <th scope="col" className="hidden px-5 py-3.5 font-semibold sm:table-cell sm:px-6">Sees</th>
                  <th scope="col" className="hidden px-5 py-3.5 font-semibold sm:table-cell sm:px-6">Never sees</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-border">
                {PARTIES.map((p) => (
                  <tr key={p.party} className="align-top">
                    <th scope="row" className="px-5 py-4 font-bold sm:w-[26%] sm:px-6">
                      {p.party}
                      <dl className="mt-2 space-y-2 font-normal sm:hidden">
                        <div>
                          <dt className="text-sm font-semibold text-muted">Sees</dt>
                          <dd className="text-muted">{p.sees}</dd>
                        </div>
                        <div>
                          <dt className="text-sm font-semibold text-muted">Never sees</dt>
                          <dd>{p.never}</dd>
                        </div>
                      </dl>
                    </th>
                    <td className="hidden px-5 py-4 text-muted text-pretty sm:table-cell sm:px-6">{p.sees}</td>
                    <td className="hidden px-5 py-4 text-pretty sm:table-cell sm:px-6">{p.never}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <p className="mt-4 text-sm text-muted">
            One caveat: if your device was offline in the hour before a payment, the chain may tell that payment was prepared in
            advance. <Link to="/privacy" className="font-semibold text-dawn-ink underline-offset-4 hover:underline">The full picture</Link>
          </p>
        </Section>

        <Section
          id="proof"
          eyebrow="Proof"
          title="Run on a real Zcash node, not a slide."
          intro="Every claim on this page was tested end to end on a local Zcash chain (Zebra and lightwalletd, regtest) on the same network upgrade mainnet runs today. Each result is logged with its transaction IDs."
        >
          <dl className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            {PROOF.map((p) => (
              <div key={p.label} className="rounded-3xl border border-border bg-surface p-6">
                <dt className="text-sm font-semibold text-muted">{p.label}</dt>
                <dd className="mt-2 font-display text-2xl leading-tight font-extrabold tracking-tight">{p.value}</dd>
                <dd className="mt-2 font-mono text-sm text-muted">{p.detail}</dd>
              </div>
            ))}
          </dl>
        </Section>

        <Section
          id="builders"
          eyebrow="For builders"
          title="Add recurring payments to anything."
          intro="Everything the app does is open to you: a TypeScript client, a keeper anyone can run, and an MCP server for AI agents."
        >
          <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.2fr)]">
            <ul className="grid gap-4">
              <li className="rounded-3xl border border-border bg-surface p-6">
                <h3 className="font-display text-xl font-bold">@promissory/client</h3>
                <p className="mt-1.5 text-[15px] text-muted">Subscribe links, address checks, the keeper and payer APIs. No runtime dependencies.</p>
              </li>
              <li className="rounded-3xl border border-border bg-surface p-6">
                <h3 className="font-display text-xl font-bold">promissory-keeper</h3>
                <p className="mt-1.5 text-[15px] text-muted">Holds sealed payments and sends each one when it opens. It can’t pay early or redirect.</p>
              </li>
              <li className="rounded-3xl border border-border bg-surface p-6">
                <h3 className="font-display text-xl font-bold">promissory-mcp</h3>
                <p className="mt-1.5 text-[15px] text-muted">Lets an AI agent ask for an allowance; you approve the cap once.</p>
              </li>
            </ul>
            <figure className="min-w-0 overflow-hidden rounded-3xl bg-ink text-[#f3eee6]">
              <figcaption className="border-b border-white/10 px-6 py-3.5 font-mono text-sm text-[#c9d1e3]">subscribe.ts</figcaption>
              <pre className="overflow-x-auto px-6 py-5 font-mono text-[13.5px] leading-relaxed">
                <code>{SNIPPET}</code>
              </pre>
            </figure>
          </div>
        </Section>

        <section aria-labelledby="cta-title" className="mb-16 overflow-hidden rounded-[32px] px-6 py-14 text-center sm:px-10 sm:py-20" style={{ background: "var(--hero)" }}>
          <div aria-hidden className="mx-auto mb-8 size-24 rounded-full bg-[radial-gradient(circle_at_50%_75%,#f2a65a_0_42%,var(--ink)_43%)] shadow-[0_20px_50px_-12px_rgb(242_166_90_/_0.7)]" />
          <h2 id="cta-title" className="font-display text-[2.3rem] leading-[1.02] font-extrabold tracking-[-0.03em] text-balance sm:text-6xl">
            Make the promise tonight.
          </h2>
          <p className="mx-auto mt-4 max-w-lg text-lg text-fg/80">It pays on the day, with your wallet closed. Cancel any time.</p>
          <div className="mt-8 flex flex-wrap justify-center gap-3">
            <ButtonLink to="/new" variant="primary" size="lg">
              Schedule a payment
            </ButtonLink>
            <ButtonLink to="/new?mode=home" size="lg">
              Send money home
            </ButtonLink>
          </div>
        </section>
      </div>

      <footer className="border-t border-border">
        <div className="mx-auto flex max-w-6xl flex-wrap items-center justify-between gap-4 px-4 py-8 text-sm text-muted sm:px-8">
          <Wordmark className="text-fg" />
          <p>Runs on your device. No smart contract, no custodian.</p>
        </div>
      </footer>
    </div>
  );
}
