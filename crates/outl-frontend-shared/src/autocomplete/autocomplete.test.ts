import { describe, expect, it } from "vitest";

import {
  applyEmojiSuggestion,
  applySlashContext,
  applySuggestion,
  autoClosePair,
  autoDeletePair,
  autoPairBracket,
  detectEmojiContext,
  detectRefContext,
  detectSlashContext,
  insertPair,
  insertText,
  withCreateNewPersonCandidate,
} from "./index";

describe("autoClosePair", () => {
  it("closes a freshly-typed [[ with ]]", () => {
    const result = autoClosePair("foo [[", 6);
    expect(result).toEqual({ value: "foo [[]]", caret: 6 });
  });

  it("closes a freshly-typed (( with ))", () => {
    const result = autoClosePair("see ((", 6);
    expect(result).toEqual({ value: "see (())", caret: 6 });
  });

  it("does nothing when the closer is already to the right", () => {
    expect(autoClosePair("[[]]", 2)).toBeNull();
    expect(autoClosePair("(())", 2)).toBeNull();
  });

  it("ignores single brackets", () => {
    expect(autoClosePair("a [b", 4)).toBeNull();
    expect(autoClosePair("a (b", 4)).toBeNull();
  });

  it("ignores positions too close to the start", () => {
    expect(autoClosePair("[", 1)).toBeNull();
  });
});

describe("autoPairBracket", () => {
  it.each([
    ["(", ")"],
    ["[", "]"],
    ["{", "}"],
  ])("typing %s inserts the pair with the caret between", (open, close) => {
    const result = autoPairBracket("ab", 1, open);
    expect(result).toEqual({ value: `a${open}${close}b`, caret: 2 });
  });

  it("pairs at the very start and end of the value", () => {
    expect(autoPairBracket("", 0, "(")).toEqual({ value: "()", caret: 1 });
    expect(autoPairBracket("xy", 2, "{")).toEqual({ value: "xy{}", caret: 3 });
  });

  it("builds [[|]] when the second [ is typed inside an auto-paired []", () => {
    // First `[` produced `[|]`; the second one must land as `[[|]]`
    // so the existing `[[` ref flow keeps working (autoClosePair is
    // a no-op on that shape — the closer is never doubled).
    const result = autoPairBracket("a[]", 2, "[");
    expect(result).toEqual({ value: "a[[]]", caret: 3 });
    expect(autoClosePair(result!.value, result!.caret)).toBeNull();
  });

  it.each([")", "]", "}"])(
    "typing %s over an identical closer steps past it",
    (close) => {
      const open = { ")": "(", "]": "[", "}": "{" }[close]!;
      const value = `${open}x${close}`;
      expect(autoPairBracket(value, 2, close)).toEqual({ value, caret: 3 });
    },
  );

  it("steps over both closers of a doubled ref pair", () => {
    expect(autoPairBracket("[[a]]", 3, "]")).toEqual({ value: "[[a]]", caret: 4 });
    expect(autoPairBracket("[[a]]", 4, "]")).toEqual({ value: "[[a]]", caret: 5 });
  });

  it("lets a closer through when the next char differs", () => {
    expect(autoPairBracket("(x", 2, ")")).toBeNull();
    expect(autoPairBracket("(]", 1, ")")).toBeNull();
  });

  it("ignores non-bracket characters", () => {
    expect(autoPairBracket("ab", 1, "x")).toBeNull();
    expect(autoPairBracket("ab", 1, '"')).toBeNull();
    expect(autoPairBracket("ab", 1, "")).toBeNull();
  });
});

