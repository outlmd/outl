/**
 * Every client asks "did the page change?" through `createPageKey`.
 *
 * This is what makes that true rather than merely documented, and it
 * exists because the rule was documented and false. Two surfaces on the
 * desktop derived a tracked source straight off `appState.page?.slug`,
 * one of them under a comment claiming it fired once per navigation:
 *
 * - the backlinks fetch, which blanked its rows and unmounted the whole
 *   section for the length of an `O(blocks-in-workspace)` scan on every
 *   block created or deleted;
 * - the sidebar's recents queue, which rebuilt its array and wrote
 *   `localStorage` synchronously on every commit.
 *
 * Neither had a test, because the question was asked inline in a
 * component where nothing could reach it. A source-read is what reaches
 * it.
 *
 * Both rules below take a **declared** exception with a reason — "declare
 * the gap, don't discover it", the same shape as `outl_shortcuts::support`
 * and `BlockRow.recursion.test.ts`'s `NOT_FORWARDED`.
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const CLIENTS = ["outl-desktop", "outl-mobile"];

/**
 * Files that track a page field directly. Each needs a reason, and the
 * reason has to say why a key would be **wrong** here — not merely
 * unnecessary.
 */
const DELIBERATE_TRACKING: Record<string, string> = {
  "outl-desktop/src/lib/page-change.ts":
    "installPageChangeEffect must re-run on every reply — it repairs a " +
    "selection whose block left the tree when the outline lands — and only " +
    "*behaves* differently on a real page change, which its own lastPageId " +
    "decides. A key would silence the re-run it depends on. See the " +
    "'when not to reach for it' note in page-key.ts.",
};

/**
 * Files that call `pageBacklinks` for a reason that is **not** "the page
 * changed". The reason has to name what re-asks instead.
 */
const DELIBERATE_REFETCH: Record<string, string> = {
  "outl-desktop/src/components/AppShell.tsx":
    "refreshActivePage — a peer edited another page, which can change this " +
    "page's backlinks while its key stays identical. Keeps its rows up " +
    "(no emptyBacklinks) and carries the pending mark instead.",
  "outl-desktop/src/components/InlineBacklinks.tsx":
    "toggleBacklinkTodo (#144) — the mutation lands on the *source* page, " +
    "so this page's key does not change and nothing else would re-ask.",
};

/**
 * A tracked source built from a page field: `on(…)` deps (single or
 * array) and `createResource` sources. These compare nothing, and the
 * object behind them is replaced on every reply.
 */
const TRACKS_A_PAGE_FIELD =
  /(?:createEffect\(\s*on|\bon|createResource)\(\s*\[?\s*\(\)\s*=>[^;]{0,120}?\bpage[?.]*\.(?:slug|id|title)\b/;

/**
 * Comments are prose, and the prose here is *about* the broken shape —
 * `page-key.ts` and `backlinks-effect.ts` both quote it to explain what
 * went wrong. Scanning them would make the rule unwritable.
 */
function stripComments(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/.*$/gm, "$1");
}

function tsFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) {
      tsFiles(path, out);
      continue;
    }
    if (/\.tsx?$/.test(entry) && !/\.test\.tsx?$/.test(entry)) out.push(path);
  }
  return out;
}

function scan(
  client: string,
  predicate: (source: string) => boolean,
  declared: Record<string, string>,
): string[] {
  const root = join(import.meta.dirname, "..", "..", "..", client, "src");
  const offenders: string[] = [];
  for (const path of tsFiles(root)) {
    const rel = `${client}/src${path.slice(root.length)}`;
    if (rel in declared) continue;
    if (predicate(stripComments(readFileSync(path, "utf8")))) offenders.push(rel);
  }
  return offenders;
}

describe("every client asks whether the page changed through createPageKey", () => {
  for (const client of CLIENTS) {
    it(`${client} builds no tracked source from a raw page field`, () => {
      // The shape is the rule, on its own. Pairing it with "but the
      // file imports createPageKey" is what a first version did, and it
      // passed against a file holding both the import and the raw
      // source — which is the regression, not the fix.
      const offenders = scan(client, (s) => TRACKS_A_PAGE_FIELD.test(s), DELIBERATE_TRACKING);

      expect(
        offenders,
        [
          "A tracked source built from `page.slug` / `page.id` / `page.title`",
          "re-fires on every reply: `on()` compares nothing, and a mutation",
          "assigns a fresh PageMeta. Use `createPageKey` (@outl/shared/navigation),",
          "or add a row to DELIBERATE_TRACKING saying why a key would be wrong.",
        ].join(" "),
      ).toEqual([]);
    });

    it(`${client} fetches backlinks only when the backlinks key changed`, () => {
      const offenders = scan(
        client,
        (source) =>
          source.includes("pageBacklinks(") && !source.includes("createBacklinksKey"),
        DELIBERATE_REFETCH,
      );

      expect(
        offenders,
        [
          "These files fetch backlinks without the shared key.",
          "Either source the fetch from `createBacklinksKey(() => page)`,",
          "or add a row to DELIBERATE_REFETCH saying what re-asks instead.",
        ].join(" "),
      ).toEqual([]);
    });

    it(`${client} is still where this test thinks it is`, () => {
      // A client renamed or moved would otherwise silently stop being
      // checked, and both lists above would read as satisfied.
      expect(() =>
        statSync(join(import.meta.dirname, "..", "..", "..", client, "src")),
      ).not.toThrow();
    });
  }

  it("every declared exception names a file that exists", () => {
    const root = join(import.meta.dirname, "..", "..", "..");
    for (const rel of [
      ...Object.keys(DELIBERATE_TRACKING),
      ...Object.keys(DELIBERATE_REFETCH),
    ]) {
      expect(() => statSync(join(root, rel)), `stale exception: ${rel}`).not.toThrow();
    }
  });
});
