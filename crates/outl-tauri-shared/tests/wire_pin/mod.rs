//! The comparisons every wire pin runs, in one place.
//!
//! `tests/ts_parser` reads what TypeScript declares; this module turns
//! that into a verdict against what `serde` actually emits. It is a
//! module rather than a copy per test binary because three binaries pin
//! the same contract (`wire_types`, `wire_enums`, `wire_mirrors`) and
//! three copies of a comparison drift the same way three copies of a
//! DTO do — which is the bug this whole directory exists to catch.
//!
//! Each binary includes it, and each leaves part of it unused; that is
//! what the `dead_code` allow below is for.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::ts_parser::{
    interface_field_union, interface_fields, source, string_union_members, tagged_union_members,
    wire_keys,
};

/// Compare what `value` serializes to against what `export interface
/// <ts_name>` declares.
///
/// `rust_only` names fields the backend emits that the frontend
/// deliberately does not model. Each entry is a decision someone made
/// and had to write down here — which is the difference between a
/// declared asymmetry and a drift nobody noticed.
pub fn assert_wire_shape<T: Serialize>(value: &T, ts_name: &str, rust_only: &[&str]) {
    let src = source();
    let declared = interface_fields(&src, ts_name);
    let emitted = wire_keys(&serde_json::to_value(value).expect("DTO serializes"));

    let exempt: BTreeSet<String> = rust_only.iter().map(|s| (*s).to_string()).collect();
    for name in &exempt {
        assert!(
            emitted.contains(name),
            "{ts_name}: `{name}` is listed as backend-only but the backend does not emit it — \
             drop the exemption"
        );
    }

    let missing_in_ts: Vec<&String> = emitted
        .difference(&declared)
        .filter(|f| !exempt.contains(*f))
        .collect();
    let missing_in_rust: Vec<&String> = declared.difference(&emitted).collect();

    assert!(
        missing_in_ts.is_empty(),
        "{ts_name}: the backend emits {missing_in_ts:?}, which the TypeScript mirror does not \
         declare.\n\
         The frontend reads those as `undefined`. Add them to `export interface {ts_name}`, \
         or list them in `rust_only` with a reason if the frontend deliberately ignores them."
    );
    assert!(
        missing_in_rust.is_empty(),
        "{ts_name}: the TypeScript mirror declares {missing_in_rust:?}, which the backend never \
         emits.\n\
         The frontend is modelling a field that is always `undefined` — either the Rust DTO \
         lost it, or the interface should."
    );
}

/// Compare the strings an enum serializes to against the members of
/// `export type <ts_name> = "a" | "b";`.
pub fn assert_string_enum<T: Serialize>(variants: &[T], ts_name: &str) {
    let src = source();
    let declared = string_union_members(&src, ts_name);
    let emitted: BTreeSet<String> = variants
        .iter()
        .map(
            |v| match serde_json::to_value(v).expect("variant serializes") {
                serde_json::Value::String(s) => s,
                other => panic!(
                    "{ts_name}: a variant serialized to {other}, not a string — \
                     it is a tagged enum, so pin it with `assert_tagged_enum`"
                ),
            },
        )
        .collect();
    compare(&emitted, &declared, ts_name, "variant");
}

/// Compare a tagged enum against `export type <ts_name> = { <tag>: "a"; … } | …`.
///
/// Both the tag set **and** each variant's field set are compared. A
/// field added to one variant of a tagged union is exactly as invisible
/// to a client as a missing variant, and it is the likelier of the two.
pub fn assert_tagged_enum<T: Serialize>(variants: &[T], ts_name: &str, tag: &str) {
    let src = source();
    let declared = tagged_union_members(&src, ts_name, tag);

    let mut emitted: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for variant in variants {
        let value = serde_json::to_value(variant).expect("variant serializes");
        let serde_json::Value::Object(map) = value else {
            panic!(
                "{ts_name}: a variant did not serialize to an object — \
                 a bare string enum belongs in `assert_string_enum`"
            );
        };
        let tag_value = match map.get(tag) {
            Some(serde_json::Value::String(s)) => s.clone(),
            _ => panic!("{ts_name}: a variant carries no string `{tag}` on the wire"),
        };
        emitted.insert(tag_value, map.keys().cloned().collect());
    }

    let emitted_tags: BTreeSet<String> = emitted.keys().cloned().collect();
    let declared_tags: BTreeSet<String> = declared.keys().cloned().collect();
    compare(&emitted_tags, &declared_tags, ts_name, "variant");

    for (tag_value, fields) in &emitted {
        let declared_fields = declared
            .get(tag_value)
            .expect("tag sets already compared equal");
        compare(
            fields,
            declared_fields,
            &format!("{ts_name} (variant `{tag_value}`)"),
            "field",
        );
    }
}

