import { cx } from "./ui";

/** A night sky with the sun coming up inside it: paid at dawn. */
export function Logo({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 32 32" className={cx("size-7 shrink-0", className)} aria-hidden>
      <defs>
        <clipPath id="logo-disc">
          <circle cx="16" cy="16" r="15" />
        </clipPath>
      </defs>
      <circle cx="16" cy="16" r="15" className="fill-ink" />
      <g clipPath="url(#logo-disc)">
        <circle cx="16" cy="25" r="9.5" className="fill-dawn" />
        <rect x="0" y="25" width="32" height="7" className="fill-ink" opacity="0.35" />
      </g>
      <circle cx="16" cy="16" r="15" fill="none" className="stroke-fg" strokeOpacity="0.12" />
    </svg>
  );
}

export function Wordmark({ className }: { className?: string }) {
  return (
    <span className={cx("inline-flex items-center gap-2.5 font-display text-[1.35rem] font-extrabold tracking-[-0.02em]", className)}>
      <Logo />
      Promissory
    </span>
  );
}
