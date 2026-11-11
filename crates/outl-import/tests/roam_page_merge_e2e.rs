//! End-to-end: Roam source pages that land on one outl page identity
//! (same slug, or same journal date) merge instead of colliding.

mod common;

use common::{import_with, read, TestWs};
use outl_import::adapters::RoamAdapter;
use outl_import::ImportReport;
use std::fs;

fn import_fixture(json: &str) -> (TestWs, ImportReport) {
    let src_dir = tempfile::tempdir().expect("src tempdir");
    let src = src_dir.path().join("backup.json");
    fs::write(&src, json).expect("write fixture");
    import_with(&RoamAdapter, &src)
}

/// Roam is case-sensitive about page names and outl is not: outl
/// derives a page's id from its slug and resolves `[[Name]]` by slug
/// first. Two Roam pages on one slug used to land as `foo-bar.md` and
/// `foo-bar-2.md`, and every link to the second one opened the first.
/// They are one outl page, so they merge: all blocks kept, the other
/// spelling kept as an alias, every ref resolvable.
#[test]
fn pages_sharing_a_slug_merge_and_keep_every_name() {
    let json = r#"[
        {"title": "foo bar", "children": [{"string": "b", "uid": "b1", "children": []}]},
        {"title": "Foo Bar", "children": [
            {"string": "a", "uid": "a1", "children": []},
            {"string": "a2", "uid": "a2", "children": []}
        ]},
        {"title": "Refs", "children": [
            {"string": "[[foo bar]] ((b1)) ((a1))", "uid": "r1", "children": []}
        ]}
    ]"#;
    let (ws, report) = import_fixture(json);
    assert!(!ws.root.join("pages/foo-bar-2.md").exists());
    let page = read(&ws.root.join("pages/foo-bar.md"));
    assert!(page.contains("title:: Foo Bar\n"), "page:\n{page}");
    assert!(page.contains("alias:: foo bar\n"), "page:\n{page}");
    let sc = outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(
        &ws.root.join("pages/foo-bar.md"),
    ))
    .expect("sidecar");
    let texts: Vec<&str> = sc.blocks.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(
        texts,
        ["a", "a2", "b"],
        "the fuller page leads, the rest follows"
    );
    assert_eq!(report.slug_collisions, 1);
    assert_eq!(report.pages_merged, 1);
    assert_eq!(report.refs_resolved, 2);
    assert_eq!(report.refs_page_fallback, 0);
    assert!(report.reconciliation.expect("reconciled").balanced);
}

/// A page whose own `alias::` exists keeps one alias line, extended.
#[test]
fn a_merged_page_extends_an_existing_alias_line() {
    let json = r#"[
        {"title": "Rust", "children": [
            {"string": "alias:: rustlang", "uid": "p1", "children": []},
            {"string": "x", "uid": "x1", "children": []}
        ]},
        {"title": "rust", "children": []}
    ]"#;
    let (ws, _report) = import_fixture(json);
    let page = read(&ws.root.join("pages/rust.md"));
    assert_eq!(page.matches("alias::").count(), 1, "page:\n{page}");
    assert!(page.contains("alias:: rustlang, rust\n"), "page:\n{page}");
}

/// Roam can hold both `March 19th, 2024` and a page titled `2024-03-19`.
/// Both are the 2024-03-19 journal in outl (and `[[2024-03-19]]` opens
/// it), so they merge; Roam's own daily note leads whatever order the
/// backup listed them in.
#[test]
fn a_same_date_page_merges_after_roams_daily_note() {
    let json = r#"[
        {"title": "2024-03-19", "children": [{"string": "iso", "uid": "i1", "children": []}]},
        {"title": "March 19th, 2024", "children": [{"string": "daily", "uid": "d1", "children": []}]}
    ]"#;
    let (ws, report) = import_fixture(json);
    let sc = outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(
        &ws.root.join("journals/2024-03-19.md"),
    ))
    .expect("sidecar");
    let texts: Vec<&str> = sc.blocks.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(texts, ["daily", "iso"]);
    assert_eq!(report.pages_merged, 1);
    assert!(report.reconciliation.expect("reconciled").balanced);
}
