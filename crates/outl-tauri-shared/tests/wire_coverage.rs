//! Is every TypeScript wire declaration watched by something?
//!
//! The pins are only as good as their coverage, and coverage is only as
//! good as the universe it is measured against. This gate used to walk
//! exactly one file, `types.ts`. It reported 26 of 34 declarations
//! pinned and the remaining 8 exempted — complete coverage of a universe
//! it had chosen, and silent about fourteen mirrors living in three
//! other files, plus every enum in all of them (a `serde` enum is not a
//! JSON object, and the struct comparison panics on anything that is
//! not).
//!
//! The honest reading of the old number was 26 pinned out of 57
//! declarations, reported as 34 of 34. **A gate that overstates its
//! reach is worse than no gate**: the number it prints is what stops the
//! next person from looking, and `ParseWarningKind` sat five variants
//! short the whole time.

mod ts_parser;

use std::collections::{BTreeMap, BTreeSet};

use ts_parser::{mirror_source, MIRROR_FILES};

const PIN_FILES: &[&str] = &[
    "tests/wire_types.rs",
    "tests/wire_plugins.rs",
    "tests/wire_enums.rs",
    "tests/wire_mirrors.rs",
];

/// Call sites whose **first string literal argument** is the TypeScript
/// declaration being pinned.
const PIN_CALLS: &[&str] = &[
    "assert_wire_shape(",
    "assert_string_enum(",
    "assert_tagged_enum(",
    "assert_field_tags(",
    "assert_union_tags(",
    "assert_union_variant_tags(",
];

/// Wire payloads the backend builds with `serde_json::json!` — no Rust
/// type, so no full pin can exist for them however wide the reader's
/// walk gets.
///
/// This is a **note, not an exemption**. Each row still has to be
/// pinned some other way or exempted elsewhere; recording it is what
/// keeps "we settled for a partial pin" from reading the same as "this
/// is covered". The distinction is the whole reason the old gate's
/// number was wrong.
///
/// The list is down to one. `ref-projection-failed` was the other, and
/// it is now `outl_tauri_shared::state::RefProjectionFailed` — the same
/// two keys on the wire, plus something a test can hold.
const UNTYPED_EVENTS: &[(&str, &str)] = &[(
    "DeepLinkNavigate",
    "built with `serde_json::json!` in both clients' `lib.rs`; \
     `wire_enums.rs` pins the `outl_actions::DeepLinkTarget` variants those \
     two builders match on, which is the drift that actually happens",
)];

/// Every TypeScript wire declaration is pinned, or says why not.
#[test]
fn every_wire_declaration_is_pinned_or_declared_unpinned() {
    let declared = declared_wire_types();
    assert!(
        declared.len() > 50,
        "only {} declarations parsed out of {} mirror files — the reader is \
         looking at the wrong shape and this test proves nothing",
        declared.len(),
        MIRROR_FILES.len()
    );

    let pinned = pinned_declaration_names();
    // `UNTYPED_EVENTS` deliberately does not feed this set: a payload
    // with no Rust type still has to be pinned some other way, and
    // letting the note double as an excuse is how a gap goes quiet.
    let exempt: BTreeSet<&str> = wire_mirrors_foreign_gaps().map(|(name, _)| *name).collect();

    let unwatched: Vec<String> = declared
        .iter()
        .filter(|(name, _)| !pinned.contains(name.as_str()) && !exempt.contains(name.as_str()))
        .map(|(name, file)| format!("{name} ({file})"))
        .collect();
    assert!(
        unwatched.is_empty(),
        "these wire declarations have no pin and no recorded gap: {unwatched:#?}.\n\
         A field or variant added to the Rust side of one of these reaches the \
         frontend as `undefined` (or an unhandled case) with nothing failing. \
         Add a pin, or a row in UNTYPED_EVENTS / FOREIGN_CRATE_GAPS with the reason."
    );

    for (name, why) in UNTYPED_EVENTS {
        assert!(
            declared.contains_key(*name),
            "UNTYPED_EVENTS names {name}, which no mirror file declares — stale row"
        );
        assert!(
            !why.trim().is_empty(),
            "the gap recorded for {name} has no reason"
        );
        assert!(
            pinned.contains(*name) || exempt.contains(name),
            "{name} has no Rust type AND no pin of any kind. The note is a \
             record of how the shape is covered, not permission to skip it."
        );
    }

    // The report is part of the test, not a comment that can rot beside
    // it, and it counts the intersection rather than the pin-call count:
    // a pin naming something no mirror declares is not coverage, and
    // adding it to the numerator is exactly how a gate flatters itself.
    // `cargo test -- --nocapture` prints the real ratio.
    let covered = declared
        .keys()
        .filter(|name| pinned.contains(name.as_str()))
        .count();
    eprintln!(
        "wire pin coverage: {covered}/{} declarations across {} mirror files \
         ({} recorded gaps, {} partial)",
        declared.len(),
        MIRROR_FILES.len(),
        exempt.len(),
        UNTYPED_EVENTS.len()
    );
    assert_eq!(
        covered + exempt.len(),
        declared.len(),
        "the coverage arithmetic does not close: every declaration is either \
         pinned or a recorded gap, so these must sum"
    );
}

