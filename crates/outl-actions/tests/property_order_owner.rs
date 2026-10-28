//! The order a block's properties render in has **one** owner:
//! `outl_actions::tree::sort_in_presentation_order`.
//!
//! It had eight. Seven in this crate and one in `outl-cli`, each a
//! copy-pasted `sort_by(|a, b| a.0.cmp(&b.0))` next to a comment
//! explaining the reason in its own words — and three of those comments
//! gave different reasons. They agreed on the answer, which is what
//! made the split invisible: nothing a user or a test could see was
//! wrong, so "what order do properties render in" had eight homes and
//! no address.
//!
//! That matters because the order is a **decision**, not a fact the op
//! log holds. `Op::SetProp` carries `(node, key, value)` and no slot,
//! so declaration order is unrecoverable and alphabetical is a policy
//! somebody picked ([issue #285](https://github.com/outlmd/outl/issues/285)).
//! A policy spread across eight files cannot be changed, documented, or
//! argued with — changing it means finding all eight and hoping.
//!
//! **This is a lexical ratchet, and it only catches the way the eight
//! actually got there: copy-paste.** A ninth caller writing
//! `sort_by_key(|p| p.0.clone())` passes this and is still the defect.
//! The behavioural half — that the order *is* alphabetical and does not
//! track the `HashMap`'s per-process seed — lives in
//! `src/tree/props.rs`'s own tests.

use std::path::{Path, PathBuf};

/// The comparator every one of the eight spelled out, as a whole
/// statement.
///
/// The `;` is load-bearing: `a.0.cmp(&b.0)` on its own is a legitimate
/// first key in a composite sort (`tree::children`'s sibling order and
/// `person`'s ranking both use it with a `.then_with`), and matching
/// those would turn this test into a list of exceptions — which is a
/// second copy of the fact in the shape of an allowlist.
const PROPERTY_SORT: &str = "sort_by(|a, b| a.0.cmp(&b.0));";

/// The one file allowed to say it, relative to the workspace root.
const OWNER: &str = "crates/outl-actions/src/tree/props.rs";

/// Crates that project properties to a user: this one, the clients that
/// render them, and the two command surfaces that list them.
///
/// Scoped rather than workspace-wide because the signature is a
/// `Vec<(String, String)>` sort, and `outl-core`'s storage layer sorts
/// unrelated pairs of that shape — sweeping it in would mean
/// allowlisting a file that has nothing to do with properties.
const SURFACES: &[&str] = &[
    "outl-actions",
    "outl-cli",
    "outl-tui",
    "outl-md",
    "outl-tauri-shared",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root is two levels above this crate")
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn only_the_tree_module_decides_what_order_properties_render_in() {
    let root = workspace_root();
    let owner = root.join(OWNER);
    assert!(
        owner.is_file(),
        "the owner moved: {OWNER} does not exist. Point this test at its \
         new home rather than deleting it — the ratchet is the reason the \
         other seven copies stayed gone."
    );

    // Per surface, not a total. `rust_sources` returns quietly on a
    // directory it cannot read, so a renamed or dropped crate is scanned
    // as zero files — and a total large enough to look healthy is reached
    // by `outl-actions/src` alone, which would let this ratchet pass over
    // the crate holding the next copy. `outl-cli` held the eighth.
    let mut sources = Vec::new();
    for crate_name in SURFACES {
        let dir = root.join("crates").join(crate_name).join("src");
        let before = sources.len();
        rust_sources(&dir, &mut sources);
        assert!(
            sources.len() > before,
            "no `.rs` file found under {} — this surface does not resolve, \
             so it is being scanned as nothing at all",
            dir.display()
        );
    }

    let offenders: Vec<String> = sources
        .iter()
        .filter(|path| **path != owner)
        .filter(|path| {
            std::fs::read_to_string(path)
                .map(|src| src.contains(PROPERTY_SORT))
                .unwrap_or(false)
        })
        .map(|path| {
            path.strip_prefix(&root)
                .unwrap_or(path)
                .display()
                .to_string()
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "{offenders:?} sort a `(key, value)` list themselves.\n\
         Call `outl_actions::tree::sort_in_presentation_order` instead. \
         If this list is sorting something that is not a property, give \
         it a comparator that says so (sort by the field, or a \
         `sort_by_key`) — the point of this test is that a property \
         order cannot be decided twice."
    );
}
