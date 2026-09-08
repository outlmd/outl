/**
 * Canonical language names for fenced code blocks.
 *
 * **Mirror of `outl_md::lang::KNOWN_ALIASES`** (`crates/outl-md/src/lang.rs`).
 * Both clients (mobile + desktop) consume this when deciding which
 * highlight.js grammar to apply, and the Rust side uses the same
 * table when dispatching `outl-exec` runtimes. If you add/rename a
 * row in one, do the matching edit on the other side in the same
 * commit — `lang_alias_table_matches_ts_mirror` in
 * `crates/outl-md/tests/lang.rs` is the canary. It parses THIS file
 * and compares row-for-row, in order, so a divergence fails CI on the
 * commit that introduced it.
 *
 * Ordering is part of the contract, not incidental: both `canonical`
 * implementations scan top to bottom and return the first match, so
 * the same rows in a different order would resolve an overlapping
 * token differently per client.
 *
 * The shape is `[canonical, [...aliases including the canonical]]`;
 * ordering matters (first match wins) but no rows overlap today.
 */
export const KNOWN_ALIASES: ReadonlyArray<readonly [string, ReadonlyArray<string>]> = [
  ["js", ["js", "javascript", "node", "nodejs"]],
  ["python", ["python", "py", "python3"]],
  ["rust", ["rust", "rs"]],
  ["lua", ["lua"]],
  ["lisp", ["lisp", "cl", "common-lisp", "elisp"]],
  ["echo", ["echo", "text", "txt", "plain"]],
  ["query", ["query", "tasks"]],
  ["typescript", ["typescript", "ts"]],
  ["tsx", ["tsx"]],
  ["jsx", ["jsx"]],
  ["shell", ["shell", "sh", "bash", "zsh"]],
  ["yaml", ["yaml", "yml"]],
  ["toml", ["toml"]],
  ["json", ["json"]],
  ["markdown", ["markdown", "md"]],
  ["html", ["html", "htm"]],
  ["css", ["css"]],
  ["scss", ["scss", "sass"]],
  ["sql", ["sql", "postgres", "postgresql", "mysql", "sqlite"]],
  ["go", ["go", "golang"]],
  ["c", ["c"]],
  ["cpp", ["cpp", "c++", "cxx", "cc"]],
  ["csharp", ["csharp", "cs", "c#"]],
  ["java", ["java"]],
  ["kotlin", ["kotlin", "kt"]],
  ["swift", ["swift"]],
  ["ruby", ["ruby", "rb"]],
  ["php", ["php"]],
  ["dockerfile", ["dockerfile", "docker"]],
  ["nix", ["nix"]],
  ["clojure", ["clojure", "clj"]],
  ["haskell", ["haskell", "hs"]],
  ["scala", ["scala"]],
  ["dart", ["dart"]],
  ["zig", ["zig"]],
];

/**
 * Resolve any known alias to its canonical form. Returns `null`
 * when the input is empty or doesn't match any alias — callers
 * fall back to "plain text" / no highlighting.
 */
export function canonical(raw: string | null | undefined): string | null {
  if (!raw) return null;
  const needle = raw.trim().toLowerCase();
  if (!needle) return null;
  for (const [canon, aliases] of KNOWN_ALIASES) {
    if (aliases.includes(needle)) return canon;
  }
  return null;
}
