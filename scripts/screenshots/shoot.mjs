#!/usr/bin/env node
// Regenerates the outl product screenshots: desktop and mobile in headless
// Chromium under `?mock=1`, the TUI recorded with vhs, all on the same
// "Atlas" demo workspace built by the real outl binary.
//
//   node shoot.mjs [--out DIR] [--outl PATH] [--only desktop,mobile,tui]
//
// See README.md for what it needs installed.
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { chromium } from "playwright";
import { frame } from "./lib/frame.mjs";
import { buildWorkspace, dumpFixture } from "./lib/workspace.mjs";
import { desktopShots, mobileShots, HERO } from "./shots.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "../..");
const { values: opts } = parseArgs({
  options: {
    out: { type: "string", default: join(repo, "target/screenshots") },
    outl: { type: "string", default: join(repo, "target/release/outl") },
    only: { type: "string", default: "desktop,mobile,tui" },
  },
});
const only = new Set(opts.only.split(","));
const out = resolve(opts.out);
const outl = resolve(opts.outl);
if (!existsSync(outl)) {
  console.error(`outl binary not found at ${outl}; build it with \`cargo build --release -p outl-cli\` or pass --outl`);
  process.exit(1);
}

rmSync(out, { recursive: true, force: true });
for (const d of ["raw", "framed", "hero"]) mkdirSync(join(out, d), { recursive: true });

const work = join(out, ".work");
// Every outl subprocess (import, JSON dumps, the TUI under vhs) shares
// disposable device and config state under .work, never the caller's.
process.env.OUTL_DEVICE_DIR = join(work, "device");
process.env.XDG_CONFIG_HOME = join(work, "xdg");
const ws = buildWorkspace({ outl, fixtureDir: join(here, "fixture"), workDir: work });
// A `remind::` that is already due would pop a TUI toast over the shot.
// The TUI reads this from $XDG_CONFIG_HOME/outl/config.toml.
mkdirSync(join(work, "xdg", "outl"), { recursive: true });
writeFileSync(join(work, "xdg", "outl", "config.toml"), "[reminders]\nenabled = false\n");
const fx = dumpFixture({ outl, ws });
writeFileSync(join(work, "fixture.json"), JSON.stringify(fx, null, 2));
console.log(`workspace: ${fx.pages.length} pages, today is ${fx.today}`);

const made = [];
const failed = [];

async function withServer(crate, port, fn) {
  const child = spawn("bun", ["x", "vite", "--port", String(port), "--strictPort"], {
    cwd: join(repo, "crates", crate),
    stdio: ["ignore", "pipe", "pipe"],
  });
  let log = "";
  child.stdout.on("data", (b) => (log += b));
  child.stderr.on("data", (b) => (log += b));
  const url = `http://localhost:${port}/`;
  try {
    const deadline = Date.now() + 60_000;
    for (;;) {
      try {
        if ((await fetch(url)).ok) break;
      } catch {}
      if (Date.now() > deadline || child.exitCode !== null) throw new Error(`vite for ${crate} did not start:\n${log}`);
      await new Promise((r) => setTimeout(r, 500));
    }
    return await fn(`${url}?mock=1`);
  } finally {
    child.kill();
  }
}

const bridge = readFileSync(join(here, "lib/bridge.js"), "utf8");

async function shootWeb(browser, url, device, shots) {
  for (const shot of shots) {
    for (const scheme of ["light", "dark"]) {
      const name = `${device.prefix}-${shot.name}-${scheme}`;
      const ctx = await browser.newContext({
        viewport: device.viewport,
        deviceScaleFactor: device.dpr,
        isMobile: device.mobile,
        hasTouch: device.mobile,
        colorScheme: scheme,
        reducedMotion: "reduce",
      });
      const link = shot.link ? shot.link(fx) : null;
      await ctx.addInitScript(
        `window.__OUTL_SHOTS__ = ${JSON.stringify(fx)};\nwindow.__OUTL_SHOTS_LINK__ = ${JSON.stringify(link)};\n${bridge}`,
      );
      const page = await ctx.newPage();
      try {
        await page.goto(url);
        await page.waitForLoadState("networkidle");
        await page.waitForTimeout(1200);
        if (shot.act) await shot.act(page, fx);
        await page.waitForTimeout(600);
        const raw = join(out, "raw", `${name}.png`);
        await page.screenshot({ path: raw });
        await frame(browser, {
          raw,
          out: join(out, "framed", `${name}-framed.png`),
          kind: device.mobile ? "mobile" : "desktop",
          scheme,
          dpr: device.dpr,
          ...device.viewport,
        });
        made.push(name);
        console.log(`ok   ${name}`);
      } catch (e) {
        failed.push(name);
        console.error(`FAIL ${name}: ${e.message.split("\n")[0]}`);
      } finally {
        await ctx.close();
      }
    }
  }
}

