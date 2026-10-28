import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/** Every .ts/.tsx file under src, tests excluded. */
function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sources(path);
    return /\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name) ? [path] : [];
  });
}

describe("effects", () => {
  // An effect's return value is called as its cleanup. An expression body returns whatever the
  // expression evaluates to: `useEffect(() => window.scrollTo(0, 0))` returns a Promise in newer
  // browsers, and every navigation then crashed with "k is not a function".
  it("never use an expression body, except to return a cleanup function", () => {
    const offenders = sources(join(__dirname, "..")).flatMap((file) =>
      readFileSync(file, "utf8")
        .split("\n")
        .map((line, i) => ({ line, at: `${file}:${i + 1}` }))
        .filter(({ line }) => /useEffect\(\s*(async\s*)?\(\)\s*=>(?!\s*\{|\s*\(\)\s*=>)/.test(line))
        .map(({ at, line }) => `${at}: ${line.trim()}`),
    );
    expect(offenders).toEqual([]);
  });
});
