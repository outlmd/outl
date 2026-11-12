//! `--force` replaces the pages an import writes instead of merging
//! into them, and a trashed page never gets its old children back.

mod common;

use common::{import_into, open_test_ws, TestWs};
use outl_actions::tree::{children_of, is_trashed};
use outl_core::id::NodeId;
use outl_import::adapters::RoamAdapter;
use outl_import::ImportOptions;
use std::fs;
use std::path::PathBuf;

const V1: &str = r#"[
    {"title": "Alpha", "children": [
        {"string": "alpha one", "uid": "a1", "children": []},
        {"string": "alpha two", "uid": "a2", "children": [
            {"string": "alpha two child", "uid": "a3", "children": []}]}]},
    {"title": "Beta", "children": [{"string": "beta one", "uid": "b1", "children": []}]}
]"#;
const V2: &str = r#"[
    {"title": "Alpha", "children": [
        {"string": "alpha one EDITED", "uid": "a1", "children": []},
        {"string": "alpha four", "uid": "a4", "children": []}]},
    {"title": "Gamma", "children": [{"string": "gamma", "uid": "g1", "children": []}]}
]"#;

fn src(json: &str) -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().expect("tempdir");
    let p = d.path().join("backup.json");
    fs::write(&p, json).expect("write");
    (d, p)
}

fn run(ws: &mut TestWs, json: &str) -> outl_import::ImportReport {
    let (_d, p) = src(json);
    import_into(&RoamAdapter, &p, ws, &ImportOptions::default())
}

fn live_texts(ws: &TestWs, slug: &str) -> Vec<String> {
    let page = NodeId::from_slug(slug);
    assert!(!is_trashed(&ws.workspace, page), "{slug} must be live");
    children_of(&ws.workspace, page)
        .into_iter()
        .map(|(id, _)| ws.workspace.block_text(id).unwrap_or_default())
        .collect()
}

#[test]
fn no_sidecar_replaces_the_old_blocks_instead_of_merging() {
    let mut ws = open_test_ws();
    run(&mut ws, V1);
    // A replica holds the page in the op log without projecting it.
    fs::remove_file(ws.root.join("pages/alpha.md")).expect("rm md");
    fs::remove_file(ws.root.join("pages/alpha.outl")).expect("rm sidecar");

    let report = run(&mut ws, V2);

    assert_eq!(live_texts(&ws, "alpha"), ["alpha one EDITED", "alpha four"]);
    assert_eq!(report.replaced_blocks, 3, "two top blocks + one child");
    let log = fs::read_to_string(ws.root.join(".outl/orphans.log")).expect("orphans.log");
    assert_eq!(log.lines().count(), 2, "one entry per discarded child");
    // Pages the source does not have are left alone.
    assert_eq!(live_texts(&ws, "beta"), ["beta one"]);
}

#[test]
fn a_trashed_page_comes_back_with_only_the_new_blocks() {
    let mut ws = open_test_ws();
    run(&mut ws, V1);
    let alpha = NodeId::from_slug("alpha");
    let old: Vec<NodeId> = children_of(&ws.workspace, alpha)
        .into_iter()
        .map(|(i, _)| i)
        .collect();
    outl_actions::block::delete(&mut ws.workspace, &ws.hlc, alpha).expect("trash page");
    fs::remove_file(ws.root.join("pages/alpha.md")).expect("rm md");
    fs::remove_file(ws.root.join("pages/alpha.outl")).expect("rm sidecar");

    run(&mut ws, V2);

    assert_eq!(live_texts(&ws, "alpha"), ["alpha one EDITED", "alpha four"]);
    for id in old {
        assert!(is_trashed(&ws.workspace, id), "old block must stay trashed");
    }
}

#[test]
fn reimport_with_a_sidecar_replaces_through_the_matcher_and_discards_nothing_extra() {
    let mut ws = open_test_ws();
    run(&mut ws, V1);
    let report = run(&mut ws, V2);
    assert_eq!(live_texts(&ws, "alpha"), ["alpha one EDITED", "alpha four"]);
    assert_eq!(report.replaced_blocks, 0);
}

#[test]
fn a_rerun_over_finished_pages_is_stable() {
    let mut ws = open_test_ws();
    run(&mut ws, V1);
    let report = run(&mut ws, V1);
    assert_eq!(live_texts(&ws, "alpha"), ["alpha one", "alpha two"]);
    assert_eq!(report.replaced_blocks, 0);
}