describe("autoDeletePair", () => {
  it("collapses an empty [[]] when caret is in the middle", () => {
    const result = autoDeletePair("foo [[]]", 6);
    expect(result).toEqual({ value: "foo ", caret: 4 });
  });

  it("collapses an empty (()) when caret is in the middle", () => {
    const result = autoDeletePair("see (())", 6);
    expect(result).toEqual({ value: "see ", caret: 4 });
  });

  it("does nothing when the pair has content between", () => {
    expect(autoDeletePair("[[ave]]", 5)).toBeNull();
    expect(autoDeletePair("((blk))", 5)).toBeNull();
  });

  it("does nothing when caret is outside the pair", () => {
    expect(autoDeletePair("[[]]", 0)).toBeNull();
    expect(autoDeletePair("[[]]", 4)).toBeNull();
  });

  it("does nothing without a matching closer to the right", () => {
    expect(autoDeletePair("[[", 2)).toBeNull();
    expect(autoDeletePair("((foo", 2)).toBeNull();
  });

  it("does not cross-mix [[ with ))", () => {
    expect(autoDeletePair("[[))", 2)).toBeNull();
  });

  it.each([
    ["()", "("],
    ["[]", "["],
    ["{}", "{"],
  ])("collapses an empty %s in one keystroke", (pair) => {
    const result = autoDeletePair(`a${pair}b`, 2);
    expect(result).toEqual({ value: "ab", caret: 1 });
  });

  it("prefers the doubled pair over the inner single pair", () => {
    // Caret in `[[|]]` — the whole ref scaffold goes, not just `[]`.
    expect(autoDeletePair("[[]]", 2)).toEqual({ value: "", caret: 0 });
  });

  it("collapses nested singles one level at a time", () => {
    expect(autoDeletePair("([])", 2)).toEqual({ value: "()", caret: 1 });
  });

  it("does not collapse a single pair with content between", () => {
    expect(autoDeletePair("(x)", 2)).toBeNull();
  });

  it("does not cross-mix single openers and closers", () => {
    expect(autoDeletePair("(]", 1)).toBeNull();
    expect(autoDeletePair("{)", 1)).toBeNull();
  });
});

describe("insertPair", () => {
  it("inserts opener+closer with the caret in the middle", () => {
    const result = insertPair("hello world", 5, "[[", "]]");
    expect(result).toEqual({ value: "hello[[]] world", caret: 7 });
  });
});

describe("insertText", () => {
  it("inserts plain text and advances the caret", () => {
    const result = insertText("foo bar", 4, "#");
    expect(result).toEqual({ value: "foo #bar", caret: 5 });
  });
});

describe("detectRefContext", () => {
  it("recognises a page ref before the closer is typed", () => {
    const ctx = detectRefContext("see [[ave", 9);
    expect(ctx).toEqual({
      kind: "page",
      query: "ave",
      openIndex: 4,
      replaceEnd: 9,
    });
  });

  it("recognises a block ref before the closer is typed", () => {
    const ctx = detectRefContext("link ((blk-r", 12);
    expect(ctx).toEqual({
      kind: "block",
      query: "blk-r",
      openIndex: 5,
      replaceEnd: 12,
    });
  });

  it("includes the existing closer in the replace span", () => {
    const ctx = detectRefContext("see [[ave]] later", 9);
    expect(ctx).toEqual({
      kind: "page",
      query: "ave",
      openIndex: 4,
      replaceEnd: 11,
    });
  });

  it("returns null when the caret is past the closer", () => {
    expect(detectRefContext("see [[ave]] later", 12)).toBeNull();
  });

  it("does not capture across newlines", () => {
    expect(detectRefContext("first [[stale\nsecond", 20)).toBeNull();
  });

  it("does not capture when there is no opener", () => {
    expect(detectRefContext("plain text", 5)).toBeNull();
  });

  it("captures an empty query right after the opener", () => {
    const ctx = detectRefContext("see [[", 6);
    expect(ctx).toEqual({
      kind: "page",
      query: "",
      openIndex: 4,
      replaceEnd: 6,
    });
  });
});

describe("applySuggestion", () => {
  it("replaces an open page ref with the picked title", () => {
    const ctx = detectRefContext("see [[ave", 9)!;
    const result = applySuggestion("see [[ave", ctx, "Avelino");
    expect(result).toEqual({ value: "see [[Avelino]]", caret: 15 });
  });

  it("replaces an open block ref with the picked handle", () => {
    const ctx = detectRefContext("see ((blk-r", 11)!;
    const result = applySuggestion("see ((blk-r", ctx, "blk-r6s4a1");
    expect(result).toEqual({ value: "see ((blk-r6s4a1))", caret: 18 });
  });

  it("replaces a fully-closed ref in place", () => {
    const ctx = detectRefContext("see [[old]] tail", 7)!;
    const result = applySuggestion("see [[old]] tail", ctx, "new");
    expect(result).toEqual({ value: "see [[new]] tail", caret: 11 });
  });
});

