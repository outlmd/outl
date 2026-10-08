import { describe, expect, it } from "vitest";

import {
  choosePasteRoute,
  hasMultipleParagraphs,
  htmlToOutlMarkdown,
  looksLikeOutline,
  looksLikeTable,
  looksLikeTabular,
  utf16OffsetToCharOffset,
} from "./index";

describe("hasMultipleParagraphs", () => {
  it("is false for a single line", () => {
    expect(hasMultipleParagraphs("just one line")).toBe(false);
    expect(hasMultipleParagraphs("https://example.com/x")).toBe(false);
  });

  it("is true for multiple non-blank lines (single or blank separators)", () => {
    // A chat reply arrives one line per paragraph, `\n`-separated.
    expect(hasMultipleParagraphs("line a\nline b\nline c")).toBe(true);
    expect(hasMultipleParagraphs("para one\n\npara two")).toBe(true);
  });

  it("ignores leading / trailing blank lines", () => {
    expect(hasMultipleParagraphs("\n\nsolo\n\n")).toBe(false);
  });

  it("treats whitespace-only lines as blank (not a second paragraph)", () => {
    expect(hasMultipleParagraphs("solo\n   \n\t")).toBe(false);
    expect(hasMultipleParagraphs("a\n   \nb")).toBe(true);
  });

  it("counts CRLF-separated lines (Windows clipboards)", () => {
    expect(hasMultipleParagraphs("line a\r\nline b")).toBe(true);
    expect(hasMultipleParagraphs("solo\r\n")).toBe(false);
  });

  it("is true at exactly two non-blank lines (the boundary)", () => {
    expect(hasMultipleParagraphs("one\ntwo")).toBe(true);
  });
});

describe("looksLikeOutline", () => {
  it("returns false for empty input", () => {
    expect(looksLikeOutline("")).toBe(false);
  });

  it("returns false for plain text", () => {
    expect(looksLikeOutline("just one line of text")).toBe(false);
    expect(looksLikeOutline("multi\nline\nbut no bullets")).toBe(false);
  });

  it("returns true on a single bullet line", () => {
    expect(looksLikeOutline("- one bullet")).toBe(true);
  });

  it("returns true when the bullet is indented", () => {
    expect(looksLikeOutline("    - nested")).toBe(true);
    expect(looksLikeOutline("\t- tab-indented")).toBe(true);
  });

  it("returns true when bullets appear after non-bullet preface", () => {
    expect(looksLikeOutline("intro paragraph\n- bullet")).toBe(true);
  });

  it("ignores leading whitespace lines", () => {
    expect(looksLikeOutline("\n\n  \n- after blanks")).toBe(true);
  });

  it("treats an empty bullet marker as outline", () => {
    expect(looksLikeOutline("-")).toBe(true);
    expect(looksLikeOutline("  -")).toBe(true);
  });

  it("returns false for dash followed by non-space", () => {
    expect(looksLikeOutline("-foo")).toBe(false);
    expect(looksLikeOutline("hyphen-word")).toBe(false);
  });
});

describe("utf16OffsetToCharOffset", () => {
  it("returns 0 for offset 0", () => {
    expect(utf16OffsetToCharOffset("anything", 0)).toBe(0);
    expect(utf16OffsetToCharOffset("", 0)).toBe(0);
  });

  it("matches the UTF-16 offset for pure ASCII", () => {
    const s = "hello world";
    expect(utf16OffsetToCharOffset(s, 5)).toBe(5);
    expect(utf16OffsetToCharOffset(s, s.length)).toBe(s.length);
  });

  it("matches the UTF-16 offset for BMP text", () => {
    // pt-BR with accents — `á` is U+00E1, still BMP, one code unit.
    const s = "olá mundo";
    expect(utf16OffsetToCharOffset(s, 4)).toBe(4); // after "olá "
    expect(utf16OffsetToCharOffset(s, s.length)).toBe(s.length);
  });

  it("collapses surrogate pairs to a single char", () => {
    // 😀 = U+1F600 — supplementary plane, takes 2 UTF-16 code units.
    const s = "hi 😀 you";
    expect(utf16OffsetToCharOffset(s, 5)).toBe(4);
    expect(s.length).toBe(9);
    expect(utf16OffsetToCharOffset(s, s.length)).toBe(8);
  });

  it("clamps when the offset overshoots", () => {
    const s = "abc";
    expect(utf16OffsetToCharOffset(s, 999)).toBe(3);
  });
});

