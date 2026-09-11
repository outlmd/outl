//! The reader itself: what one TypeScript source declares.
//!
//! Split from `mod.rs` on size, and along the seam that was already
//! there — `mod.rs` answers *which files are mirrors*, this answers
//! *what does one of them say*. The tests stay in `mod.rs` so their
//! paths (`ts_parser::parser_tests::…`) do not move.
//!
//! Deliberately not a TypeScript parser. It answers two questions —
//! which names does this interface declare at its top level, and which
//! members does this union have — and nothing else, because that is the
//! whole of the contract being pinned.
//!
//! Anything it cannot read is a hard failure rather than a silent empty
//! set. A reader that returned "no fields" for an interface it did not
//! understand would make the comparison pass for exactly the shapes most
//! likely to be wrong, which is the failure mode this whole directory
//! exists to remove.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

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
pub(super) fn strip_comments(line: &str, in_block: &mut bool) -> String {
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
pub(super) fn field_name(line: &str) -> Option<String> {
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

// --- unions ---------------------------------------------------------------

/// Strip every `//` and `/* … */` comment from a whole source file,
/// keeping the line structure.
///
/// The union readers below split on `|` and `;`, and both characters
/// appear inside this codebase's doc comments often enough that reading
/// a union without stripping first is a coin flip.
pub fn clean(src: &str) -> String {
    let mut in_block = false;
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        out.push_str(&strip_comments(line, &mut in_block));
        out.push('\n');
    }
    out
}

/// The right-hand side of `export type <name> = …;`, comments stripped.
///
/// Panics when the alias is absent, for the same reason
/// [`interface_fields`] does: a renamed union is a contract change, not
/// a reason to skip the check.
pub fn type_alias_body(src: &str, name: &str) -> String {
    let cleaned = clean(src);
    let header = format!("export type {name} ");
    let start = cleaned
        .find(&header)
        .unwrap_or_else(|| panic!("types mirrors declare no `export type {name}`"))
        + header.len();
    let rest = &cleaned[start..];
    let eq = rest
        .find('=')
        .unwrap_or_else(|| panic!("`export type {name}` has no `=`"));
    let body = &rest[eq + 1..];

    let mut depth = 0usize;
    let mut in_string = false;
    for (idx, ch) in body.char_indices() {
        if in_string {
            if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => return body[..idx].to_string(),
            _ => {}
        }
    }
    panic!("`export type {name}` is never terminated by `;`")
}

/// Split a union body into its members on the top-level `|`.
///
/// A leading `|` (the multi-line style this repo uses) yields an empty
/// first member, which is dropped.
fn union_members(body: &str) -> Vec<String> {
    let mut members = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut in_string = false;
    for ch in body.chars() {
        if in_string {
            current.push(ch);
            if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => {
                in_string = true;
                current.push(ch);
            }
            '{' | '[' | '(' => {
                depth += 1;
                current.push(ch);
            }
            '}' | ']' | ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            '|' if depth == 0 => members.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    members.push(current);
    members
        .into_iter()
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .collect()
}

/// The string literals of `export type <name> = "a" | "b";`.
///
/// Panics when a member is not a bare string literal — a union that
/// grew an object member is no longer the shape being compared, and
/// quietly skipping it would shrink the declared set until the
/// comparison passed.
pub fn string_union_members(src: &str, name: &str) -> BTreeSet<String> {
    let body = type_alias_body(src, name);
    union_members(&body)
        .iter()
        .map(|m| {
            string_literal(m).unwrap_or_else(|| {
                panic!("`export type {name}`: member `{m}` is not a string literal")
            })
        })
        .collect()
}

/// `"plain"` → `plain`. `None` for anything else.
fn string_literal(member: &str) -> Option<String> {
    let inner = member.strip_prefix('"')?.strip_suffix('"')?;
    (!inner.contains('"')).then(|| inner.to_string())
}

/// Members of a discriminated union, keyed by their `tag_field` value.
///
/// `export type Key = { kind: "Char"; value: string } | { kind: "Esc" }`
/// with `tag_field = "kind"` yields `{"Char": {"kind","value"},
/// "Esc": {"kind"}}` — the tag counts as a field because `serde` puts
/// it on the wire as one.
pub fn tagged_union_members(
    src: &str,
    name: &str,
    tag_field: &str,
) -> BTreeMap<String, BTreeSet<String>> {
    let body = type_alias_body(src, name);
    let mut out = BTreeMap::new();
    for member in union_members(&body) {
        let inner = member
            .strip_prefix('{')
            .and_then(|m| m.strip_suffix('}'))
            .unwrap_or_else(|| {
                panic!("`export type {name}`: member `{member}` is not an object literal")
            });
        let (fields, tag) = object_literal_fields(inner, tag_field);
        let tag = tag.unwrap_or_else(|| {
            panic!("`export type {name}`: member `{member}` declares no `{tag_field}` literal")
        });
        if out.insert(tag.clone(), fields).is_some() {
            panic!("`export type {name}`: two members share the tag `{tag}`");
        }
    }
    out
}

/// Field names of an inline object literal's body, plus the string
/// literal `tag_field` is assigned (when it is one).
fn object_literal_fields(inner: &str, tag_field: &str) -> (BTreeSet<String>, Option<String>) {
    let mut fields = BTreeSet::new();
    let mut tag = None;
    for part in split_object_members(inner) {
        let Some(name) = field_name(&part) else {
            continue;
        };
        if name == tag_field {
            let value = part[part.find(':').expect("field_name found a colon") + 1..].trim();
            tag = string_literal(value);
        }
        fields.insert(name);
    }
    (fields, tag)
}

/// Split an object literal's body on the top-level `;` / `,` / newline.
fn split_object_members(inner: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut in_string = false;
    for ch in inner.chars() {
        if in_string {
            current.push(ch);
            if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => {
                in_string = true;
                current.push(ch);
            }
            '{' | '[' | '(' => {
                depth += 1;
                current.push(ch);
            }
            '}' | ']' | ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ';' | ',' | '\n' if depth == 0 => parts.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    parts.push(current);
    parts
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// The string literals a single interface field's type union declares.
///
/// `change: "created" | "edited";` inside `export interface
/// TimelineEvent` → `{"created", "edited"}`. This is the shape a
/// hand-serialized tag takes: the Rust side emits a `String` and the
/// contract lives entirely in the TypeScript union, so nothing but this
/// can compare the two.
pub fn interface_field_union(src: &str, iface: &str, field: &str) -> BTreeSet<String> {
    let cleaned = clean(src);
    let header = format!("export interface {iface} {{");
    let start = cleaned
        .find(&header)
        .unwrap_or_else(|| panic!("types mirrors declare no `export interface {iface}`"))
        + header.len();

    let mut depth = 1usize;
    let mut in_string = false;
    let mut current = String::new();
    let mut members = Vec::new();
    for ch in cleaned[start..].chars() {
        if in_string {
            current.push(ch);
            if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => {
                in_string = true;
                current.push(ch);
            }
            '{' | '[' | '(' => {
                depth += 1;
                current.push(ch);
            }
            '}' | ']' | ')' if depth > 1 => {
                depth -= 1;
                current.push(ch);
            }
            '}' => {
                members.push(std::mem::take(&mut current));
                break;
            }
            ';' if depth == 1 => members.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }

    for member in members {
        let member = member.trim();
        if field_name(member).as_deref() != Some(field) {
            continue;
        }
        let ty = &member[member.find(':').expect("field_name found a colon") + 1..];
        return union_members(ty)
            .iter()
            .map(|m| {
                string_literal(m).unwrap_or_else(|| {
                    panic!("`{iface}.{field}`: member `{m}` is not a string literal")
                })
            })
            .collect();
    }
    panic!("`export interface {iface}` declares no field `{field}`")
}
