//! `not-tag:` / `not-prop:` end-to-end through the real index.
//!
//! The unit tests in `runtimes/query/engine.rs` build a `BlockEntry`
//! by hand, so they prove the matcher and nothing about where its
//! inputs come from. These drive the whole path a user's fence takes:
//! blocks projected from the tree into a `WorkspaceIndex`, the DSL
//! parsed from the fence body, embeds out the other end.
//!
//! What that catches and a unit test cannot: a block property that
//! never reaches `BlockEntry::properties` makes every `not-prop:`
//! trivially true — the filter would look like it works (it returns
//! results) while excluding nothing at all.

#![cfg(feature = "lang-query")]

use std::path::{Path, PathBuf};

use outl_core::id::NodeId;
use outl_exec::{ExecContext, RuntimeRegistry};
use outl_md::block_index::IdentifiedNode;
use outl_md::index::{PageEntry, WorkspaceIndex};
use tempfile::TempDir;

/// One block: text, plus the `key:: value` properties it carries.
struct Block(&'static str, &'static [(&'static str, &'static str)]);

fn index_with(blocks: &[Block], root: &Path) -> (WorkspaceIndex, Vec<NodeId>) {
    let mut idx = WorkspaceIndex::default();
    let page_path = root.join("pages/notes.md");
    idx.insert_page(PageEntry {
        path: page_path.clone(),
        slug: "notes".to_string(),
        title: "Notes".to_string(),
        icon: None,
        is_journal: false,
        pinned: false,
        page_type: None,
    });
    let nodes: Vec<IdentifiedNode> = blocks
        .iter()
        .map(|Block(text, props)| IdentifiedNode {
            id: NodeId::new(),
            text: (*text).to_string(),
            properties: props
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            children: Vec::new(),
        })
        .collect();
    let ids = nodes.iter().map(|n| n.id).collect();
    idx.collect_page_blocks_from_tree("notes", &page_path, &nodes);
    idx.collect_refs_from_indexed();
    (idx, ids)
}

fn empty_root() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("pages")).unwrap();
    let root = dir.path().to_path_buf();
    (dir, root)
}

/// Assert that `dsl` matched exactly `expected`.
///
/// Order-insensitive on purpose: the runtime walks a hash map and only
/// `sort:` promises an order, so pinning emission order here would be
/// a flaky test asserting something the DSL never claimed.
fn assert_matches(dsl: &str, index: &WorkspaceIndex, root: &Path, expected: &[NodeId]) {
    let mut got = run(dsl, index, root);
    let mut want = expected.to_vec();
    got.sort();
    want.sort();
    assert_eq!(got, want, "query:\n{dsl}");
}

/// Run `dsl` against `index` and return the ids it matched.
fn run(dsl: &str, index: &WorkspaceIndex, root: &Path) -> Vec<NodeId> {
    let runtime = RuntimeRegistry::with_builtins()
        .get("query")
        .expect("the query runtime is registered");
    let ctx = ExecContext {
        workspace_root: root.to_path_buf(),
        index: Some(index),
        ..Default::default()
    };
    let out = runtime.execute(dsl, &ctx).expect("the query runs");
    out.stdout
        .lines()
        .map(|line| {
            let handle = line
                .trim_start_matches("!((")
                .trim_end_matches("))")
                .to_string();
            index
                .resolve_block_ref(&handle)
                .unwrap_or_else(|| panic!("handle {handle} resolves"))
                .id
        })
        .collect()
}

#[test]
fn not_tag_excludes_only_the_blocks_carrying_that_tag() {
    let (_dir, root) = empty_root();
    let (index, ids) = index_with(
        &[
            Block("TODO ship the parser #work", &[]),
            Block("TODO read the paper #work #research", &[]),
            Block("TODO plan the offsite #work #someday", &[]),
        ],
        &root,
    );

    assert_matches(
        "status: todo\ntag: work\nnot-tag: research",
        &index,
        &root,
        &[ids[0], ids[2]],
    );
}

#[test]
fn repeated_not_tag_lines_and_against_each_other() {
    // The issue's motivating query: open work tasks, minus anything
    // parked under #research / #future / #someday.
    let (_dir, root) = empty_root();
    let (index, ids) = index_with(
        &[
            Block("TODO ship the parser #work", &[]),
            Block("TODO read the paper #work #research", &[]),
            Block("TODO plan the offsite #work #someday", &[]),
            Block("TODO rewrite it all #work #future", &[]),
        ],
        &root,
    );

    assert_matches(
        "status: todo\ntag: work\nnot-tag: research\nnot-tag: future\nnot-tag: someday",
        &index,
        &root,
        &[ids[0]],
    );
}

#[test]
fn not_tag_does_not_swallow_a_longer_tag_that_starts_the_same() {
    // Issue #323 open question 1, at the surface the user sees: a
    // substring negative would hide `#workflow` behind `not-tag: work`
    // and the block would simply be gone, with nothing to notice.
    let (_dir, root) = empty_root();
    let (index, ids) = index_with(
        &[
            Block("TODO document the #workflow", &[]),
            Block("TODO ship it #work", &[]),
        ],
        &root,
    );

    assert_matches("status: todo\nnot-tag: work", &index, &root, &[ids[0]]);
}

