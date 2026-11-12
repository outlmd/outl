//! End-to-end: with `--preserve-timestamps`, a `created::` / `edited::`
//! attribute the user wrote on a block wins over Roam's own block
//! timestamps, in the file and in the op log.

mod common;

use common::{import_with_opts, read, TestWs};
use outl_import::adapters::RoamAdapter;
use outl_import::ImportOptions;
use std::fs;

fn import_stamped(json: &str) -> TestWs {
    let src_dir = tempfile::tempdir().expect("src tempdir");
    let src = src_dir.path().join("backup.json");
    fs::write(&src, json).expect("write fixture");
    let opts = ImportOptions {
        preserve_timestamps: true,
        ..Default::default()
    };
    import_with_opts(&RoamAdapter, &src, &opts).0
}

/// The page as the op log projects it, the way every later write
/// re-renders it.
fn projected(ws: &TestWs, rel: &str) -> String {
    let path = ws.root.join(rel);
    let sc = outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(&path)).expect("sidecar");
    outl_actions::journal::render_page_md(&ws.workspace, sc.page_id)
}

#[test]
fn a_user_created_attribute_beats_the_block_timestamp() {
    // A nested pure-attribute block (lifted to block properties) and a
    // block with prose that carries its own `edited::` line.
    let json = r#"[
        {"title": "Clippings", "children": [
            {"string": "saved an article", "uid": "c0", "children": [
                {"string": "source:: somewhere\ncreated:: 2024-01-02", "uid": "c1",
                 "create-time": 1700000000000, "edit-time": 1700000500000, "children": []}
            ]},
            {"string": "a note\nedited:: by hand", "uid": "c2",
             "create-time": 1700000000000, "edit-time": 1700000500000, "children": []}
        ]}
    ]"#;
    let ws = import_stamped(json);
    for text in [
        read(&ws.root.join("pages/clippings.md")),
        projected(&ws, "pages/clippings.md"),
    ] {
        assert!(text.contains("created:: 2024-01-02\n"), "page:\n{text}");
        assert!(text.contains("edited:: by hand\n"), "page:\n{text}");
        // Block c1 keeps its user `created::` and still gets Roam's
        // `edited::`; block c2 keeps its user `edited::` and still gets
        // Roam's `created::`. Block c0 has no stamps.
        assert_eq!(
            text.matches("created:: ").count(),
            2,
            "one user value, one Roam stamp:\n{text}"
        );
        assert_eq!(
            text.matches("edited:: ").count(),
            2,
            "one user value, one Roam stamp:\n{text}"
        );
    }
}

#[test]
fn blocks_without_their_own_attribute_keep_both_stamps() {
    let json = r#"[
        {"title": "Plain", "children": [
            {"string": "just text", "uid": "p1",
             "create-time": 1700000000000, "edit-time": 1700000500000, "children": []}
        ]}
    ]"#;
    let ws = import_stamped(json);
    let text = projected(&ws, "pages/plain.md");
    assert!(
        text.contains("created:: 2023-11-14T22:13:20"),
        "page:\n{text}"
    );
    assert!(
        text.contains("edited:: 2023-11-14T22:21:40"),
        "page:\n{text}"
    );
}
