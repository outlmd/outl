import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it, vi } from "vitest";

import { createTransclusionOps } from "./outline-transclusions";

import type { BlockNode } from "@outl/shared/api/types";

function block(id: string, tokens: BlockNode["tokens"]): BlockNode {
  return {
    id,
    text: "",
    tokens,
    children: [],
    collapsed: false,
    todo: null,
    properties: {},
  } as unknown as BlockNode;
}

function deps(slug = "2026-11-17") {
  const resolveEmbeds = vi.fn().mockResolvedValue({});
  const resolvePageEmbeds = vi.fn().mockResolvedValue({});
  const setEmbeds = vi.fn();
  const setPageEmbeds = vi.fn();
  return {
    resolveEmbeds,
    resolvePageEmbeds,
    setEmbeds,
    setPageEmbeds,
    slug: () => slug,
  };
}

const pageEmbed = (value: string) =>
  block(`b-${value}`, [{ kind: "pageembed", value }] as BlockNode["tokens"]);

describe("createTransclusionOps", () => {
  // The defect this module was extracted for. The resolve used to hang
  // off `<OutlineView />`'s `applyView`, which only runs on a mutation;
  // `<AppShell />` has its own `applyView` for every navigation the
  // chrome owns, boot included. So an *opened* page never asked, and
  // every transclusion rendered its unresolved fallback — pixels
  // identical to "that page does not exist".
  it("resolves on the first outline it is given, with no mutation first", () => {
    const d = deps();
    createTransclusionOps(d).resolve([pageEmbed("buser/team/people/beneficios")]);

    expect(d.resolvePageEmbeds).toHaveBeenCalledWith([
      "buser/team/people/beneficios",
    ]);
  });

  it("carries a namespaced name through verbatim", () => {
    // `/` survives only in `title::`; the slug is `-`-folded. Folding
    // here would hand the resolver a slug and lose the title rung.
    const d = deps();
    createTransclusionOps(d).resolve([pageEmbed("os/linux/debian")]);

    expect(d.resolvePageEmbeds).toHaveBeenCalledWith(["os/linux/debian"]);
  });

  it("skips a repeat of the same outline on the same page", () => {
    const d = deps();
    const ops = createTransclusionOps(d);
    const outline = [pageEmbed("infra")];

    ops.resolve(outline);
    ops.resolve(outline);

    expect(d.resolvePageEmbeds).toHaveBeenCalledTimes(1);
  });

  it("resolves again when a name is added", () => {
    const d = deps();
    const ops = createTransclusionOps(d);

    ops.resolve([pageEmbed("infra")]);
    ops.resolve([pageEmbed("infra"), pageEmbed("ops")]);

    expect(d.resolvePageEmbeds).toHaveBeenCalledTimes(2);
    expect(d.resolvePageEmbeds).toHaveBeenLastCalledWith(["infra", "ops"]);
  });

  it("resolves again on a navigation, even when the names match", () => {
    // The source page's own text may have been edited elsewhere.
    let slug = "a";
    const d = { ...deps(), slug: () => slug };
    const ops = createTransclusionOps(d);
    const outline = [pageEmbed("infra")];

    ops.resolve(outline);
    slug = "b";
    ops.resolve(outline);

    expect(d.resolvePageEmbeds).toHaveBeenCalledTimes(2);
  });

  it("keeps the two kinds' skip keys apart", () => {
    // A newly typed `[[ref]]` must not invalidate the page-embed
    // answer, and vice versa.
    const d = deps();
    const ops = createTransclusionOps(d);

    ops.resolve([pageEmbed("infra")]);
    ops.resolve([
      pageEmbed("infra"),
      block("b2", [
        { kind: "blockref", value: "blk-r6s4a1" },
      ] as BlockNode["tokens"]),
    ]);

    expect(d.resolveEmbeds).toHaveBeenCalledTimes(1);
    expect(d.resolvePageEmbeds).toHaveBeenCalledTimes(1);
  });

  it("asks for nothing when the page transcludes nothing", () => {
    const d = deps();
    createTransclusionOps(d).resolve([
      block("b1", [{ kind: "plain", value: "just prose" }] as BlockNode["tokens"]),
    ]);

    expect(d.resolveEmbeds).not.toHaveBeenCalled();
    expect(d.resolvePageEmbeds).not.toHaveBeenCalled();
  });

  it("leaves the previous map in place when a resolve rejects", async () => {
    const d = deps();
    d.resolvePageEmbeds.mockRejectedValue(new Error("command not found"));

    createTransclusionOps(d).resolve([pageEmbed("infra")]);
    await Promise.resolve();

    expect(d.setPageEmbeds).not.toHaveBeenCalled();
  });
});

/**
 * The module above always worked. The defect was the **wiring**: who
 * calls it. A unit test of this module structurally cannot catch that,
 * so these read the source, the way `lib/page-view.test.ts` pins its
 * own "nobody writes this twice" rule.
 *
 * Two wrong wirings were shipped before this landed, and each test
 * below denies one of them.
 */
describe("who calls the resolve", () => {
  const read = (rel: string) =>
    readFileSync(join(import.meta.dirname, rel), "utf8");

  it("resolves from applyPageState, the single writer of the outline", () => {
    // `<OutlineView />`'s `applyView` is not that writer: `<AppShell />`
    // has its own for every navigation the chrome owns, boot included,
    // so a page that was *opened* never asked and every transclusion
    // showed its unresolved fallback.
    const src = read("./page-view.ts");
    const body = src.slice(src.indexOf("export function applyPageState"));
    expect(body).toContain("transclusionOps().resolve(view.outline)");
  });

  it("does not drive the resolve off an effect on the outline", () => {
    // `setOutline` writes through `reconcile(…, { key: "id" })`, which
    // patches in place, so an effect tracking the top-level reference
    // fires once and never again.
    const src = read("../components/OutlineView.tsx");
    expect(src).not.toContain("() => appState.outline");
  });
});
