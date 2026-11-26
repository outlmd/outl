/**
 * A "Linked from" row draws its block with the outline's own read
 * branch, `<BlockReadBody>`, and nothing else — the mobile twin of the
 * desktop's `BacklinkRow.parity.test.ts`.
 *
 * The row used to carry its own "table, else inline tokens" renderer,
 * so a fence read as a line of raw backticks and a quote, several
 * lines, the property line and a page embed came out differently from
 * the block's own page. A source-read, because a second renderer that
 * comes back draws plausibly enough that only a block it does not know
 * would show it. `BacklinksSection.render.test.tsx` covers what the one
 * renderer does.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

function source(file: string): string {
  return readFileSync(join(import.meta.dirname, file), "utf8");
}

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

const RENDERERS = ["MarkdownInline", "MarkdownTable", "EmbeddedSubtree"];

describe("a block has one read renderer on mobile", () => {
  it.each(["BacklinksSection.tsx", "BlockBody.tsx"])(
    "%s renders the block through BlockReadBody",
    (file) => {
      const src = source(file);
      expect(src).toMatch(/import \{ BlockReadBody \} from "\.\/BlockReadBody"/);
      expect(src).toContain("<BlockReadBody");
    },
  );

  it.each(
    ["BacklinksSection.tsx", "BlockBody.tsx"].flatMap((file) =>
      RENDERERS.map((name) => [file, name] as const),
    ),
  )("%s does not draw a block itself with %s", (file, name) => {
    const src = source(file);
    expect(markdownImports(src)).not.toContain(name);
    expect(src).not.toContain(`<${name}`);
  });
});