const sq = (s) => `'${s.replaceAll("'", `'\\''`)}'`;

function shootTui() {
  const tapes = join(here, "tui");
  const shots = [
    { name: "journal", tape: "journal.tape" },
    { name: "person-backlinks", tape: "person-backlinks.tape" },
  ];
  // The brand presets every client ships by default (outl-theme
  // presets.rs). The terminal canvas uses the same bg/fg so the TUI
  // reads like it does in a terminal themed to match.
  const themes = { light: "outl-light", dark: "outl" };
  const ansi = {
    black: "#24292f", red: "#cf222e", green: "#116329", yellow: "#4d2d00",
    blue: "#0969da", magenta: "#8250df", cyan: "#1b7c83", white: "#6e7781",
    brightBlack: "#57606a", brightRed: "#a40e26", brightGreen: "#1a7f37", brightYellow: "#633c01",
    brightBlue: "#218bff", brightMagenta: "#a475f9", brightCyan: "#3192aa", brightWhite: "#8c959f",
  };
  const canvas = {
    light: { ...ansi, name: "outl-light", background: "#ffffff", foreground: "#111827", selection: "#e5e7eb", cursor: "#7c3aed" },
    dark: { ...ansi, name: "outl", background: "#0c0814", foreground: "#f4f1fa", selection: "#382c54", cursor: "#c4b5fd" },
  };
  const jobs = [];
  for (const shot of shots) {
    for (const [scheme, theme] of Object.entries(themes)) {
      const name = `tui-${shot.name}-${scheme}`;
      const raw = join(out, "raw", `${name}.png`);
      const body = readFileSync(join(tapes, shot.tape), "utf8");
      const tape = join(work, `${name}.tape`);
      writeFileSync(
        tape,
        [
          `Output "${join(work, name)}.gif"`,
          `Set Shell "bash"`,
          `Set FontSize 18`,
          `Set Width 1400`,
          `Set Height 820`,
          `Set Padding 24`,
          `Set Theme ${JSON.stringify(canvas[scheme])}`,
          // Same color depth on every machine: without these the TUI
          // falls back to fewer colors wherever the caller's env lacks them.
          `Env TERM "xterm-256color"`,
          `Env COLORTERM "truecolor"`,
          `Hide`,
          // Backtick-delimited so the shell's single quotes pass through untouched.
          // NO_COLOR in the caller's env (CI images, sandboxes) would strip the palette.
          `Type \`unset NO_COLOR FORCE_COLOR && cd ${sq(ws)} && clear && ${sq(outl)} --theme ${theme}\``,
          `Enter`,
          `Sleep 2s`,
          `Show`,
          body.trim(),
          `Screenshot "${raw}"`,
          // vhs writes the screenshot asynchronously; give it a beat.
          `Sleep 1s`,
          `Hide`,
          `Ctrl+C`,
          "",
        ].join("\n"),
      );
      jobs.push({ name, raw, scheme, tape });
    }
  }
  return jobs;
}

const browser = await chromium.launch();
try {
  if (only.has("desktop")) {
    await withServer("outl-desktop", 1421, (url) =>
      shootWeb(browser, url, { prefix: "desktop", viewport: { width: 1440, height: 900 }, dpr: 2, mobile: false }, desktopShots),
    );
  }
  if (only.has("mobile")) {
    await withServer("outl-mobile", 1420, async (url) => {
      await shootWeb(browser, url, { prefix: "mobile-iphone", viewport: { width: 390, height: 844 }, dpr: 3, mobile: true }, mobileShots);
      await shootWeb(browser, url, { prefix: "mobile-android", viewport: { width: 412, height: 915 }, dpr: 2.625, mobile: true }, mobileShots);
    });
  }
  if (only.has("tui")) {
    for (const job of shootTui()) {
      // The first vhs run on a cold machine sometimes exits before the
      // screenshot lands, so give each tape one retry.
      let r;
      for (let attempt = 0; attempt < 2 && !existsSync(job.raw); attempt++) {
        r = spawnSync("vhs", [job.tape], {
          stdio: ["ignore", "ignore", "pipe"],
          encoding: "utf8",
          // Keep the runner's own ~/.config/outl out of the shot.
          env: { ...process.env, XDG_CONFIG_HOME: join(work, "xdg") },
        });
      }
      if (!existsSync(job.raw)) {
        failed.push(job.name);
        console.error(`FAIL ${job.name}: vhs exited ${r.status}: ${(r.stderr || r.error?.message || "").trim().split("\n").pop()}`);
        continue;
      }
      await frame(browser, {
        raw: job.raw,
        out: join(out, "framed", `${job.name}-framed.png`),
        kind: "tui",
        scheme: job.scheme,
        dpr: 1,
        width: 1400,
        height: 820,
      });
      made.push(job.name);
      console.log(`ok   ${job.name}`);
    }
  }
} finally {
  await browser.close();
}

for (const name of HERO) {
  if (made.includes(name)) {
    writeFileSync(join(out, "hero", `${name}.png`), readFileSync(join(out, "framed", `${name}-framed.png`)));
  }
}
rmSync(work, { recursive: true, force: true });
console.log(`\n${made.length} shots in ${out}${failed.length ? `, ${failed.length} failed: ${failed.join(", ")}` : ""}`);
process.exit(failed.length ? 1 : 0);