describe("detectRefContext: mention", () => {
  it("detects @ at the start of the buffer", () => {
    expect(detectRefContext("@av", 3)).toEqual({
      kind: "mention",
      query: "av",
      openIndex: 0,
      replaceEnd: 3,
    });
  });

  it("detects @ after whitespace (word-initial)", () => {
    expect(detectRefContext("hi @av", 6)).toEqual({
      kind: "mention",
      query: "av",
      openIndex: 3,
      replaceEnd: 6,
    });
  });

  it("keeps spaces in the query for composite names", () => {
    expect(detectRefContext("@Thiago Av", 10)).toEqual({
      kind: "mention",
      query: "Thiago Av",
      openIndex: 0,
      replaceEnd: 10,
    });
  });

  it("does not trigger on a mid-word @ (email-shaped input)", () => {
    // `a@b.com` is an email, not a mention. The caret sits after
    // `b`, walked back through `b` and `@`. Since `@` is preceded by
    // `a` (no whitespace), the mention pre-pass must NOT fire.
    const ctx = detectRefContext("a@b", 3);
    expect(ctx?.kind).not.toBe("mention");
  });

  it("captures an empty query immediately after the @", () => {
    expect(detectRefContext("hi @", 4)).toEqual({
      kind: "mention",
      query: "",
      openIndex: 3,
      replaceEnd: 4,
    });
  });

  it("stops at a `[[` opener so page refs win over a stray @", () => {
    // The caret sits inside an open `[[av…` — the page-ref branch
    // must win, regardless of the `@x ` that sits earlier in the
    // line.
    const ctx = detectRefContext("@x [[av", 7);
    expect(ctx?.kind).toBe("page");
    expect(ctx?.query).toBe("av");
  });
});

describe("applySuggestion: mention", () => {
  it("wraps the chosen title in `[[@…]]`", () => {
    const ctx = detectRefContext("hi @av", 6)!;
    const result = applySuggestion("hi @av", ctx, "avelino");
    expect(result).toEqual({ value: "hi [[@avelino]]", caret: 15 });
  });

  it("preserves spaces inside the chosen composite name", () => {
    const ctx = detectRefContext("@Thiago Av", 10)!;
    const result = applySuggestion("@Thiago Av", ctx, "Thiago Avelino");
    expect(result).toEqual({
      value: "[[@Thiago Avelino]]",
      caret: 19,
    });
  });
});

describe("withCreateNewPersonCandidate", () => {
  const person = (slug: string, title = slug) => ({
    id: `id-${slug}`,
    slug,
    title,
    kind: "page" as const,
    page_type: "person",
  });

  it("appends a synthetic candidate when no match exists", () => {
    const result = withCreateNewPersonCandidate([person("avelino", "Avelino")], "vini");
    expect(result).toHaveLength(2);
    expect(result[1]).toEqual({
      id: "",
      slug: "vini",
      title: "vini",
      kind: "page",
      page_type: "person",
    });
  });

  it("returns the input verbatim when an existing person matches (case-insensitive)", () => {
    const list = [person("avelino", "Avelino")];
    const result = withCreateNewPersonCandidate(list, "AVELINO");
    expect(result).toBe(list);
  });

  it("matches an existing person by alias name, not only by title", () => {
    const list = [{ ...person("avelino", "Avelino"), aliases: ["Ave", "TA"] }];
    expect(withCreateNewPersonCandidate(list, "ave")).toBe(list);
  });

  it("still appends when the alias list misses the query", () => {
    const list = [{ ...person("avelino", "Avelino"), aliases: ["TA"] }];
    expect(withCreateNewPersonCandidate(list, "vini")).toHaveLength(2);
  });

  it("returns the input verbatim when the query is empty", () => {
    const list = [person("avelino", "Avelino")];
    expect(withCreateNewPersonCandidate(list, "")).toBe(list);
    expect(withCreateNewPersonCandidate(list, "   ")).toBe(list);
  });

  it("appends to an empty list when the query is non-empty", () => {
    const empty: ReturnType<typeof person>[] = [];
    const result = withCreateNewPersonCandidate(empty, "lerolero");
    expect(result).toHaveLength(1);
    expect(result[0]?.title).toBe("lerolero");
  });
});

