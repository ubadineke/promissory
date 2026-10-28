import { Check, Copy } from "lucide-react";
import type { ButtonHTMLAttributes, ReactNode } from "react";
import { Link, type LinkProps } from "react-router";
import { formatZec, truncateMiddle, type Network } from "@promissory/client";
import { useCopy } from "../hooks";

export function cx(...parts: (string | false | null | undefined)[]): string {
  return parts.filter(Boolean).join(" ");
}

const focus =
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-bg";

type Variant = "primary" | "secondary" | "danger" | "danger-soft" | "ghost";

const VARIANTS: Record<Variant, string> = {
  primary: "bg-accent text-accent-fg hover:bg-accent-hover shadow-[0_8px_20px_-10px_rgb(20_33_61_/_0.55)]",
  secondary: "bg-surface/80 text-fg border border-border-strong hover:bg-surface-2",
  danger: "bg-danger-solid text-white hover:brightness-110 shadow-sm",
  "danger-soft": "bg-danger-soft text-danger border border-danger/30 hover:border-danger/60",
  ghost: "text-fg hover:bg-surface-2",
};

type Size = "lg" | "md" | "sm";

const SIZES: Record<Size, string> = {
  lg: "h-13 rounded-2xl px-7 text-base",
  md: "h-11 rounded-2xl px-5 text-[15px]",
  sm: "h-9 rounded-xl px-3.5 text-sm",
};

export function buttonClass(variant: Variant = "secondary", size: Size = "md"): string {
  return cx(
    "inline-flex items-center justify-center gap-2 font-semibold whitespace-nowrap select-none",
    "transition-[background-color,filter,box-shadow] duration-150 disabled:opacity-50 disabled:pointer-events-none",
    SIZES[size],
    VARIANTS[variant],
    focus,
  );
}

export function Button({
  variant = "secondary",
  size = "md",
  className,
  type = "button",
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant; size?: Size }) {
  return <button type={type} className={cx(buttonClass(variant, size), className)} {...props} />;
}

export function ButtonLink({
  variant = "secondary",
  size = "md",
  className,
  ...props
}: LinkProps & { variant?: Variant; size?: Size }) {
  return <Link className={cx(buttonClass(variant, size), className)} {...props} />;
}

export function Card({ className, children }: { className?: string; children: ReactNode }) {
  return <section className={cx("rounded-3xl border border-border bg-surface", className)}>{children}</section>;
}

export function CardHeader({ title, action, children }: { title: ReactNode; action?: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex flex-wrap items-start justify-between gap-3 border-b border-border px-5 py-4 sm:px-6">
      <div className="min-w-0">
        <h2 className="text-base font-bold">{title}</h2>
        {children && <div className="mt-0.5 text-sm text-muted">{children}</div>}
      </div>
      {action}
    </div>
  );
}

export function PageHeader({ title, subtitle, action }: { title: ReactNode; subtitle?: ReactNode; action?: ReactNode }) {
  return (
    <div className="mb-8 flex flex-wrap items-end justify-between gap-4">
      <div className="min-w-0">
        <h1 className="text-[2.1rem] leading-[1.05] font-extrabold tracking-[-0.03em] text-balance sm:text-[2.6rem]">{title}</h1>
        {subtitle && <p className="mt-2 max-w-prose text-base text-muted text-pretty">{subtitle}</p>}
      </div>
      {action}
    </div>
  );
}

/** An amount in ZEC, formatted by promissory-js, with tabular figures. */
export function Zec({ zat, className, unit = true }: { zat: number; className?: string; unit?: boolean }) {
  return (
    <span className={cx("tabular-nums", className)}>
      {formatZec(zat)}
      {unit && <span className="ml-1 text-[0.85em] font-normal text-muted">ZEC</span>}
    </span>
  );
}

export function NetworkBadge({ network }: { network: Network }) {
  return network === "main" ? (
    <span className="inline-flex h-7 items-center rounded-full bg-danger-solid px-3 text-xs font-bold tracking-[0.08em] text-white">
      MAINNET
    </span>
  ) : (
    <span className="inline-flex h-7 items-center rounded-full border border-border-strong bg-surface-2 px-3 font-mono text-xs text-muted">
      regtest
    </span>
  );
}

export function CopyButton({ text, label, className }: { text: string; label: string; className?: string }) {
  const [copied, copy] = useCopy();
  return (
    <button
      type="button"
      onClick={() => void copy(text)}
      aria-label={copied ? "Copied" : label}
      title={copied ? "Copied" : label}
      className={cx(
        "inline-flex size-9 shrink-0 items-center justify-center rounded-xl text-muted transition-colors hover:bg-surface-2 hover:text-fg",
        focus,
        className,
      )}
    >
      {copied ? <Check className="size-4 text-ok" aria-hidden /> : <Copy className="size-4" aria-hidden />}
      <span className="sr-only" aria-live="polite">
        {copied ? "Copied" : ""}
      </span>
    </button>
  );
}

