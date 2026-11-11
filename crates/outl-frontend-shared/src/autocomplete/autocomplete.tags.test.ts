/**
 * The `#tag` trigger (issue 15), and the two questions it cannot be
 * tested without: which trigger wins when several could fire, and
 * where a tag name ends.
 *
 * Split out of `autocomplete.test.ts` when that file crossed the
 * 600-line ratchet. The seam is the trigger itself, not an arbitrary
 * cut: `#` is the only opener with no closing delimiter *and* a
 * cross-runtime alphabet to honour, so its cases read together.
 */

import { describe, expect, it } from "vitest";

import { applySuggestion, detectRefContext, refReplacement } from "./index";

describe("detectRefContext — tags", () => {
  it("detects a tag after a word-initial #", () => {
    const ctx = detectRefContext("note #wor", 9);
    expect(ctx).toEqual({
      kind: "tag",
      query: "wor",
      openIndex: 5,
      replaceEnd: 9,
    });
  });

  it("detects a tag at the very start of a block", () => {
    expect(detectRefContext("#wor", 4)?.kind).toBe("tag");
  });

  it("detects a bare # with an empty query", () => {
    // The empty query is what lists the workspace's tags, the way an
    // empty `((` lists the newest blocks.
    expect(detectRefContext("note #", 6)).toEqual({
      kind: "tag",
      query: "",
      openIndex: 5,
      replaceEnd: 6,
    });
  });

  it("does not fire mid-word", () => {
    // `a#b` is not a tag — the same trap `@` has in an email address.
    expect(detectRefContext("a#b", 3)).toBeNull();
  });

  it("does not fire on a markdown heading", () => {
    // A heading is `# ` with a space. A tag has no space after the #,
    // so the space is what tells them apart.
    expect(detectRefContext("# heading", 9)).toBeNull();
  });

  it("does not fire inside a code fence", () => {
    // A shell comment is not a tag. An odd number of fences before the
    // caret means the caret is inside one.
    expect(detectRefContext("```sh\n#!/bin/sh", 15)).toBeNull();
  });

  it("fires again after a fence closes", () => {
    expect(detectRefContext("```sh\necho\n```\n#wor", 19)?.kind).toBe("tag");
  });

  it("stops at whitespace, so a tag query holds no spaces", () => {
    // Unlike a mention, which allows `@Thiago Avelino`.
    expect(detectRefContext("#one two", 8)).toBeNull();
  });

  it("leaves [[ and (( alone", () => {
    expect(detectRefContext("[[pag", 5)?.kind).toBe("page");
    expect(detectRefContext("((blk", 5)?.kind).toBe("block");
  });
});

describe("detectRefContext — trigger precedence", () => {
  it("leaves a # inside an open [[ ref to the page ref (#15)", () => {
    // `[[Issue #42` — the `#` is part of the page's title. A tag
    // pre-pass that claimed it spliced from the `#`, so accepting
    // wrote `[[Issue #deep-work]]`: a ref to a page that does not
    // exist, with the typed title destroyed. Any page whose title
    // holds a `#` after a space was uncompletable.
    const ctx = detectRefContext("[[Issue #42", 11);
    expect(ctx?.kind).toBe("page");
    expect(ctx?.query).toBe("Issue #42");
    expect(ctx?.openIndex).toBe(0);
  });

  it("leaves a # inside an open (( ref to the block ref", () => {
    expect(detectRefContext("((blk-a#b", 9)?.kind).toBe("block");
  });

  it("still reads a tag after a ref that is already closed", () => {
    // The bracket only wins while it is *open*. `[[done]] #wor` is a
    // finished ref followed by a tag.
    const ctx = detectRefContext("[[done]] #wor", 13);
    expect(ctx?.kind).toBe("tag");
    expect(ctx?.query).toBe("wor");
  });

  it("gives a freshly typed # to the tag, not to an earlier @ (#15)", () => {
    // The mention walk does not stop at whitespace (composite names),
    // so running it first let any `@` in the preceding 64 chars
    // swallow the `#` under the caret: the client searched persons for
    // "ana about #pro", got nothing, and the tag popup silently never
    // opened. The tag walk ends at the first non-tag char, so when it
    // matches it is always the closer trigger.
    const ctx = detectRefContext("ping @ana about #pro", 20);
    expect(ctx?.kind).toBe("tag");
    expect(ctx?.query).toBe("pro");
  });

  it("still reads a mention when the caret is in the @ word", () => {
    const ctx = detectRefContext("ping @ana про", 13);
    expect(ctx?.kind).toBe("mention");
    expect(ctx?.query).toBe("ana про");
  });
});

describe("detectRefContext — the tag alphabet mirrors the tokenizer", () => {
  // `outl_md::reference::try_tag` ends a tag name at the first char
  // outside `alphanumeric | - | _ | /`, and the TUI's `detect_trigger`
  // gates on the same set. Offering a popup past that boundary offers
  // to complete a tag the parser would end earlier.
  it.each([".", ",", "!", ":", "?", "'", "+"])(
    "refuses a tag query containing %s",
    (ch) => {
      const value = `#foo${ch}bar`;
      expect(detectRefContext(value, value.length)).toBeNull();
    },
  );

  it("accepts the four non-alphanumeric chars a tag name may hold", () => {
    for (const q of ["a-b", "a_b", "a/b/c", "a1"]) {
      const value = `#${q}`;
      expect(detectRefContext(value, value.length)?.query).toBe(q);
    }
  });

  it("accepts a non-ASCII tag name, like the tokenizer does", () => {
    // Rust's `is_alphanumeric()` is Unicode, not ASCII.
    expect(detectRefContext("#reunião", 8)?.query).toBe("reunião");
  });
});

describe("applySuggestion — tags", () => {
  it("replaces the typed tag with the chosen slug, no brackets", () => {
    const value = "note #wor";
    const ctx = detectRefContext(value, 9)!;
    expect(applySuggestion(value, ctx, "work")).toEqual({
      value: "note #work",
      caret: 10,
    });
  });

  it("keeps the text after the caret", () => {
    const value = "a #wo b";
    const ctx = detectRefContext(value, 5)!;
    expect(applySuggestion(value, ctx, "work").value).toBe("a #work b");
  });
});

describe("refReplacement — tags", () => {
  const page = { kind: "page" as const, slug: "deep-work", title: "Deep Work" };

  it("inserts the slug, because a tag ends at the first space", () => {
    expect(refReplacement(page, { tag: true })).toBe("deep-work");
  });

  it("still inserts the title for a plain page ref", () => {
    expect(refReplacement(page)).toBe("Deep Work");
  });

  it("inserts the slug for a journal, tag or not", () => {
    const j = { kind: "journal" as const, slug: "2026-11-11", title: "Nov 11" };
    expect(refReplacement(j)).toBe("2026-11-11");
    expect(refReplacement(j, { tag: true })).toBe("2026-11-11");
  });
});
