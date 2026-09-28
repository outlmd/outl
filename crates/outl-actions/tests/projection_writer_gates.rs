//! Every public writer in `journal::apply` carries a recorded verdict:
//! **does it ask the invariant-8 guards before it overwrites a `.md`?**
//!
//! Root `CLAUDE.md` invariant 8 is enforced by three questions — does the
//! file hold block text the op log never saw, would the write delete the
//! page's YAML fence, and is an absent `.md` really absent — and
//! `journal::guard` owns all three. What it cannot own is *who asks
//! them*. That was a sentence in a module doc ("both write gates run all
//! three"), and a sentence cannot fail: `apply_page_md_with_sidecar_rendered`
//! shipped as a public, exported, guard-free `write_page_projection` with
//! no caller anywhere in the repo, and the doc it contradicted went on
//! saying there were two.
//!
//! During the #281 upgrade window — an op log carrying a page's
//! frontmatter fence as bullets, a render that does not carry it — that
//! call is the write that deletes the fence. #281 again, through a door
//! nobody had counted.
//!
//! So the count is a test. A new `pub fn` in `apply.rs` does not pass
//! until somebody writes down which side of the gate it is on, which is
//! the same shape as invariant 12's exhaustive `match`: the gap has to be
//! declared, not discovered by a user.
//!
//! **This asserts the verdict exists, not that it is obeyed.** The
//! behaviour behind `Gate::Guarded` is pinned separately and must stay
//! that way — `crates/outl-actions/src/journal/tests/guard.rs`,
//! `.../if_stale.rs` and `.../frontmatter.rs` are the regression net
//! named in root `CLAUDE.md` invariant 8.

use std::collections::BTreeMap;

/// Which side of the invariant-8 gate a writer sits on.
#[derive(Debug, PartialEq, Eq)]
enum Gate {
    /// A gate: asks `journal::guard` itself and refuses rather than
    /// deleting what the op log cannot account for.
    Guarded,
    /// Not a gate, but every write it performs goes through one.
    Delegates,
    /// Writes whatever it is given. Legitimate only where the caller has
    /// already established there is nothing on disk to lose — the `why`
    /// column is that argument, and it is the thing review reads.
    Ungated,
}

/// The recorded verdict, one row per `pub fn` in `journal/apply.rs`.
///
/// Scoped to that one file on purpose: it is the module `guard.rs` names,
/// and the module whose doc used to carry this fact as prose.
const WRITERS: &[(&str, Gate, &str)] = &[
    (
        "apply_page_md",
        Gate::Ungated,
        "renders and writes the `.md` with no sidecar at all, so it cannot \
         ask a sidecar anything. No caller in the repo",
    ),
    (
        "apply_page_md_with_sidecar",
        Gate::Ungated,
        "unconditional projection. Test-only today: every production path \
         goes through the guarded one",
    ),
    (
        "apply_page_md_with_sidecar_guarded",
        Gate::Guarded,
        "the post-mutation gate — step 5 of `commit_page`",
    ),
    (
        "apply_page_md_with_sidecar_if_absent",
        Gate::Ungated,
        "writes only when the `.md` is absent, so there are no bytes to \
         lose. Does NOT ask `guard_absent_markdown`, so it can still write \
         over an undownloaded iCloud placeholder; the `_if_stale` gate \
         subsumes it and is what the read paths call",
    ),
    (
        "apply_page_md_with_sidecar_if_stale",
        Gate::Guarded,
        "the re-projection gate — page open, and `outl serve`'s sweep",
    ),
    (
        "apply_all_pages_md",
        Gate::Delegates,
        "a sweep over the post-mutation gate; refusals come back per page \
         in `ProjectionSweep::failures`",
    ),
];

/// Every `pub fn` declared in `journal/apply.rs`, in source order.
fn public_writers() -> Vec<String> {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/journal/apply.rs"))
        .expect("read journal/apply.rs");
    src.lines()
        .filter_map(|line| line.trim().strip_prefix("pub fn "))
        .filter_map(|rest| rest.split(['(', '<']).next())
        .map(str::to_string)
        .collect()
}

#[test]
fn every_public_writer_in_apply_has_a_recorded_gate_verdict() {
    let declared: BTreeMap<&str, &Gate> = WRITERS.iter().map(|(n, g, _)| (*n, g)).collect();
    let found = public_writers();

    let undeclared: Vec<&String> = found
        .iter()
        .filter(|n| !declared.contains_key(n.as_str()))
        .collect();
    assert!(
        undeclared.is_empty(),
        "a public writer in journal/apply.rs with no recorded verdict: {undeclared:?}.\n\
         Add a row to WRITERS saying whether it runs the invariant-8 guards, \
         and why that is correct. A writer nobody counted is how \
         `apply_page_md_with_sidecar_rendered` shipped guard-free."
    );

    let stale: Vec<&&str> = declared
        .keys()
        .filter(|n| !found.iter().any(|f| f == *n))
        .collect();
    assert!(
        stale.is_empty(),
        "WRITERS names functions journal/apply.rs no longer has: {stale:?}.\n\
         A verdict for something that does not exist is the same drift in \
         the other direction — drop the row."
    );
}

/// `guard.rs`'s module doc has to name **every** writer in `apply.rs`,
/// not just the gates.
///
/// It said "both write gates in `super::apply` run all three" while
/// `apply.rs` held six public writers, two of them gates. Nothing in that
/// sentence is false, and it still read as a census — which is how a
/// public, exported, guard-free writer sat next to it uncounted. Prose
/// that lists some of a set is indistinguishable from prose that lists
/// all of it, so the doc names them all and this test is what keeps the
/// list from going stale.
#[test]
fn the_guard_doc_names_every_writer_and_says_which_are_gates() {
    let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/journal/guard.rs"))
        .expect("read journal/guard.rs");
    let header: String = doc
        .lines()
        .take_while(|l| l.starts_with("//!"))
        .collect::<Vec<_>>()
        .join("\n");

    let missing: Vec<&str> = WRITERS
        .iter()
        .map(|(n, _, _)| *n)
        .filter(|n| !header.contains(n))
        .collect();
    assert!(
        missing.is_empty(),
        "guard.rs's module doc leaves out {missing:?}. A writer the doc \
         does not name is a writer a reader assumes is one of the ones it \
         does."
    );

    let gates = WRITERS
        .iter()
        .filter(|(_, g, _)| *g == Gate::Guarded)
        .count();
    assert_eq!(gates, 2, "the gate count moved; guard.rs's doc says two");
}