#[test]
fn a_namespace_child_answers_to_both_sides_of_its_parent() {
    let (_dir, root) = empty_root();
    let (index, ids) = index_with(
        &[
            Block("TODO roll it out #ops/deploy", &[]),
            Block("TODO write the postmortem #docs", &[]),
        ],
        &root,
    );

    assert_matches("tag: ops", &index, &root, &[ids[0]]);
    assert_matches("not-tag: ops", &index, &root, &[ids[1]]);
}

#[test]
fn a_block_property_reaches_the_index_and_not_prop_can_see_it() {
    let (_dir, root) = empty_root();
    let (index, ids) = index_with(
        &[
            Block("TODO ship the parser", &[]),
            Block("TODO revisit later", &[("status", "parked")]),
            Block("TODO chase the flake", &[("status", "blocked")]),
        ],
        &root,
    );

    // Bare key: anything carrying `status::` at all is dropped.
    assert_matches("not-prop: status", &index, &root, &[ids[0]]);
    // Key + value: only that pair is dropped.
    assert_matches("not-prop: status: parked", &index, &root, &[ids[0], ids[2]]);
    // And the positive half sees the same properties.
    assert_matches("prop: status: parked", &index, &root, &[ids[1]]);
}

#[test]
fn a_negative_filter_ands_with_every_other_directive() {
    let (_dir, root) = empty_root();
    let (index, ids) = index_with(
        &[
            Block("TODO ship the parser #work", &[]),
            Block("DONE ship the docs #work", &[]),
            Block("TODO park this #work", &[("status", "parked")]),
        ],
        &root,
    );

    assert_matches(
        "status: todo\ntag: work\nnot-prop: status: parked",
        &index,
        &root,
        &[ids[0]],
    );
}

#[test]
fn a_fence_asking_for_a_tag_and_its_negation_returns_nothing() {
    // The complement law, at the surface. If this ever returns a hit,
    // the two sides have drifted into different notions of "has tag".
    let (_dir, root) = empty_root();
    let (index, _ids) = index_with(
        &[
            Block("TODO ship it #ops", &[]),
            Block("TODO roll out #ops/deploy", &[]),
            Block("TODO audit #opsec", &[]),
            Block("TODO nothing tagged", &[]),
        ],
        &root,
    );

    assert!(run("tag: ops\nnot-tag: ops", &index, &root).is_empty());
    assert!(run("prop: x\nnot-prop: x", &index, &root).is_empty());
}

#[test]
fn a_malformed_negative_filter_is_reported_not_ignored() {
    let (_dir, root) = empty_root();
    let (index, _ids) = index_with(&[Block("TODO ship it #work", &[])], &root);

    let runtime = RuntimeRegistry::with_builtins().get("query").unwrap();
    let ctx = ExecContext {
        workspace_root: root,
        index: Some(&index),
        ..Default::default()
    };
    // Silently treating `not-tag:` as "no filter" would return
    // everything; treating it as "match any tag" would return
    // nothing. Both are wrong answers dressed as results.
    let err = runtime.execute("status: todo\nnot-tag:", &ctx).unwrap_err();
    assert!(
        err.to_string().contains("line 2"),
        "the error should point at the offending line, got {err}"
    );
}

#[test]
fn every_directive_has_a_working_negative() {
    // The generic `Filter::Not` means a new directive gets its
    // negative with no extra wiring. This is the surface proof: the
    // negative of each key returns exactly the blocks the positive
    // did not.
    let (_dir, root) = empty_root();
    let (index, ids) = index_with(
        &[
            Block("TODO ship the parser #ops", &[("status", "live")]),
            Block("DONE ship the docs #docs", &[]),
        ],
        &root,
    );

    for (pos, neg) in [
        ("status: todo", "not-status: todo"),
        ("tag: ops", "not-tag: ops"),
        ("prop: status", "not-prop: status"),
        ("text: parser", "not-text: parser"),
    ] {
        assert_matches(pos, &index, &root, &[ids[0]]);
        assert_matches(neg, &index, &root, &[ids[1]]);
        // And together they cancel, on every key.
        assert!(
            run(&format!("{pos}\n{neg}"), &index, &root).is_empty(),
            "{pos} + {neg} must return nothing"
        );
    }
}

#[test]
fn not_kind_and_not_since_cancel_their_positives() {
    // Both read the hosting page rather than the block, so they are
    // worth their own case: `kind` flips on a page property and
    // `since` on a date parsed out of the slug.
    let (_dir, root) = empty_root();
    let (index, _ids) = index_with(&[Block("TODO ship it", &[])], &root);
    for pair in ["kind: page\nnot-kind: page", "since: 7d\nnot-since: 7d"] {
        assert!(
            run(pair, &index, &root).is_empty(),
            "{pair} must return nothing"
        );
    }
}
