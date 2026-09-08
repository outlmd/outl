//! Canonical language names for fenced code blocks.
//!
//! Markdown fence info-strings come in many flavours — `js`,
//! `javascript`, `node` are all "the same thing" to a user but used
//! to fail the `outl-exec` runtime lookup unless they spelled the
//! one the runtime registered with. This module is the single
//! source of truth for those aliases:
//!
//! - [`canonical`] folds every alias down to the form the runtime
//!   registry registered with.
//! - [`KNOWN_ALIASES`] is the underlying table; clients that need
//!   the full alias list (e.g. the desktop / mobile syntax
//!   highlighter that has to know `rs` is Rust) walk the same table
//!   instead of re-deriving it.
//!
//! Keep this in sync with the TS mirror at
//! `crates/outl-frontend-shared/src/highlight/aliases.ts`. The
//! `lang_alias_table_matches_ts_mirror` test in
//! `crates/outl-md/tests/lang.rs` is the canary: it parses the real
//! `.ts` file and compares row-for-row, in order. A CI failure there
//! means one of the two drifted — edit both in the same commit.
//!
//! That test carried a `(TODO)` here for long enough to be worth a
//! note: the comment named a guard that did not exist, and the TS side
//! pointed back at this file for the same guarantee. Neither end was
//! checking anything.

/// `(canonical, &[aliases including the canonical itself])` table.
///
/// **Ordering matters:** if a token could match more than one row
/// (it can't today, but if a future entry overlaps), the **first**
/// row wins. Canonical lookups go through [`canonical`].
pub const KNOWN_ALIASES: &[(&str, &[&str])] = &[
    // Runtime-backed (outl-exec dispatches on these).
    ("js", &["js", "javascript", "node", "nodejs"]),
    ("python", &["python", "py", "python3"]),
    ("rust", &["rust", "rs"]),
    ("lua", &["lua"]),
    ("lisp", &["lisp", "cl", "common-lisp", "elisp"]),
    ("echo", &["echo", "text", "txt", "plain"]),
    ("query", &["query", "tasks"]),
    // Highlight-only (no runtime, but the syntax highlighter on
    // every client should know these so a fence labelled `ts`,
    // `tsx`, `bash`, `yml`, etc. still gets colorized). Listed here
    // so the canonical form is shared with the TS side.
    ("typescript", &["typescript", "ts"]),
    ("tsx", &["tsx"]),
    ("jsx", &["jsx"]),
    ("shell", &["shell", "sh", "bash", "zsh"]),
    ("yaml", &["yaml", "yml"]),
    ("toml", &["toml"]),
    ("json", &["json"]),
    ("markdown", &["markdown", "md"]),
    ("html", &["html", "htm"]),
    ("css", &["css"]),
    ("scss", &["scss", "sass"]),
    ("sql", &["sql", "postgres", "postgresql", "mysql", "sqlite"]),
    ("go", &["go", "golang"]),
    ("c", &["c"]),
    ("cpp", &["cpp", "c++", "cxx", "cc"]),
    ("csharp", &["csharp", "cs", "c#"]),
    ("java", &["java"]),
    ("kotlin", &["kotlin", "kt"]),
    ("swift", &["swift"]),
    ("ruby", &["ruby", "rb"]),
    ("php", &["php"]),
    ("dockerfile", &["dockerfile", "docker"]),
    ("nix", &["nix"]),
    ("clojure", &["clojure", "clj"]),
    ("haskell", &["haskell", "hs"]),
    ("scala", &["scala"]),
    ("dart", &["dart"]),
    ("zig", &["zig"]),
];

/// Resolve any known alias to its canonical form.
///
/// Lower-cases the input before matching so `Rust` / `RUST` / `rust`
/// all collapse to `"rust"`. Returns `None` when the input is empty
/// or doesn't match any known alias — callers either pass the
/// original string through (frontend highlighter falls back to
/// "plain text"; `outl-exec` returns "no runtime registered").
pub fn canonical(raw: &str) -> Option<&'static str> {
    let needle = raw.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return None;
    }
    for (canon, aliases) in KNOWN_ALIASES {
        for alias in *aliases {
            if needle == *alias {
                return Some(*canon);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn javascript_resolves_to_js() {
        assert_eq!(canonical("javascript"), Some("js"));
        assert_eq!(canonical("node"), Some("js"));
        assert_eq!(canonical("nodejs"), Some("js"));
        assert_eq!(canonical("JS"), Some("js"));
    }

    #[test]
    fn rust_aliases_resolve() {
        assert_eq!(canonical("rs"), Some("rust"));
        assert_eq!(canonical("rust"), Some("rust"));
        assert_eq!(canonical("RUST"), Some("rust"));
    }

    #[test]
    fn python_aliases_resolve() {
        assert_eq!(canonical("py"), Some("python"));
        assert_eq!(canonical("python3"), Some("python"));
        assert_eq!(canonical("python"), Some("python"));
    }

    #[test]
    fn lisp_family_resolves() {
        assert_eq!(canonical("lisp"), Some("lisp"));
        assert_eq!(canonical("cl"), Some("lisp"));
        assert_eq!(canonical("elisp"), Some("lisp"));
    }

    #[test]
    fn shell_aliases_resolve() {
        assert_eq!(canonical("sh"), Some("shell"));
        assert_eq!(canonical("bash"), Some("shell"));
        assert_eq!(canonical("zsh"), Some("shell"));
    }

    #[test]
    fn unknown_lang_returns_none() {
        assert_eq!(canonical("brainfuck"), None);
        assert_eq!(canonical(""), None);
        assert_eq!(canonical("   "), None);
    }

    #[test]
    fn trims_whitespace() {
        assert_eq!(canonical("  rust  "), Some("rust"));
    }
}
