import { describe, expect, it } from "vitest";

import type { BlockNode, InlineToken } from "../api/types";
import { collectPageEmbedNames, pageEmbedOnlyName } from "./page-embed";

function node(id: string, tokens: InlineToken[], children: BlockNode[] = []) {
  return {
    id,
    text: id,
    tokens,
    todo: null,
    collapsed: false,
    properties: [],
    children,
  } as BlockNode;
}

const embed = (name: string): InlineToken => ({
  kind: "pageembed",
  value: name,
});

describe("pageEmbedOnlyName", () => {
  it("returns the name of a lone page embed", () => {
    expect(pageEmbedOnlyName([embed("Projeto X")])).toBe("Projeto X");
  });

  it("ignores whitespace-only plain tokens around it", () => {
    // The whole point: a trailing space after `![[page]]` must not stop
    // the expansion, because the user cannot see it.
    expect(
      pageEmbedOnlyName([
        { kind: "plain", value: "  " },
        embed("Projeto X"),
        { kind: "plain", value: "\t" },
      ]),
    ).toBe("Projeto X");
  });

  it("returns null for mixed prose", () => {
    // The block keeps its one-line title render instead: expanding a
    // page under a sentence would bury the sentence.
    expect(
      pageEmbedOnlyName([
        { kind: "plain", value: "see " },
        embed("Projeto X"),
      ]),
    ).toBeNull();
  });

  it("returns null for two page embeds", () => {
    expect(pageEmbedOnlyName([embed("A"), embed("B")])).toBeNull();
  });

  it("returns null for a plain page ref without the `!`", () => {
    expect(pageEmbedOnlyName([{ kind: "ref", value: "Projeto X" }])).toBeNull();
  });

  it("returns null for a block embed, which is the other owner", () => {
    // `embedOnlyHandle` answers for `!((blk-…))`; neither function
    // claims the other's token, so a block holding both is mixed prose
    // to both of them.
    expect(pageEmbedOnlyName([{ kind: "embed", value: "blk-aaaaaa" }])).toBeNull();
  });

  it("returns null for an empty token list", () => {
    expect(pageEmbedOnlyName([])).toBeNull();
  });
});

describe("collectPageEmbedNames", () => {
  it("collects names across the whole outline, depth first", () => {
    const outline = [
      node("a", [embed("First")], [node("a1", [embed("Second")])]),
      node("b", [embed("Third")]),
    ];
    expect(collectPageEmbedNames(outline)).toEqual([
      "First",
      "Second",
      "Third",
    ]);
  });

  it("de-dupes a name used on several blocks", () => {
    // One round-trip asks about each name once; the reply is a map, so
    // a repeat would only cost the backend a second lookup.
    const outline = [
      node("a", [embed("Projeto X")]),
      node("b", [embed("Projeto X")], [node("b1", [embed("Projeto X")])]),
    ];
    expect(collectPageEmbedNames(outline)).toEqual(["Projeto X"]);
  });

  it("descends into bold / italic / strike / highlight inner tokens", () => {
    // A page embed inside `**bold**` arrives as `Bold { inner: [...] }`,
    // so a walk over the top-level tokens alone would miss it and the
    // block would render its literal forever.
    const outline = [
      node("a", [{ kind: "bold", inner: [embed("Bold")] }]),
      node("b", [{ kind: "italic", inner: [embed("Italic")] }]),
      node("c", [{ kind: "strike", inner: [embed("Strike")] }]),
      node("d", [
        { kind: "highlight", inner: [{ kind: "bold", inner: [embed("Deep")] }] },
      ]),
    ];
    expect(collectPageEmbedNames(outline)).toEqual([
      "Bold",
      "Italic",
      "Strike",
      "Deep",
    ]);
  });

  it("collects a page embed sitting in mixed prose", () => {
    // Resolving is not gated on the expansion rule: the title still has
    // to render, so the name still has to be asked about.
    const outline = [
      node("a", [{ kind: "plain", value: "see " }, embed("Projeto X")]),
    ];
    expect(collectPageEmbedNames(outline)).toEqual(["Projeto X"]);
  });

  it("ignores block refs and page refs", () => {
    const outline = [
      node("a", [
        { kind: "ref", value: "Projeto X" },
        { kind: "blockref", value: "blk-aaaaaa" },
        { kind: "embed", value: "blk-bbbbbb" },
      ]),
    ];
    expect(collectPageEmbedNames(outline)).toEqual([]);
  });

  it("drops an empty name", () => {
    // It can only ever resolve to nothing, and asking would key the
    // reply map on `""`.
    expect(collectPageEmbedNames([node("a", [embed("")])])).toEqual([]);
  });
});