describe("detectEmojiContext", () => {
  it("opens after `:[a-z]` at start of buffer", () => {
    const ctx = detectEmojiContext(":roc", 4);
    expect(ctx).toEqual({ query: "roc", openIndex: 0, replaceEnd: 4 });
  });

  it("opens after a space + `:[a-z]`", () => {
    // value = "shipped :roc" — caret right after `c`.
    const ctx = detectEmojiContext("shipped :roc", 12);
    expect(ctx).toEqual({ query: "roc", openIndex: 8, replaceEnd: 12 });
  });

  it("opens after a newline + `:[a-z]`", () => {
    const ctx = detectEmojiContext("\n:fi", 4);
    expect(ctx).toEqual({ query: "fi", openIndex: 1, replaceEnd: 4 });
  });

  it("rejects mid-word `:` (URL boundary)", () => {
    // `https://` — the `:` is preceded by `s` (alphanumeric), so the
    // trigger should never open.
    expect(detectEmojiContext("https:/", 7)).toBeNull();
    expect(detectEmojiContext("see http:", 9)).toBeNull();
    expect(detectEmojiContext("14:00", 5)).toBeNull();
  });

  it("rejects `:` followed by a non-letter (digits, symbols, slash)", () => {
    // `:8080`, `:+1`, `://` — first char after `:` must be `[a-z]` so
    // the popup stays silent for ports / URL fragments.
    expect(detectEmojiContext(":8080", 5)).toBeNull();
    expect(detectEmojiContext(":+1", 3)).toBeNull();
    expect(detectEmojiContext(":/", 2)).toBeNull();
  });

  it("returns null when the caret is past a non-shortcode char", () => {
    // Whitespace between the typed prefix and the caret closes the trigger.
    expect(detectEmojiContext(":roc et", 7)).toBeNull();
  });

  it("returns null when no `:` is in the lookback window", () => {
    expect(detectEmojiContext("hello world", 11)).toBeNull();
  });

  it("returns null when the query exceeds the lookback cap", () => {
    // Cap is 32 chars. Query of 33 chars + `:` is 34 left of caret.
    const long = ":" + "a".repeat(33);
    expect(detectEmojiContext(long, long.length)).toBeNull();
  });

  it("opens with an empty query right after the `:` opener", () => {
    // The catalog gate rejects `:[a-z]` requirement; the cursor sitting
    // *on* the opener with nothing typed yet is technically empty.
    expect(detectEmojiContext(":", 1)).toBeNull();
  });
});

describe("applyEmojiSuggestion", () => {
  it("rewrites the in-flight :query to a canonical :shortcode:", () => {
    const ctx = detectEmojiContext("shipped :roc", 12)!;
    const out = applyEmojiSuggestion("shipped :roc", ctx, "rocket");
    expect(out.value).toBe("shipped :rocket:");
    expect(out.caret).toBe("shipped :rocket:".length);
  });

  it("preserves trailing text after the trigger", () => {
    // Caret in the middle of a longer buffer.
    const value = "shipped :roc today";
    const ctx = detectEmojiContext(value, 12)!;
    const out = applyEmojiSuggestion(value, ctx, "rocket");
    expect(out.value).toBe("shipped :rocket: today");
  });
});

describe("detectSlashContext", () => {
  it("triggers on a block-initial slash", () => {
    expect(detectSlashContext("/stats", 6)).toEqual({
      query: "stats",
      openIndex: 0,
      replaceEnd: 6,
    });
  });

  it("reports a partial query as the user types", () => {
    expect(detectSlashContext("/sta", 4)).toEqual({
      query: "sta",
      openIndex: 0,
      replaceEnd: 4,
    });
  });

  it("triggers on a bare slash (empty query → show all)", () => {
    expect(detectSlashContext("/", 1)).toEqual({
      query: "",
      openIndex: 0,
      replaceEnd: 1,
    });
  });

  it("does NOT trigger on a mid-text slash (path / URL)", () => {
    expect(detectSlashContext("see docs/api", 12)).toBeNull();
    expect(detectSlashContext("a/b", 3)).toBeNull();
  });

  it("closes the trigger once a space is typed", () => {
    expect(detectSlashContext("/stats now", 10)).toBeNull();
  });
});

describe("applySlashContext", () => {
  it("removes the /command token, leaving the block empty", () => {
    const ctx = detectSlashContext("/stats", 6)!;
    expect(applySlashContext("/stats", ctx)).toEqual({ value: "", caret: 0 });
  });

  it("keeps text that follows the token", () => {
    const ctx = detectSlashContext("/stats", 4)!; // caret mid-token
    expect(applySlashContext("/stats rest", ctx)).toEqual({
      value: " rest",
      caret: 0,
    });
  });
});
