//! Roam fixture shared by the Roam end-to-end test binaries.

use super::{import_with, TestWs};
use outl_import::adapters::RoamAdapter;
use outl_import::ImportReport;
use std::fs;

/// Three pages: a ref/embed target, a page that points at it every way
/// Roam can, and a journal. 8 blocks in total.
pub const FIXTURE: &str = r#"[
    {
        "title": "Source",
        "children": [
            {"string": "the original decision", "uid": "src-uid", "children": [
                {"string": "supporting detail", "uid": "child-uid", "children": [], "open": false}
            ]}
        ]
    },
    {
        "title": "Referrer",
        "children": [
            {"string": "see ((src-uid)) please", "uid": "r1", "children": []},
            {"string": "context: {{[[embed]]: ((src-uid))}}", "uid": "r2", "children": []},
            {"string": "dangling ((nope-uid)) ref", "uid": "r3", "children": []},
            {"string": "Done!((src-uid))", "uid": "r4", "children": []},
            {"string": "negociar: {{embed: ((src-uid))}}\ncollapsed:: true\nid:: 6908fc01-aaaa", "uid": "r5", "children": []}
        ]
    },
    {
        "title": "May 25th, 2026",
        "children": [
            {"string": "{{[[TODO]]}} review #[[My Project]] __soon__", "uid": "j1", "children": []}
        ]
    }
]"#;

pub fn import_fixture(json: &str) -> (TestWs, ImportReport) {
    let src_dir = tempfile::tempdir().expect("src tempdir");
    let src = src_dir.path().join("backup.json");
    fs::write(&src, json).expect("write fixture");
    import_with(&RoamAdapter, &src)
}