describe("choosePasteRoute", () => {
  it("routes rich when HTML adds formatting the plain text lacks", () => {
    const d = choosePasteRoute("<b>bold</b> word", "bold word");
    expect(d).toEqual({ route: "rich", text: "**bold** word" });
  });

  it("does NOT round-trip when HTML is just a styled wrapper (md === plain)", () => {
    // A <span> with no markdown-visible formatting converts to the same
    // text as the plain flavour → not rich; a single line → native.
    const d = choosePasteRoute("<span>hello world</span>", "hello world");
    expect(d).toEqual({ route: "native" });
  });

  it("treats an alt-less image (md empty) as non-rich, falls to plain", () => {
    // md === "" → not rich; single-line plain → native.
    expect(choosePasteRoute('<img src="x.png">', "a url")).toEqual({
      route: "native",
    });
  });

  it("routes structured for a plain outline with no richer HTML", () => {
    expect(choosePasteRoute("", "- one\n- two")).toEqual({
      route: "structured",
      text: "- one\n- two",
    });
  });

  it("routes structured for multi-paragraph plain text", () => {
    const plain = "First line.\nSecond line.";
    expect(choosePasteRoute("", plain)).toEqual({
      route: "structured",
      text: plain,
    });
  });

  it("routes native for trivial single-line plain text", () => {
    expect(choosePasteRoute("", "https://example.com")).toEqual({
      route: "native",
    });
    expect(choosePasteRoute("", "")).toEqual({ route: "native" });
  });

  it("ignores trailing whitespace when comparing HTML vs plain", () => {
    // Plain has a trailing newline the HTML doesn't; the trimmed compare
    // must not flag it as rich when the content is identical.
    expect(choosePasteRoute("<span>hi</span>", "hi\n")).toEqual({
      route: "native",
    });
  });

  it("prefers rich over structured when both HTML and multi-paragraph plain exist", () => {
    const d = choosePasteRoute(
      "<p><b>H</b></p><p>body</p>",
      "H\nbody",
    );
    expect(d.route).toBe("rich");
    if (d.route === "rich") expect(d.text).toContain("**H**");
  });
});

// ---- tabular paste -----------------------------------------------------
//
// #329: tabular data pasted from anywhere becomes an outl table. These
// pin the client's half — the routing decision and the HTML conversion.
// The backend half (`outl_md::tsv_to_markdown`, `looks_like_table`) has
// its own tests in Rust; the mirrors below are what keep the two from
// disagreeing about whether a round trip even happens.

describe("looksLikeTable", () => {
  it("is true for a header and a delimiter row", () => {
    expect(looksLikeTable("| a | b |\n| --- | --- |")).toBe(true);
    expect(looksLikeTable("a | b\n--- | ---\n1 | 2")).toBe(true);
  });

  it("finds a table further down the payload", () => {
    expect(looksLikeTable("intro\n\n| a | b |\n| --- | --- |")).toBe(true);
  });

  it("is false for prose that merely carries a pipe", () => {
    // The false positive that would matter — this must stay on the
    // paragraph path.
    expect(looksLikeTable("run `a | b` in the shell")).toBe(false);
    expect(looksLikeTable("a | b\nc | d")).toBe(false);
    expect(looksLikeTable("plain words")).toBe(false);
    expect(looksLikeTable("")).toBe(false);
  });

  it("does not read a bullet as a row", () => {
    expect(looksLikeTable("- item | with a pipe\n- another | one")).toBe(false);
  });
});

describe("looksLikeTabular", () => {
  it("is true for tab-separated rows of equal width", () => {
    expect(looksLikeTabular("Route\tPax\nSP\t1203")).toBe(true);
  });

  it("is false for a ragged field count", () => {
    expect(looksLikeTabular("a\tb\nc\td\te")).toBe(false);
  });

  it("is false for a single line", () => {
    expect(looksLikeTabular("a\tb\tc")).toBe(false);
  });

  it("is false for tab-indented text", () => {
    // Code and outlines pass the width test; the leading tab is what
    // separates them from a grid.
    expect(looksLikeTabular("\tif x:\n\treturn y")).toBe(false);
  });

  it("is false without tabs", () => {
    expect(looksLikeTabular("a, b\nc, d")).toBe(false);
  });
});

describe("choosePasteRoute with tabular data", () => {
  it("routes a spreadsheet's plain flavour to the backend", () => {
    // Before this, two tab-separated rows reached the paragraph gate
    // and landed as one block per row.
    const route = choosePasteRoute("", "Route\tPax\nSP\t1203");
    expect(route.route).toBe("structured");
  });

  it("routes a single-line markdown table pair to the backend", () => {
    const route = choosePasteRoute("", "| a | b |\n| --- | --- |");
    expect(route.route).toBe("structured");
  });

  it("still leaves one plain sentence on the native splice", () => {
    expect(choosePasteRoute("", "just one sentence").route).toBe("native");
  });
});

