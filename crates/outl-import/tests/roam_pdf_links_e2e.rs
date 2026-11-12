//! End-to-end: a Roam `{{[[pdf]]: url}}` component lands as outl's file
//! link `[name](url)`, and a local PDF is copied into `assets/`.

mod common;

use common::{import_with, read};
use outl_import::adapters::RoamAdapter;
use std::fs;

#[test]
fn pdf_components_become_file_links() {
    let src_dir = tempfile::tempdir().expect("src tempdir");
    fs::write(src_dir.path().join("local.pdf"), b"%PDF-1.4 synthetic").expect("write pdf");
    let json = r#"[
        {"title": "Reading", "children": [
            {"string": "{{[[pdf]]: https://files.example.com/o/a%2Fb%2Fpaper.pdf?alt=media}}", "uid": "r1", "children": []},
            {"string": "notes {{pdf: local.pdf}} here", "uid": "r2", "children": []},
            {"string": "`{{[[pdf]]: https://files.example.com/x.pdf}}`", "uid": "r3", "children": []}
        ]}
    ]"#;
    let src = src_dir.path().join("backup.json");
    fs::write(&src, json).expect("write fixture");
    let (ws, report) = import_with(&RoamAdapter, &src);
    let page = read(&ws.root.join("pages/reading.md"));

    assert!(
        page.contains("- [paper.pdf](https://files.example.com/o/a%2Fb%2Fpaper.pdf?alt=media)\n"),
        "page:\n{page}"
    );
    assert!(
        page.contains("- notes [local.pdf](assets/"),
        "page:\n{page}"
    );
    assert!(!page.contains("{{pdf:"), "page:\n{page}");
    // Inside a code span the component is code, not a PDF.
    assert!(
        page.contains("- `{{[[pdf]]: https://files.example.com/x.pdf}}`\n"),
        "page:\n{page}"
    );
    assert_eq!(report.pdfs_linked, 2);
    assert_eq!(report.assets_copied, 1);
    assert_eq!(report.components_verbatim.get("pdf"), None);
}
