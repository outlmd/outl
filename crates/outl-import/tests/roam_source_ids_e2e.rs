//! End-to-end: `--keep-source-ids` lands every Roam block as one outl
//! block carrying its Roam uid, pages too, in the file and in the op log.

mod common;

use common::{import_with_opts, read, TestWs};
use outl_import::adapters::RoamAdapter;
use outl_import::{ImportOptions, ImportReport};
use std::fs;

const FIXTURE: &str = r#"[
    {"title": "Ana Lima", "uid": "pgAna0001", "children": [
        {"string": "icon:: 👤", "uid": "attrIcon1", "create-time": 1700000000000, "edit-time": 1700000100000, "children": []},
        {"string": "type:: person\ntitle:: Ana", "uid": "attrType1", "children": []},
        {"string": "met at the meetup", "uid": "body00001", "children": [
            {"string": "{{[[table]]}}", "uid": "table0001", "children": [
                {"string": "Time", "uid": "cellT0001", "children": [
                    {"string": "PM", "uid": "cellP0001", "children": []}
                ]},
                {"string": "10:00", "uid": "cellV0001", "children": [
                    {"string": "Ana", "uid": "cellA0001", "children": []}
                ]}
            ]}
        ]}
    ]},
    {"title": "@Ana Lima", "uid": "pgAna0002", "children": [
        {"string": "second spelling", "uid": "body00002", "children": []}
    ]}
]"#;

fn import(keep_source_ids: bool) -> (TestWs, ImportReport) {
    let src_dir = tempfile::tempdir().expect("src tempdir");
    let src = src_dir.path().join("backup.json");
    fs::write(&src, FIXTURE).expect("write fixture");
    let opts = ImportOptions {
        preserve_timestamps: true,
        keep_source_ids,
        ..Default::default()
    };
    import_with_opts(&RoamAdapter, &src, &opts)
}

/// The page as the op log projects it, the way every later write
/// re-renders it.
fn projected(ws: &TestWs, rel: &str) -> String {
    let path = ws.root.join(rel);
    let sc = outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(&path)).expect("sidecar");
    outl_actions::journal::render_page_md(&ws.workspace, sc.page_id)
}

const BLOCK_UIDS: [&str; 9] = [
    "attrIcon1",
    "attrType1",
    "body00001",
    "table0001",
    "cellT0001",
    "cellP0001",
    "cellV0001",
    "cellA0001",
    "body00002",
];

#[test]
fn every_block_and_page_keeps_its_roam_uid() {
    let (ws, report) = import(true);
    let rel = "pages/ana-lima.md";
    for text in [read(&ws.root.join(rel)), projected(&ws, rel)] {
        for uid in BLOCK_UIDS {
            assert_eq!(
                text.matches(&format!("roam-uid:: {uid}\n")).count(),
                1,
                "{uid}:\n{text}"
            );
        }
        // The merged page lists both page uids.
        assert!(
            text.contains("roam-uid:: pgAna0001, pgAna0002\n"),
            "page:\n{text}"
        );
        // The header still carries the page attributes outl reads...
        let header = format!("{}\n", text.split("\n\n").next().unwrap());
        assert!(header.contains("icon:: 👤\n"), "header:\n{header}");
        assert!(header.contains("type:: person\n"), "header:\n{header}");
        assert_eq!(header.matches("title::").count(), 1, "header:\n{header}");
        // ...and the attribute block is still a block with its stamps.
        let body = &text[header.len()..];
        assert!(body.contains("icon:: 👤\n"), "body:\n{body}");
        assert!(
            body.contains("created:: 2023-11-14T22:13:20"),
            "body:\n{body}"
        );
        // The table keeps its cell blocks.
        assert!(body.contains("- {{[[table]]}}\n"), "body:\n{body}");
        assert!(body.contains("- PM\n"), "body:\n{body}");
    }
    assert_eq!(report.blocks_lifted_to_props, 0);
    assert_eq!(report.blocks_folded_into_tables, 0);
    assert_eq!(report.tables_converted, 0);
    let r = report.reconciliation.as_ref().expect("reconciliation");
    assert!(r.balanced, "reconciliation: {r:?}");
    assert_eq!(r.emitted_blocks, BLOCK_UIDS.len());
}

#[test]
fn without_the_flag_nothing_changes() {
    let (ws, report) = import(false);
    let text = read(&ws.root.join("pages/ana-lima.md"));
    assert!(!text.contains("roam-uid"), "page:\n{text}");
    assert_eq!(report.blocks_lifted_to_props, 2);
    assert_eq!(report.tables_converted, 1);
    assert!(text.contains("| Time | PM |"), "page:\n{text}");
}
