import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

describe("pele import", () => {
  it("loads pele through pele/lazy and never a static pele import", () => {
    const dir = join(process.cwd(), "src/diagram");
    const files = readdirSync(dir).filter((name) => name.endsWith(".ts") || name.endsWith(".tsx"));
    const sources = files.map((name) => readFileSync(join(dir, name), "utf8")).join("\n");
    expect(sources).toContain('import("pele/lazy")');
    expect(sources).not.toMatch(/from ["']pele["']/);
    expect(sources).not.toMatch(/import\(["']pele["']\)/);
  });
});
