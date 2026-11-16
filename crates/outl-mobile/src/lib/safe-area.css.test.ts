/**
 * Pins the mechanism's other half: the stylesheet declares the variables
 * with a length fallback, and no component reads `env(safe-area-inset-*)`
 * behind its back (which would skip the native-measured value).
 */
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const src = join(__dirname, "..");

function walk(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? walk(join(dir, e.name)) : [join(dir, e.name)],
  );
}

describe("safe-area variables", () => {
  const css = readFileSync(join(src, "styles.css"), "utf8");

  it.each(["top", "right", "bottom", "left"])(
    "declares --safe-area-inset-%s from env() with a length fallback",
    (side) => {
      expect(css).toContain(
        `--safe-area-inset-${side}: env(safe-area-inset-${side}, 0px);`,
      );
    },
  );

  it("reads env(safe-area-inset-*) only in the :root declaration", () => {
    const offenders = walk(src)
      .filter((f) => /\.(tsx?|css)$/.test(f))
      .filter((f) => !/\.test\.tsx?$/.test(f) && !f.endsWith("safe-area.ts"))
      .flatMap((f) =>
        readFileSync(f, "utf8")
          .split("\n")
          .filter((l) => /env\(safe-area-inset/.test(l))
          .filter((l) => !/^\s*--safe-area-inset-(top|right|bottom|left): env\(/.test(l))
          // The explanatory comment in styles.css names it in prose.
          .filter((l) => !/^\s*(\/\*|\*)|`env\(\)`|\*\/\s*$/.test(l))
          .map((l) => `${f}: ${l.trim()}`),
      );
    expect(offenders).toEqual([]);
  });
});
