import { describe, expect, it } from "vitest";

import { canonical, KNOWN_ALIASES } from "./aliases";

describe("highlight/aliases", () => {
  it("resolves the common Rust aliases", () => {
    expect(canonical("rs")).toBe("rust");
    expect(canonical("rust")).toBe("rust");
    expect(canonical("RUST")).toBe("rust");
  });

  it("resolves the JavaScript family to `js`", () => {
    expect(canonical("javascript")).toBe("js");
    expect(canonical("node")).toBe("js");
    expect(canonical("nodejs")).toBe("js");
    expect(canonical("JS")).toBe("js");
  });

  it("resolves Python aliases", () => {
    expect(canonical("py")).toBe("python");
    expect(canonical("python3")).toBe("python");
  });

  it("returns null for empty / unknown", () => {
    expect(canonical("")).toBeNull();
    expect(canonical(null)).toBeNull();
    expect(canonical(undefined)).toBeNull();
    expect(canonical("brainfuck")).toBeNull();
  });

  it("trims whitespace", () => {
    expect(canonical("  rust  ")).toBe("rust");
  });

  it("mirrors the Rust canonical set", () => {
    // Failsafe: if the Rust side drops a canonical, this test still
    // passes (the row is just absent) — the hard guard is
    // `lang_alias_table_matches_ts_mirror` in
    // `crates/outl-md/tests/lang.rs`, which parses this file and
    // compares it row-for-row against `outl_md::lang::KNOWN_ALIASES`.
    // What this catches locally is a typo / duplicate canonical
    // introduced when editing this file in isolation.
    const canonicals = KNOWN_ALIASES.map(([c]) => c);
    expect(new Set(canonicals).size).toBe(canonicals.length);
  });

  it("each alias group includes its canonical form", () => {
    for (const [canon, aliases] of KNOWN_ALIASES) {
      expect(aliases).toContain(canon);
    }
  });
});
