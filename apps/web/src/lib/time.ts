const dateTime = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });
const timeOnly = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit", second: "2-digit" });

export function formatDateTime(unix: number): string {
  return dateTime.format(unix * 1000);
}

/** Time of day with seconds when `unix` is today, else date and time. */
export function formatWhen(unix: number): string {
  const d = new Date(unix * 1000);
  return d.toDateString() === new Date().toDateString() ? timeOnly.format(d) : dateTime.format(d);
}

/** Value for `<input type="datetime-local">` in local time. */
export function toLocalInput(unix: number): string {
  const d = new Date(unix * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
}

export function fromLocalInput(value: string): number | null {
  const ms = new Date(value).getTime();
  return Number.isFinite(ms) ? Math.floor(ms / 1000) : null;
}

/** Host of a URL, or the input if it does not parse. */
export function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}
