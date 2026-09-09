//! Minimal reader for the field names an `export interface` declares in
//! `@outl/shared`'s `api/types.ts`.
//!
//! Deliberately not a TypeScript parser. It answers one question — which
//! names does this interface declare at its top level — and nothing
//! else, because that is the whole of the contract being pinned.
//!
//! Anything it cannot read is a hard failure rather than a silent empty
//! set. A parser that returned "no fields" for an interface it did not
//! understand would make the comparison pass for exactly the shapes most
//! likely to be wrong, which is the failure mode this whole file exists
//! to remove.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// Read the one `types.ts` every GUI client consumes.
pub fn source() -> String {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../outl-frontend-shared/src/api/types.ts");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Field names declared directly on `export interface <name>`.
///
/// A field whose type is an inline object literal contributes its own
/// name and none of its members, which is what a key-set comparison
/// against `serde_json` wants.
///
/// Panics when the interface is absent — a renamed interface is a
/// contract change, not a reason to skip the check.
pub fn interface_fields(src: &str, name: &str) -> BTreeSet<String> {
    let header = format!("export interface {name} {{");
    let start = src
        .find(&header)
        .unwrap_or_else(|| panic!("types.ts declares no `export interface {name}`"))
        + header.len();

    let mut fields = BTreeSet::new();
    let mut depth = 1usize;
    let mut in_block_comment = false;

    for raw in src[start..].lines() {
        let line = strip_comments(raw, &mut in_block_comment);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if depth == 1 {
            if let Some(field) = field_name(line) {
                fields.insert(field);
            }
        }

        let opens = line.matches('{').count();
        let closes = line.matches('}').count();
        if closes >= depth + opens {
            return fields;
        }
        depth = depth + opens - closes;
    }

    panic!("`export interface {name}` is never closed in types.ts");
}

/// Remove `//` and `/* … */` comments so a brace inside prose never
/// opens a phantom nesting level. `in_block` carries the open-comment
/// state across lines.
fn strip_comments(line: &str, in_block: &mut bool) -> String {
    let mut out = String::with_capacity(line.len());
    let bytes: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if *in_block {
            if bytes[i] == '*' && bytes.get(i + 1) == Some(&'/') {
                *in_block = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if bytes[i] == '/' && bytes.get(i + 1) == Some(&'*') {
            *in_block = true;
            i += 2;
            continue;
        }
        if bytes[i] == '/' && bytes.get(i + 1) == Some(&'/') {
            break;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// `  page_type?: string | null;` → `page_type`.
///
/// Returns `None` for anything that is not a field declaration — an
/// index signature, a method, a continuation of a multi-line union type.
fn field_name(line: &str) -> Option<String> {
    let colon = line.find(':')?;
    let ident = line[..colon].trim();
    let ident = ident.strip_suffix('?').unwrap_or(ident);
    if ident.is_empty() {
        return None;
    }
    let valid = ident
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    let starts_ok = ident
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$');
    (valid && starts_ok).then(|| ident.to_string())
}

/// Keys a serialized value puts on the wire.
///
/// Panics on a non-object, because every type pinned here is a struct
/// and a shape that stopped being one is precisely the change worth
/// failing on.
pub fn wire_keys(value: &serde_json::Value) -> BTreeSet<String> {
    match value {
        serde_json::Value::Object(map) => map.keys().cloned().collect(),
        other => panic!("expected a JSON object on the wire, got {other}"),
    }
}

/// The parser is the thing standing between a real contract check and
/// a vacuous one, so it gets its own tests. They run in this same
/// integration binary — an integration test crate is not compiled with
/// `--cfg test`, so a `#[cfg(test)]` gate here would silently skip them.
mod parser_tests {
    use super::{field_name, interface_fields, strip_comments};

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
    fn field_name_ignores_non_declarations() {
        assert_eq!(field_name("a: string;").as_deref(), Some("a"));
        assert_eq!(field_name("a?: string;").as_deref(), Some("a"));
        assert_eq!(field_name("| \"x\";"), None);
        assert_eq!(field_name("no colon here"), None);
    }
}