describe("htmlToOutlMarkdown with a table", () => {
  it("converts a <table> to a markdown table", () => {
    const html =
      "<table><thead><tr><th>Route</th><th>Pax</th></tr></thead>" +
      "<tbody><tr><td>SP</td><td>1203</td></tr></tbody></table>";
    expect(htmlToOutlMarkdown(html)).toBe(
      "| Route | Pax |\n| --- | --- |\n| SP | 1203 |",
    );
  });

  it("keeps a cell's inline formatting", () => {
    const html =
      "<table><tr><th>k</th></tr><tr><td><b>bold</b> and <a href='https://x.dev'>link</a></td></tr></table>";
    expect(htmlToOutlMarkdown(html)).toContain(
      "| **bold** and [link](https://x.dev) |",
    );
  });

  it("reads each column's alignment off the header", () => {
    const html =
      "<table><tr><th style='text-align:right'>n</th><th align='center'>m</th></tr>" +
      "<tr><td>1</td><td>2</td></tr></table>";
    expect(htmlToOutlMarkdown(html)).toContain("| ---: | :---: |");
  });

  it("escapes a pipe inside a cell", () => {
    const html = "<table><tr><th>k</th></tr><tr><td>a | b</td></tr></table>";
    expect(htmlToOutlMarkdown(html)).toContain("| a \\| b |");
  });

  it("flattens a multi-line cell to one line", () => {
    // A markdown row cannot carry a newline; a spreadsheet cell can.
    const html =
      "<table><tr><th>k</th></tr><tr><td>first<br>second</td></tr></table>";
    const md = htmlToOutlMarkdown(html);
    expect(md).toContain("| first second |");
  });

  it("pads a short row so later cells stay under their header", () => {
    const html =
      "<table><tr><th>a</th><th>b</th><th>c</th></tr><tr><td>1</td></tr></table>";
    expect(htmlToOutlMarkdown(html)).toContain("| 1 |  |  |");
  });

  it("expands a merged cell into the columns it spans", () => {
    const html =
      "<table><tr><th>a</th><th>b</th></tr><tr><td colspan='2'>wide</td></tr></table>";
    expect(htmlToOutlMarkdown(html)).toContain("| wide |  |");
  });

  it("produces a payload the table detector claims", () => {
    // The two halves have to meet: whatever the HTML conversion emits
    // must be something `looksLikeTable` (and the Rust parser) reads as
    // a table, or the paste lands as one block per row.
    const html = "<table><tr><th>a</th><th>b</th></tr><tr><td>1</td><td>2</td></tr></table>";
    expect(looksLikeTable(htmlToOutlMarkdown(html))).toBe(true);
  });

  it("leaves an empty table out entirely", () => {
    expect(htmlToOutlMarkdown("<table></table>")).toBe("");
  });

  it("does not donate a nested table's rows to the outer grid", () => {
    // An HTML email or a legacy layout table. `querySelectorAll` matches
    // every descendant, so the inner rows landed in the outer grid AND
    // were flattened into the cell containing them.
    const html =
      "<table><tr><th>outer</th></tr>" +
      "<tr><td><table><tr><td>inner1</td><td>inner2</td></tr></table></td></tr></table>";
    const md = htmlToOutlMarkdown(html);
    // Three lines: header, rule, one body row. Not four.
    expect(md.split("\n")).toHaveLength(3);
    expect(md).not.toContain("| inner1 | inner2 |");
  });

  it("escapes a pipe that already has a backslash in front of it", () => {
    // Turndown escapes `\` in a text node but not inside `<code>`, so a
    // code cell arrives with one backslash. Escaping the pipe blindly
    // made an even run, which the parser reads as a real delimiter and
    // the cell splits in two.
    const html =
      "<table><tr><th>k</th><th>v</th></tr>" +
      "<tr><td><code>a\\|b</code></td><td>c</td></tr></table>";
    const md = htmlToOutlMarkdown(html);
    const body = md.split("\n")[2];
    // Two cells, not three: one `|` between them.
    expect(body.split(/(?<!\\)\|/).filter((s) => s.trim() !== "")).toHaveLength(2);
  });

  it("clamps a merged cell at the colspan ceiling", () => {
    // `getAttribute` returns the literal attribute, so this asked for a
    // 16 GB array before the clamp.
    const html =
      '<table><tr><td colspan="50000000">x</td></tr><tr><td>y</td></tr></table>';
    const md = htmlToOutlMarkdown(html);
    // 1 cell + 999 padded ones = 1000 columns, the spec's own ceiling.
    const cells = md.split("\n")[0].split("|").slice(1, -1);
    expect(cells).toHaveLength(1000);
  });
});
