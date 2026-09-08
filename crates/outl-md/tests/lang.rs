//! Drift gate between `outl_md::lang::KNOWN_ALIASES` and its TypeScript
//! mirror at `crates/outl-frontend-shared/src/highlight/aliases.ts`.
//!
//! The two tables are the same fact written twice, for two runtimes that
//! cannot import each other: the Rust side dispatches `outl-exec`
//! runtimes on a fence's canonical language, and the TS side picks the
//! highlight.js grammar for the identical fence. A row present on one
//! side and missing on the other means a fence that executes but renders
//! unhighlighted, or highlights but finds no runtime — neither of which
//! fails anything today.
//!
//! Both files already *claimed* to be guarded. `lang.rs` named this test
//! and marked it `(TODO)`; `aliases.ts` pointed at "`lang.rs::tests::*`
//! and the catalog-sync hook". Neither existed for this pair — the hook
//! covers `docs/primitives-*.md` against the Copilot instructions, not
//! this one. Two files each naming the other as the safety net, with
//! nothing in between. This is the net.
//!
//! Parsing the `.ts` rather than generating it is deliberate: a
//! generator makes the Rust side authoritative and the TS side a build
//! artifact, which is a bigger change than the drift warrants, and it
//! would not survive someone editing the generated file by hand. Reading
//! the real file means the assertion fails on exactly the edit that
//! introduced the divergence.

use outl_md::lang::KNOWN_ALIASES;

/// The TS mirror's source, embedded at compile time.
///
/// `include_str!` over a runtime read on purpose: if the file is moved
/// or deleted, this test stops compiling instead of silently passing on
/// an empty parse.
const TS_MIRROR: &str = include_str!("../../outl-frontend-shared/src/highlight/aliases.ts");

/// Parse `["canon", ["a", "b", ...]],` rows out of the TS table.
///
/// Deliberately not a general JS parser: the table's shape is pinned by
/// `the_ts_mirror_is_shaped_the_way_this_parser_expects` below, so a
/// reformat that this cannot read fails loudly rather than returning a
/// short list that happens to match a truncated Rust table.
fn parse_ts_table(src: &str) -> Vec<(String, Vec<String>)> {
    // Anchor on the DECLARATION, not on the identifier.
    //
    // `KNOWN_ALIASES` appears first in the module doc comment, ~19 lines
    // above the real table, and anchoring there put `start` inside the
    // prose: the `[` it found belonged to "The shape is `[canonical,
    // [...aliases]]`". That parsed correctly only because the prose in
    // between happens to contain no `"` — so editing a COMMENT could make
    // this test fail with "the tables have diverged", which is the wrong
    // diagnosis for a comment edit. `the_parser_ignores_the_module_doc_comment`
    // below pins the fix.
    let decl = src
        .find("export const KNOWN_ALIASES")
        .expect("aliases.ts must export a KNOWN_ALIASES const");
    let start = decl
        + src[decl..]
            .find("= [")
            .expect("the KNOWN_ALIASES const must be assigned an array literal")
        + "= [".len();
    let end = start
        + src[start..]
            .find("\n];")
            .expect("the KNOWN_ALIASES array must be terminated by a line starting `];`");

    let mut rows = Vec::new();
    for raw in src[start..end].split("],") {
        // Every string literal on the row, in source order. The table
        // uses no escapes and no single quotes, so this is exact.
        let strings: Vec<String> = raw
            .split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect();
        let Some((canonical, aliases)) = strings.split_first() else {
            continue; // trailing whitespace between the last row and `];`
        };
        rows.push((canonical.clone(), aliases.to_vec()));
    }
    rows
}

#[test]
fn the_ts_mirror_is_shaped_the_way_this_parser_expects() {
    let rows = parse_ts_table(TS_MIRROR);

    assert!(
        rows.len() > 20,
        "parsed only {} rows out of aliases.ts — the file was probably \
         reformatted into a shape `parse_ts_table` cannot read. Fix the \
         parser; do NOT relax this assertion, because a parser that \
         silently returns a short list turns every check below into a \
         no-op.",
        rows.len()
    );

    for (canonical, aliases) in &rows {
        assert!(
            aliases.contains(canonical),
            "row `{canonical}` in aliases.ts must list its own canonical \
             name among its aliases (the Rust table's convention)"
        );
    }
}

#[test]
fn lang_alias_table_matches_ts_mirror() {
    let ts = parse_ts_table(TS_MIRROR);
    let rust: Vec<(String, Vec<String>)> = KNOWN_ALIASES
        .iter()
        .map(|(canonical, aliases)| {
            (
                (*canonical).to_owned(),
                aliases.iter().map(|a| (*a).to_owned()).collect(),
            )
        })
        .collect();

    // Compared as ordered lists, not sets. The Rust doc comment says
    // "Ordering matters: if a token could match more than one row, the
    // first row wins" and both `canonical` implementations scan top to
    // bottom. Two tables holding the same rows in a different order
    // would resolve an overlapping token differently per client.
    assert_eq!(
        rust, ts,
        "\n`outl_md::lang::KNOWN_ALIASES` and its TS mirror at \
         crates/outl-frontend-shared/src/highlight/aliases.ts have \
         diverged.\n\nA row on only one side means a fenced code block \
         that runs but renders unhighlighted (or the reverse). Edit both \
         tables in the same commit — same rows, same order.\n"
    );
}

#[test]
fn every_runtime_backed_language_is_mirrored() {
    let ts = parse_ts_table(TS_MIRROR);

    // The rows `outl-exec` dispatches on. A highlight-only row missing
    // from the TS side costs colour; one of these missing costs the
    // client its ability to name the runtime a fence would execute in.
    for canonical in ["js", "python", "rust", "lua", "lisp", "echo", "query"] {
        assert!(
            ts.iter().any(|(c, _)| c == canonical),
            "runtime-backed language `{canonical}` is registered in \
             outl-exec and present in the Rust alias table, but absent \
             from the TS mirror"
        );
    }
}

#[test]
fn the_parser_ignores_the_module_doc_comment() {
    // Regression for the anchor bug: `parse_ts_table` used to `find` the
    // bare identifier, which matches the doc comment ~19 lines above the
    // table. Injecting a quoted example into that comment then prepended
    // stray strings to row 0, and the failure read "the tables have
    // diverged" — a comment edit diagnosed as a data divergence.
    //
    // This mutates the doc comment only. The table below it is untouched,
    // so a parser anchored on the declaration must return the identical
    // rows.
    let baseline = parse_ts_table(TS_MIRROR);

    let noisy = TS_MIRROR.replace(
        " * Resolve any known alias to its canonical form.",
        " * Example: `[\"javascript\"]` folds to \"js\", first match wins.\n \
         * Resolve any known alias to its canonical form.",
    );
    assert_ne!(noisy, TS_MIRROR, "the comment anchor text moved; update it");

    let header = format!(
        "/**\n * Mentions KNOWN_ALIASES and shows `[canonical, [\"a\", \"b\"]]`.\n */\n{noisy}"
    );

    assert_eq!(
        parse_ts_table(&header),
        baseline,
        "a doc comment mentioning KNOWN_ALIASES (with brackets and quotes) \
         changed what the parser read out of the table. The anchor slipped \
         back to the identifier instead of the declaration."
    );
}
