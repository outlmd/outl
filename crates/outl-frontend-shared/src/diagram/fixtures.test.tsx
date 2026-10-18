import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it } from "vitest";

import { DiagramFence } from "./DiagramFence";

const docsDir = join(process.cwd(), "../../docs");

function mermaidBodies(markdown: string): string[] {
  const bodies: string[] = [];
  const re = /```mermaid\n([\s\S]*?)```/g;
  for (const match of markdown.matchAll(re)) {
    bodies.push(match[1].replace(/\n$/, ""));
  }
  return bodies;
}

function fencesInDocs(): { file: string; source: string }[] {
  const files = readdirSync(docsDir).filter((name) => name.endsWith(".md"));
  const fences: { file: string; source: string }[] = [];
  for (const file of files) {
    const markdown = readFileSync(join(docsDir, file), "utf8");
    for (const source of mermaidBodies(markdown)) {
      fences.push({ file, source });
    }
  }
  return fences;
}

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

describe("docs mermaid fixtures", () => {
  const fences = fencesInDocs();

  it("finds the mermaid fences shipped in docs/", () => {
    expect(fences.length).toBeGreaterThan(0);
  });

  it.each(fences.map((fence, index) => [fence.file, index, fence.source] as const))(
    "renders %s fence %i",
    async (_file, _index, source) => {
      const host = document.createElement("div");
      document.body.appendChild(host);
      dispose = render(
        () => (
          <DiagramFence
            source={source}
            fallback={<pre data-fallback>{source}</pre>}
          />
        ),
        host,
      );
      const start = Date.now();
      while (Date.now() - start < 8000) {
        if (host.querySelector("svg")) return;
        if (host.querySelector("[data-diagram-error]")) {
          throw new Error(
            `${host.querySelector("[data-diagram-error]")?.textContent}\n${source.slice(0, 240)}`,
          );
        }
        await new Promise((r) => setTimeout(r, 25));
      }
      expect(host.querySelector("svg"), source.slice(0, 240)).not.toBeNull();
    },
    15000,
  );
});
