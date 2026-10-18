import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

import { describe, expect, it } from "vitest";

const MARKER = "No diagram type detected for the given text.";

describe("desktop bundle", () => {
  it("keeps pele out of the main entry chunk", () => {
    const out = mkdtempSync(join(tmpdir(), "outl-desktop-pele-"));
    const require = createRequire(join(process.cwd(), "package.json"));
    const vite = join(dirname(require.resolve("vite/package.json")), "bin/vite.js");
    execFileSync(process.execPath, [vite, "build", "--outDir", out, "--emptyOutDir"], {
      cwd: process.cwd(),
      timeout: 180_000,
      env: process.env,
      stdio: "pipe",
    });
    const html = readFileSync(join(out, "index.html"), "utf8");
    const entryHref = html.match(/<script type="module"[^>]*src="([^"]+)"/)?.[1];
    expect(entryHref, html).toBeTruthy();
    const entryName = entryHref!.replace(/^\//, "");
    const entry = readFileSync(join(out, entryName), "utf8");
    expect(entry).not.toContain(MARKER);

    const assets = readdirSync(join(out, "assets"));
    const peleChunk = assets.find((name) => {
      if (name === entryName.replace(/^assets\//, "")) return false;
      return readFileSync(join(out, "assets", name), "utf8").includes(MARKER);
    });
    expect(peleChunk, assets.join("\n")).toBeTruthy();
  }, 180_000);
});