/// Every mirror file contributes at least one declaration.
///
/// A typo in a `MIRROR_FILES` path would panic on read, but a file that
/// stopped declaring wire types (everything moved to `@outl/shared`,
/// say) would quietly shrink the universe the gate reports on. This is
/// the cheap way to notice.
#[test]
fn every_mirror_file_declares_something() {
    for rel in MIRROR_FILES {
        let count = declarations_in(&mirror_source(rel)).len();
        assert!(
            count > 0,
            "{rel} declares no wire type — either it is no longer a mirror \
             (drop it from MIRROR_FILES) or the reader stopped understanding it"
        );
    }
}

/// No two mirror files declare the same name.
///
/// The readers search one concatenation of every mirror file, which is
/// only sound while names are unique across them. A duplicate would
/// pin whichever copy came first and leave its twin unwatched —
/// precisely the by-omission drift this file exists to catch, wearing
/// a passing test as a disguise.
#[test]
fn no_two_mirror_files_declare_the_same_name() {
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();
    for rel in MIRROR_FILES {
        for name in declarations_in(&mirror_source(rel)) {
            if let Some(first) = seen.insert(name.clone(), rel) {
                panic!(
                    "`{name}` is declared in both {first} and {rel}. The wire readers \
                     concatenate the mirror files, so one of the two would be pinned \
                     and the other silently ignored."
                );
            }
        }
    }
}

/// Every `export interface X` / `export type X =` in a mirror file,
/// mapped to the file that declares it.
fn declared_wire_types() -> BTreeMap<String, &'static str> {
    let mut out = BTreeMap::new();
    for rel in MIRROR_FILES {
        for name in declarations_in(&mirror_source(rel)) {
            out.insert(name, *rel);
        }
    }
    out
}

/// Names declared at the top level of one TypeScript source.
///
/// A re-export (`export type { PropertyKey } from …`) declares nothing
/// — it names a shape pinned where it is defined — so it is skipped.
fn declarations_in(src: &str) -> Vec<String> {
    src.lines()
        .filter_map(|line| {
            let rest = line
                .strip_prefix("export interface ")
                .or_else(|| line.strip_prefix("export type "))?;
            let name = rest.split_whitespace().next()?;
            let declares_something = line.contains('{') || line.contains('=');
            (declares_something && !name.starts_with('{')).then(|| name.to_string())
        })
        .collect()
}

/// TypeScript names the pin files pass to an assertion.
///
/// Read out of the sources rather than tracked by hand: a list
/// maintained beside the calls is a list that drifts from them.
fn pinned_declaration_names() -> BTreeSet<String> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut names = BTreeSet::new();
    for rel in PIN_FILES {
        let src = std::fs::read_to_string(root.join(rel))
            .unwrap_or_else(|e| panic!("cannot read {rel}: {e}"));
        for call in PIN_CALLS {
            for (idx, _) in src.match_indices(call) {
                let rest = &src[idx + call.len()..];
                let Some(open) = rest.find('"') else { continue };
                let after = &rest[open + 1..];
                let Some(close) = after.find('"') else {
                    continue;
                };
                names.insert(after[..close].to_string());
            }
        }
    }
    names
}

/// `wire_mirrors.rs`'s `FOREIGN_CRATE_GAPS`, read out of its source.
///
/// A second test binary cannot be linked against, and duplicating the
/// table here would put the reason in two places — the exact failure
/// mode the eight wrong `UNPINNED` reasons came from.
fn wire_mirrors_foreign_gaps() -> impl Iterator<Item = &'static (&'static str, &'static str)> {
    // Kept as a compile-time mirror of the names only; the reasons live
    // in `wire_mirrors.rs` and are asserted there.
    const NAMES: &[(&str, &str)] = &[
        ("Settings", "see wire_mirrors.rs FOREIGN_CRATE_GAPS"),
        (
            "PeerPairingTicketPayload",
            "see wire_mirrors.rs FOREIGN_CRATE_GAPS",
        ),
    ];
    NAMES.iter()
}

/// The names above really are the ones `wire_mirrors.rs` records.
///
/// Two tables, one fact — so the fact gets a test rather than a hope.
#[test]
fn the_foreign_crate_gap_names_match_wire_mirrors() {
    let src = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wire_mirrors.rs"),
    )
    .expect("wire_mirrors.rs is readable");
    let table = src
        .split_once("pub const FOREIGN_CRATE_GAPS")
        .expect("wire_mirrors.rs declares FOREIGN_CRATE_GAPS")
        .1;
    let table = &table[..table.find("];").expect("the table is terminated")];
    for (name, _) in wire_mirrors_foreign_gaps() {
        assert!(
            table.contains(&format!("\"{name}\"")),
            "{name} is treated as a foreign-crate gap here but wire_mirrors.rs \
             no longer records it — one of the two is stale"
        );
    }
    let rows = table.matches("outl-desktop").count();
    assert_eq!(
        rows,
        wire_mirrors_foreign_gaps().count(),
        "wire_mirrors.rs records a different number of foreign-crate gaps than \
         this file exempts"
    );
}