/** A long identifier truncated in the middle, with the full value on hover and a copy button. */
export function Address({
  value,
  head = 8,
  tail = 6,
  label = "Copy address",
  className,
}: {
  value: string;
  head?: number;
  tail?: number;
  label?: string;
  className?: string;
}) {
  return (
    <span className={cx("inline-flex min-w-0 items-center gap-0.5 align-middle", className)}>
      <span className="truncate font-mono text-[0.92em]" title={value}>
        {truncateMiddle(value, head, tail)}
      </span>
      <CopyButton text={value} label={label} className="-my-2" />
    </span>
  );
}

export function Skeleton({ className }: { className?: string }) {
  return <div className={cx("animate-pulse rounded-xl bg-surface-2 motion-reduce:animate-none", className)} aria-hidden />;
}

type Tone = "neutral" | "accent" | "ok" | "warn" | "danger";

const TONES: Record<Tone, string> = {
  neutral: "bg-surface-2 text-muted border-border",
  accent: "bg-accent-soft text-accent border-transparent",
  ok: "bg-ok-soft text-ok border-transparent",
  warn: "bg-warn-soft text-warn border-transparent",
  danger: "bg-danger-soft text-danger border-transparent",
};

export function Chip({ tone = "neutral", children }: { tone?: Tone; children: ReactNode }) {
  return (
    <span className={cx("inline-flex h-7 items-center gap-1.5 rounded-full border px-3 text-xs font-semibold whitespace-nowrap", TONES[tone])}>
      {children}
    </span>
  );
}

export function Notice({ tone = "neutral", icon, children }: { tone?: Tone; icon?: ReactNode; children: ReactNode }) {
  return (
    <div className={cx("flex gap-3 rounded-2xl border px-4 py-3.5 text-sm", TONES[tone])}>
      {icon && <span className="mt-0.5 shrink-0">{icon}</span>}
      <div className="min-w-0 text-pretty [&_strong]:font-semibold">{children}</div>
    </div>
  );
}

export const inputClass = cx(
  "block w-full rounded-xl border border-border-strong bg-surface px-3.5 text-[15px] text-fg placeholder:text-muted/70",
  "transition-[border-color,box-shadow] duration-150 focus:border-accent focus:outline-none focus:ring-2 focus:ring-ring/30",
  "aria-[invalid=true]:border-danger aria-[invalid=true]:focus:ring-danger/30",
);

export function Field({
  id,
  label,
  hint,
  error,
  children,
  aside,
}: {
  id: string;
  label: ReactNode;
  hint?: ReactNode;
  error?: string | undefined;
  children: ReactNode;
  aside?: ReactNode;
}) {
  return (
    <div className="space-y-1.5">
      <div className="flex items-baseline justify-between gap-2">
        <label htmlFor={id} className="text-sm font-semibold">
          {label}
        </label>
        {aside}
      </div>
      {children}
      {error ? (
        <p id={`${id}-error`} className="text-sm text-danger" role="alert">
          {error}
        </p>
      ) : hint ? (
        <p id={`${id}-hint`} className="text-sm text-muted">
          {hint}
        </p>
      ) : null}
    </div>
  );
}

/** `aria-describedby` and `aria-invalid` for an input inside `Field`. */
export function describedBy(id: string, error: string | undefined, hasHint = true) {
  return {
    "aria-invalid": error ? true : undefined,
    "aria-describedby": error ? `${id}-error` : hasHint ? `${id}-hint` : undefined,
  } as const;
}

/** A progress ring drawn with a conic gradient; `children` sit in the middle. */
export function Ring({
  value,
  size = 36,
  thickness = 4,
  className,
  children,
}: {
  value: number;
  size?: number;
  thickness?: number;
  className?: string;
  children?: ReactNode;
}) {
  const deg = Math.max(0, Math.min(1, value)) * 360;
  return (
    <span
      aria-hidden
      className={cx("relative inline-flex shrink-0 items-center justify-center rounded-full", className)}
      style={{
        width: size,
        height: size,
        background: `conic-gradient(var(--fg) 0deg ${deg}deg, color-mix(in srgb, var(--fg) 14%, transparent) ${deg}deg 360deg)`,
      }}
    >
      <span
        className="absolute rounded-full bg-surface"
        style={{ inset: thickness }}
      />
      <span className="relative inline-flex items-center justify-center">{children}</span>
    </span>
  );
}

/** The Daybreak countdown: a frosted-glass pill with a progress ring. */
export function Dial({
  label,
  value,
  progress,
  dateTime,
}: {
  label: string;
  value: string;
  progress: number;
  dateTime?: string;
}) {
  return (
    <div className="glass flex items-center gap-4 rounded-3xl px-5 py-4 shadow-[0_20px_40px_-24px_rgb(20_33_61_/_0.45)]">
      <Ring value={progress} size={60} thickness={6} className="[&>span:first-child]:bg-surface/90" />
      <div>
        <p className="text-sm text-muted">{label}</p>
        <p className="font-mono text-[1.65rem] leading-tight font-medium tabular-nums">
          {dateTime ? <time dateTime={dateTime}>{value}</time> : value}
        </p>
      </div>
    </div>
  );
}
