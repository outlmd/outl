/**
 * Pins the page half of the floating iOS toolbar's clearance: the
 * property the native side writes (`ToolbarLayout.clearanceProperty` in
 * OutlKit) is declared with a length fallback, and the outline reads it
 * as scroll padding, once. A second consumer (padding on top of the
 * outline's existing tail room) would double the gap.
 */
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const src = join(__dirname, "..");
const PROPERTY = "--outl-toolbar-clearance";

function walk(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? walk(join(dir, e.name)) : [join(dir, e.name)],
  );
}

describe("toolbar clearance variable", () => {
  const css = readFileSync(join(src, "styles.css"), "utf8");

  it("is declared with a length fallback", () => {
    expect(css).toContain(`${PROPERTY}: 0px;`);
  });

  it("is consumed as the outline's scroll-padding-bottom", () => {
    expect(css).toMatch(
      new RegExp(
        `main\\.ios-scroll\\s*\\{[^}]*scroll-padding-bottom:\\s*var\\(${PROPERTY}\\)`,
      ),
    );
  });

  it("is read nowhere else in the frontend", () => {
    const readers = walk(src)
      .filter((f) => /\.(tsx?|css)$/.test(f))
      .filter((f) => !/\.test\.tsx?$/.test(f))
      .flatMap((f) =>
        readFileSync(f, "utf8")
          .split("\n")
          .filter((l) => l.includes(`var(${PROPERTY}`))
          .map(() => f),
      );
    expect(readers).toEqual([join(src, "styles.css")]);
  });

  it("matches the property OutlKit publishes", () => {
    const swift = readFileSync(
      join(
        src,
        "..",
        "swift/OutlKit/Sources/OutlKit/Toolbar/ToolbarLayout.swift",
      ),
      "utf8",
    );
    expect(swift).toContain(`clearanceProperty = "${PROPERTY}"`);
  });
});
