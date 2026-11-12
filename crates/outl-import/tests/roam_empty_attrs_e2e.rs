//! End-to-end: a Roam attribute written with no value (`published::`)
//! keeps its `key::` line, in the file and in the op log projection,
//! including on a page whose refs get rewritten after the first write.

mod common;

use common::{import_with, read, TestWs};
use outl_import::adapters::RoamAdapter;
use std::fs;

fn import_json(json: &str) -> TestWs {
    let src_dir = tempfile::tempdir().expect("src tempdir");
    let src = src_dir.path().join("backup.json");
    fs::write(&src, json).expect("write fixture");
    import_with(&RoamAdapter, &src).0
}

/// The page as the op log projects it, the way every later write
/// re-renders it.
fn projected(ws: &TestWs, rel: &str) -> String {
    let path = ws.root.join(rel);
    let sc = outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(&path)).expect("sidecar");
    outl_actions::journal::render_page_md(&ws.workspace, sc.page_id)
}

#[test]
fn an_empty_attribute_keeps_its_key_line() {
    let json = r#"[
        {"title": "Draft Post", "children": [
            {"string": "published::", "uid": "d0", "children": []},
            {"string": "an outline item", "uid": "d1", "children": [
                {"string": "ai-context::\nkind:: note", "uid": "d2", "children": []}
            ]},
            {"string": "see ((d1)) above", "uid": "d3", "children": []}
        ]}
    ]"#;
    let ws = import_json(json);
    for text in [
        read(&ws.root.join("pages/draft-post.md")),
        projected(&ws, "pages/draft-post.md"),
    ] {
        assert!(text.contains("published::\n"), "page:\n{text}");
        assert!(text.contains("ai-context::\n"), "page:\n{text}");
        assert!(text.contains("kind:: note\n"), "page:\n{text}");
    }
}
