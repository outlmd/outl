// Builds the "Atlas" demo workspace from fixture/ with the real outl
// binary, then dumps what the clients need as JSON straight from the
// CLI, so every block, token and backlink in a shot is backend output.
import { execFileSync } from "node:child_process";
import { appendFileSync, cpSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const p2 = (n) => String(n).padStart(2, "0");
export const slugFor = (d) => `${d.getFullYear()}-${p2(d.getMonth() + 1)}-${p2(d.getDate())}`;
const shift = (base, by) => new Date(base.getFullYear(), base.getMonth(), base.getDate() + by);

// The fixture is dated relative to the day the script runs, so "today"
// in every client is a populated journal. `{{day:+7}}` in a fixture file
// and `journals/day+7.md` both mean "today plus seven days".
const expand = (text, today) =>
  text.replace(/\{\{day:([+-]\d+)\}\}/g, (_, n) => slugFor(shift(today, Number(n))));

export function buildWorkspace({ outl, fixtureDir, workDir, today = new Date() }) {
  rmSync(workDir, { recursive: true, force: true });
  const src = join(workDir, "src");
  const ws = join(workDir, "workspace");
  mkdirSync(join(src, "pages"), { recursive: true });
  mkdirSync(join(src, "journals"), { recursive: true });
  for (const f of readdirSync(join(fixtureDir, "pages"))) {
    writeFileSync(join(src, "pages", f), expand(readFileSync(join(fixtureDir, "pages", f), "utf8"), today));
  }
  for (const f of readdirSync(join(fixtureDir, "journals"))) {
    const m = /^day([+-]\d+)\.md$/.exec(f);
    if (!m) throw new Error(`fixture journal ${f} must be named day+N.md / day-N.md`);
    // Logseq names journals with underscores; `outl import logseq` maps them.
    const name = slugFor(shift(today, Number(m[1]))).replaceAll("-", "_");
    writeFileSync(join(src, "journals", `${name}.md`), expand(readFileSync(join(fixtureDir, "journals", f), "utf8"), today));
  }
  // The fixture is plain outl markdown laid out like a Logseq graph, which
  // is the importer that takes a directory of .md as-is.
  run(outl, ["import", "logseq", src, ws, "--json"]);
  // A `remind::` that is already due would pop a TUI toast over the shot.
  // The rule stays in the page; this device just doesn't fire it.
  appendFileSync(join(ws, ".outl", "config.toml"), "\n[reminders]\nenabled = false\n");
  cpSync(src, join(workDir, "fixture-expanded"), { recursive: true });
  return ws;
}

function run(outl, args) {
  return execFileSync(outl, args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
}

function cli(outl, ws, args) {
  const env = JSON.parse(run(outl, ["-w", ws, ...args, "--json"]));
  if (!env.ok) throw new Error(`outl ${args.join(" ")}: ${env.error?.message}`);
  return env.data;
}

/** Queries the shots type into a search box; answered from `outl search`. */
export const SEARCHES = ["sync"];

// `outl page prop list` leaves some keys out (owner::, type::), so take
// the keys from the page header the CLI wrote and read each one back
// through `outl page prop get`.
function pageProperties(outl, ws, meta) {
  const dir = meta.kind === "journal" ? "journals" : "pages";
  const md = readFileSync(join(ws, dir, `${meta.slug}.md`), "utf8");
  const keys = [];
  for (const line of md.split("\n")) {
    const m = /^([A-Za-z][\w-]*):: /.exec(line);
    if (!m) break;
    keys.push(m[1]);
  }
  return keys
    .filter((k) => k !== "page-slug" && k !== "page-kind")
    .map((k) => [k, cli(outl, ws, ["page", "prop", "get", meta.slug, k]).value])
    .sort((a, b) => a[0].localeCompare(b[0]));
}

export function dumpFixture({ outl, ws, today = new Date() }) {
  const pages = cli(outl, ws, ["page", "list"]).pages;
  const views = {};
  const backlinks = {};
  for (const meta of pages) {
    const got = cli(outl, ws, ["page", "get", meta.slug]);
    const props = pageProperties(outl, ws, meta);
    views[meta.slug] = { page: { ...meta, ...got.meta }, outline: got.outline, page_properties: props };
    backlinks[meta.slug] = cli(outl, ws, ["backlinks", "page", meta.slug]).backlinks;
  }
  const templates = cli(outl, ws, ["template", "list"]).templates;
  const searches = {};
  for (const q of SEARCHES) {
    searches[q] = cli(outl, ws, ["search", q]).blocks.map((b) => ({
      handle: b.handle,
      id: b.id,
      text: b.text,
      source_slug: b.page,
    }));
  }
  return { today: slugFor(today), pages, views, backlinks, templates, searches };
}
