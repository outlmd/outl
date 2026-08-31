/**
 * Parity gate between the shared catalog and this client's handler map.
 *
 * `outl_shortcuts::support` declares, per action, what the desktop
 * does with it. `buildHandlers` decides what it *actually* does. Two
 * declarations of one fact drift, and this is the assertion that
 * makes them drift loudly instead of silently:
 *
 * - Catalog says `full` / `partial` → a handler must exist. Without
 *   this, the catalog promises the user a behaviour the client never
 *   wired, which is exactly what `docs/shortcuts.md` did for `y r`
 *   and `:` — both documented as desktop chords, neither handled,
 *   both dead keys.
 * - Catalog says `missing` / `n/a` / `native` → a handler must NOT
 *   exist. A handler here means someone shipped the feature and left
 *   the catalog saying it is absent, so the user gets told the thing
 *   they just used isn't available.
 *
 * The catalog is read from `docs/client-parity.md`, which is
 * generated from `support.rs` and pinned by
 * `the_parity_doc_matches_the_code` on the Rust side. So this test
 * transitively checks the code, not the prose.
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import { buildHandlers } from "./action-handlers";

/** `full` and `partial` are the two states that owe a handler. */
type Expectation = "handler" | "no-handler";

function desktopColumn(): Map<string, { expect: Expectation; cell: string }> {
  // vitest runs with `crates/outl-desktop` as cwd (its vite config
  // lives there), so the repo root is two levels up. `import.meta.url`
  // is not a `file:` URL under the happy-dom environment.
  const path = resolve(process.cwd(), "../../docs/client-parity.md");
  const doc = readFileSync(path, "utf8");

  const begin = doc.indexOf("<!-- BEGIN GENERATED: client-parity -->");
  const end = doc.indexOf("<!-- END GENERATED: client-parity -->");
  expect(begin, "docs/client-parity.md lost its generated block").toBeGreaterThan(-1);
  expect(end).toBeGreaterThan(begin);

  const out = new Map<string, { expect: Expectation; cell: string }>();
  for (const line of doc.slice(begin, end).split("\n")) {
    // `| \`ActionName\` | tui | desktop | mobile |`
    const m = /^\|\s*`([A-Za-z]+)`\s*\|([^|]*)\|([^|]*)\|/.exec(line);
    if (!m) continue;
    const [, action, , desktopRaw] = m;
    const cell = desktopRaw.trim();
    // `✅ _native — …_` is reachable but has no handler by
    // definition, so it groups with the absent states, not with `✅`.
    const isFull = cell.startsWith("✅") && !cell.includes("_native");
    const isPartial = cell.startsWith("⚠️");
    out.set(action, {
      expect: isFull || isPartial ? "handler" : "no-handler",
      cell,
    });
  }
  return out;
}

describe("desktop handler map vs the shared support catalog", () => {
  const support = desktopColumn();
  const handlers = buildHandlers({
    applyView: () => {},
    setError: () => {},
  });
  const wired = new Set(Object.keys(handlers));

  it("reads a non-empty desktop column", () => {
    // A parsing regression here would make every assertion below
    // vacuously pass, which is a worse outcome than a red test.
    expect(support.size).toBeGreaterThan(70);
  });

  it("wires a handler for every action the catalog says it supports", () => {
    const promised = [...support.entries()]
      .filter(([, v]) => v.expect === "handler")
      .map(([action]) => action)
      .filter((action) => !wired.has(action));

    expect(
      promised,
      "the catalog promises the desktop performs these, but no handler is wired — " +
        "either add the handler or change the row in crates/outl-shortcuts/src/support.rs",
    ).toEqual([]);
  });

  it("wires no handler for an action the catalog calls absent", () => {
    const surprises = [...wired]
      .filter((action) => support.get(action)?.expect === "no-handler")
      .map((action) => `${action} (catalog: ${support.get(action)?.cell})`);

    expect(
      surprises,
      "these have handlers but the catalog tells the user they are unavailable — " +
        "update crates/outl-shortcuts/src/support.rs to match what shipped",
    ).toEqual([]);
  });

  it("has no handler for an action outside the catalog", () => {
    const unknown = [...wired].filter((action) => !support.has(action));
    expect(
      unknown,
      "handler for an action the catalog doesn't know — a typo in the key, " +
        "or a variant removed from the Rust enum",
    ).toEqual([]);
  });
});
