//! Minimal reader for what the TypeScript side of the Tauri wire
//! contract declares: the fields of an `export interface`, and the
//! members of an `export type` union.
//!
//! Deliberately not a TypeScript parser. It answers two questions —
//! which names does this interface declare at its top level, and which
//! members does this union have — and nothing else, because that is the
//! whole of the contract being pinned.
//!
//! Anything it cannot read is a hard failure rather than a silent empty
//! set. A parser that returned "no fields" for an interface it did not
//! understand would make the comparison pass for exactly the shapes most
//! likely to be wrong, which is the failure mode this whole file exists
//! to remove.
//!
//! ## Why a union reader had to exist
//!
//! [`wire_keys`] panics on anything that is not a JSON object, and a
//! `serde` enum serializes to a *string* (or an externally/internally
//! tagged object). So every enum on the wire was unpinnable **by
//! construction**, and twelve of them were: `ParseWarningKind` shipped
//! one of its six variants to TypeScript and nothing could fail.
//!
//! ## Compiled into several test binaries
//!
//! Every pin binary (`wire_types`, `wire_enums`, `wire_mirrors`,
//! `wire_plugins`, `wire_coverage`) includes this module, so each gets
//! its own copy and each leaves part of it unused. That is what the
//! crate-level `dead_code` allow is for — the alternative is a reader
//! per binary, and copies of a reader drift exactly the way copies of a
//! DTO do.
#![allow(dead_code)]

use std::path::PathBuf;

mod reader;

// Re-exported for the pin binaries. `wire_coverage` needs only the
// discovery half, so the allow keeps it from reporting the rest dead.
#[allow(unused_imports)]
pub use reader::{
    interface_field_union, interface_fields, string_union_members, tagged_union_members, wire_keys,
};

/// Every TypeScript file that hand-mirrors a Rust wire type, relative
/// to this crate's manifest directory.
///
/// The gate used to walk only `types.ts`, so a mirror living anywhere
/// else was not merely unpinned — it was **invisible**, and the
/// coverage report counted a universe that excluded it. `BlockHit`,
/// `EmojiHit`, `ImportedAsset`, the whole shortcut catalog
/// (`Chord` / `Key` / `Action` / `Binding` / `SupportDto`) and both
/// event payloads were in that blind spot.
///
/// A client-local mirror of a shape already declared in `@outl/shared`
/// does not belong here: `outl-mobile`'s private `DeepLinkNavigate` in
/// `Journal.tsx` is a copy of the desktop's, and pinning a copy twice
/// pins nothing extra.
pub const MIRROR_FILES: &[&str] = &[
    "../outl-frontend-shared/src/api/types.ts",
    "../outl-frontend-shared/src/api/plugins.ts",
    "../outl-frontend-shared/src/api/commands.ts",
    "../outl-desktop/src/lib/api.ts",
    "../outl-desktop/src/lib/events.ts",
];

