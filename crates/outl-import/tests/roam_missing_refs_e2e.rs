//! End-to-end: a block ref or embed whose uid is not in the export
//! keeps the text the source wrote instead of an `unresolved:` marker.

mod common;

use common::{import_with, read};
use outl_import::adapters::RoamAdapter;
use std::fs;

#[test]
fn refs_to_uids_the_export_lacks_keep_their_source_text() {
    let json = r#"[
        {"title": "Target", "children": [
            {"string": "here", "uid": "tgt-uid", "children": []}
        ]},
        {"title": "Notes", "children": [
            {"string": "see ((gone-uid1)) later", "uid": "n1", "children": []},
            {"string": "as [noted](((gone-uid2)))", "uid": "n2", "children": []},
            {"string": "Done!((gone-uid3))", "uid": "n3", "children": []},
            {"string": "{{[[embed]]: ((gone-uid4))}}", "uid": "n4", "children": []},
            {"string": "both ((tgt-uid)) and ((gone-uid5))", "uid": "n5", "children": []}
        ]}
    ]"#;
    let src_dir = tempfile::tempdir().expect("src tempdir");
    let src = src_dir.path().join("backup.json");
    fs::write(&src, json).expect("write fixture");
    let (ws, report) = import_with(&RoamAdapter, &src);
    let page = read(&ws.root.join("pages/notes.md"));

    assert!(!page.contains("unresolved:"), "page:\n{page}");
    assert!(!page.contains("outl-import"), "page:\n{page}");
    assert!(
        page.contains("- see ((gone-uid1)) later\n"),
        "page:\n{page}"
    );
    assert!(
        page.contains("- as [noted](((gone-uid2)))\n"),
        "page:\n{page}"
    );
    // No separator is inserted: `!((gone-uid3))` is not an outl embed.
    assert!(page.contains("- Done!((gone-uid3))\n"), "page:\n{page}");
    // The embed keeps outl's `!` form and the source uid.
    assert!(page.contains("- !((gone-uid4))\n"), "page:\n{page}");
    // A resolvable ref next to a missing one still resolves.
    assert!(page.contains("- both ((blk-"), "page:\n{page}");
    assert!(page.contains(") and ((gone-uid5))\n"), "page:\n{page}");

    assert_eq!(report.refs_unresolved, 5);
    assert_eq!(report.refs_resolved, 1);
}