/// Compare hand-written wire tags against the string union declared on
/// one interface field.
///
/// For the shapes where the wire carries a `String` and no Rust enum
/// reaches the boundary — `TimelineEventDto.change`,
/// `ThemeConfigDto.mode`, `SupportDto.kind`. `serde` proves nothing
/// about those, so the tags must come from the real producer.
pub fn assert_field_tags(tags: &[&str], ts_iface: &str, ts_field: &str) {
    let src = source();
    let declared = interface_field_union(&src, ts_iface, ts_field);
    let emitted: BTreeSet<String> = tags.iter().map(|t| (*t).to_string()).collect();
    compare(
        &emitted,
        &declared,
        &format!("{ts_iface}.{ts_field}"),
        "tag",
    );
}

/// Compare wire tags gathered by hand against a top-level string union.
///
/// For an enum whose wire form can only be observed through a carrier
/// (`TodoState` rides `OutlineNode.todo` behind a `serialize_with`).
pub fn assert_union_tags(tags: &BTreeSet<String>, ts_name: &str) {
    let src = source();
    compare(
        tags,
        &string_union_members(&src, ts_name),
        ts_name,
        "variant",
    );
}

/// Compare wire tags gathered by hand against a *tagged* union's tags.
///
/// For a payload with no Rust type on the wire at all — see
/// `wire_enums.rs`'s deep-link pin.
pub fn assert_union_variant_tags(tags: &BTreeSet<String>, ts_name: &str, tag: &str) {
    let declared: BTreeSet<String> = tagged_union_members(&source(), ts_name, tag)
        .keys()
        .cloned()
        .collect();
    compare(tags, &declared, ts_name, "variant");
}

/// Compare two label sets, with an empty-set guard.
pub fn compare(emitted: &BTreeSet<String>, declared: &BTreeSet<String>, what: &str, noun: &str) {
    // Two empty sets compare equal, and a reader that misunderstood a
    // declaration returns an empty set. Without this, the pins most
    // likely to be wrong are exactly the ones that pass.
    assert!(
        !emitted.is_empty() && !declared.is_empty(),
        "{what}: one side produced no {noun}s at all (backend {}, TypeScript {}). \
         An empty comparison passes and proves nothing — fix the reader, do not \
         accept the green.",
        emitted.len(),
        declared.len()
    );

    let missing_in_ts: Vec<&String> = emitted.difference(declared).collect();
    let missing_in_rust: Vec<&String> = declared.difference(emitted).collect();

    assert!(
        missing_in_ts.is_empty(),
        "{what}: the backend emits the {noun}(s) {missing_in_ts:?}, which the \
         TypeScript side does not declare.\n\
         A client narrowing on this reads them as an unhandled case — silently, \
         at runtime, on a device. Extend the union."
    );
    assert!(
        missing_in_rust.is_empty(),
        "{what}: the TypeScript side declares the {noun}(s) {missing_in_rust:?}, \
         which the backend never emits.\n\
         Either the Rust side lost them, or the union should."
    );
}

/// Build an exhaustive list of an enum's variants.
///
/// The generated `match` is the whole point. `vec![…]` alone pins
/// whatever subset the author happened to type, which is how
/// `ParseWarningKind` shipped five unmodelled variants; the `match`
/// turns "someone forgot" into a compile error in the file that also
/// names the TypeScript union.
///
/// The left side of each arm is a pattern (so a data-carrying variant
/// matches with `{ .. }`), the right an expression that builds one.
// Only `wire_enums.rs` uses it; the other two binaries compile this
// module too and would report it dead.
#[allow(unused_macros)]
macro_rules! wire_variants {
    ($ty:ty; $($pat:pat => $val:expr),+ $(,)?) => {{
        #[allow(dead_code)]
        fn exhaustiveness_guard(v: &$ty) {
            match v {
                $($pat => {}),+
            }
        }
        vec![$($val),+]
    }};
}

#[allow(unused_imports)]
pub(crate) use wire_variants;
