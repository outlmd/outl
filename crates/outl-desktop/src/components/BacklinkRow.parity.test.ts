/**
 * A "Linked from" row draws its block with the outline's `<BlockBody />`,
 * and nothing else.
 *
 * The row used to carry its own renderer — "table, else inline tokens",
 * with a hand-copied `!((blk))` expansion — and every shape it did not
 * know about came out wrong: a ```` ```call: ```` fence as one line of
 * raw backticks, a quote with no chrome, several lines run together,
 * properties and images missing, refs that did nothing. Each was a
 * feature the outline already had, drifting by omission.
 *
 * A source-read, in the shape of `navigation/page-key.parity.test.ts`:
 * the defect is which component the row imports, and a second renderer
 * that comes back draws plausibly enough that no behaviour test would
 * notice until someone writes a block it does not know.
 * `InlineBacklinks.render.test.tsx` covers what the one renderer does.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const source = readFileSync(join(import.meta.dirname, "BacklinkRow.tsx"), "utf8");

/** Every name imported from `@outl/shared/markdown`. */
function markdownImports(src: string): string[] {
  const names: string[] = [];
  for (const m of src.matchAll(/import\s*\{([^}]*)\}\s*from\s*"@outl\/shared\/markdown"/g)) {
    for (const raw of m[1].split(",")) {
      const name = raw.replace(/^\s*type\s+/, "").trim();
      if (name) names.push(name);
    }
  }
  return names;
}

describe("a backlink row has one renderer: the outline's", () => {
  it("BacklinkRow.tsx renders the block through BlockBody", () => {
    expect(source).toMatch(/import \{ BlockBody \} from "\.\/BlockBody"/);
    expect(source).toContain("<BlockBody");
  });

  it.each(["MarkdownInline", "MarkdownTable", "EmbeddedSubtree"])(
    "BacklinkRow.tsx does not draw a block itself with %s",
    (name) => {
      expect(markdownImports(source)).not.toContain(name);
      expect(source).not.toContain(`<${name}`);
    },
  );
});