/// Read one mirror file by its [`MIRROR_FILES`] path.
pub fn mirror_source(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Every mirror file, concatenated.
///
/// Concatenating is safe only while no two files declare the same name;
/// `no_two_mirror_files_declare_the_same_name` in `wire_types.rs` is
/// what keeps that true, because a duplicate would silently pin one
/// declaration and leave its twin unwatched.
pub fn source() -> String {
    let mut out = String::new();
    for rel in MIRROR_FILES {
        out.push_str(&mirror_source(rel));
        out.push('\n');
    }
    out
}

/// The parser is the thing standing between a real contract check and
/// a vacuous one, so it gets its own tests. They run in this same
/// integration binary — an integration test crate is not compiled with
/// `--cfg test`, so a `#[cfg(test)]` gate here would silently skip them.
mod parser_tests {
    use super::reader::{field_name, interface_fields, strip_comments};

    #[test]
    fn reads_a_plain_interface() {
        let src = "export interface Foo {\n  a: string;\n  b?: number;\n}\n";
        let fields = interface_fields(src, "Foo");
        assert_eq!(fields.into_iter().collect::<Vec<_>>(), ["a", "b"]);
    }

    #[test]
    fn a_brace_inside_prose_does_not_open_a_level() {
        // The real file is heavy on doc comments; one of them mentioning
        // `{` used to be enough to swallow every field after it.
        let src = "export interface Foo {\n  /** shaped like { x: 1 } */\n  a: string;\n}\n";
        assert_eq!(
            interface_fields(src, "Foo").into_iter().collect::<Vec<_>>(),
            ["a"]
        );
    }

    #[test]
    fn a_nested_object_literal_contributes_only_its_own_name() {
        let src = "export interface Foo {\n  a: {\n    hidden: string;\n  };\n  b: number;\n}\n";
        assert_eq!(
            interface_fields(src, "Foo").into_iter().collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[test]
    fn a_multiline_union_contributes_nothing_extra() {
        let src = "export interface Foo {\n  a:\n    | \"x\"\n    | \"y\";\n  b: number;\n}\n";
        assert_eq!(
            interface_fields(src, "Foo").into_iter().collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[test]
    fn generic_types_with_brackets_are_not_braces() {
        let src = "export interface Foo {\n  a: Array<[string, string]>;\n}\n";
        assert_eq!(
            interface_fields(src, "Foo").into_iter().collect::<Vec<_>>(),
            ["a"]
        );
    }

    #[test]
    #[should_panic(expected = "declares no `export interface Missing`")]
    fn a_renamed_interface_fails_loudly() {
        interface_fields("export interface Foo {\n}\n", "Missing");
    }

    #[test]
    fn block_comments_survive_across_lines() {
        let mut open = false;
        assert_eq!(strip_comments("a /* start", &mut open).trim(), "a");
        assert!(open);
        assert_eq!(strip_comments("still comment", &mut open).trim(), "");
        assert_eq!(strip_comments("end */ b: 1;", &mut open).trim(), "b: 1;");
        assert!(!open);
    }

    #[test]
    fn reads_a_single_line_string_union() {
        let src = "export type Foo = \"a\" | \"b\";\n";
        assert_eq!(
            super::reader::string_union_members(src, "Foo")
                .into_iter()
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[test]
    fn reads_a_multi_line_string_union_with_a_leading_pipe() {
        // The style every union in `types.ts` is written in, and the
        // one a naive split on `|` gets wrong (it yields an empty
        // first member).
        let src = "export type Foo =\n  | \"a\"\n  /** doc */\n  | \"b\";\n";
        assert_eq!(
            super::reader::string_union_members(src, "Foo")
                .into_iter()
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[test]
    fn a_pipe_inside_a_comment_does_not_split_a_union() {
        let src = "export type Foo =\n  | \"a\"\n  // either | or\n  | \"b\";\n";
        assert_eq!(super::reader::string_union_members(src, "Foo").len(), 2);
    }

    #[test]
    fn a_semicolon_inside_an_object_member_does_not_end_the_alias() {
        let src = "export type Foo =\n  | { kind: \"a\"; v: string }\n  | { kind: \"b\" };\n";
        let members = super::reader::tagged_union_members(src, "Foo", "kind");
        assert_eq!(members.keys().collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(
            members["a"].iter().collect::<Vec<_>>(),
            ["kind", "v"],
            "the tag counts as a field — serde puts it on the wire as one"
        );
        assert_eq!(members["b"].iter().collect::<Vec<_>>(), ["kind"]);
    }

    #[test]
    #[should_panic(expected = "is not a string literal")]
    fn a_union_that_stopped_being_strings_fails_loudly() {
        // Silently skipping an unreadable member would shrink the
        // declared set until the comparison passed — the one outcome
        // this reader must never produce.
        super::reader::string_union_members("export type Foo = \"a\" | number;\n", "Foo");
    }

    #[test]
    #[should_panic(expected = "declare no `export type Missing`")]
    fn a_renamed_union_fails_loudly() {
        super::reader::string_union_members("export type Foo = \"a\";\n", "Missing");
    }

    #[test]
    fn reads_a_field_level_union_inside_an_interface() {
        let src = "export interface Foo {\n  a: string;\n  change: \"x\" | \"y\";\n}\n";
        assert_eq!(
            super::reader::interface_field_union(src, "Foo", "change")
                .into_iter()
                .collect::<Vec<_>>(),
            ["x", "y"]
        );
    }

    #[test]
    #[should_panic(expected = "declares no field `missing`")]
    fn an_absent_field_union_fails_loudly() {
        super::reader::interface_field_union(
            "export interface Foo {\n  a: string;\n}\n",
            "Foo",
            "missing",
        );
    }

    #[test]
    fn field_name_ignores_non_declarations() {
        assert_eq!(field_name("a: string;").as_deref(), Some("a"));
        assert_eq!(field_name("a?: string;").as_deref(), Some("a"));
        assert_eq!(field_name("| \"x\";"), None);
        assert_eq!(field_name("no colon here"), None);
    }
}
