//! End-to-end: a Roam `{{[[table]]}}` block lands as one outl pipe-table
//! block, and the reconciliation still closes.

mod common;

use common::read;
use common::roam::import_fixture;

#[test]
fn a_roam_table_lands_as_one_pipe_table_block() {
    let json = r#"[
        {"title": "Target", "children": [
            {"string": "the source", "uid": "tgt0", "children": []}
        ]},
        {"title": "Stock", "children": [
            {"string": "{{[[table]]}}", "uid": "tb", "children": [
                {"string": "Item", "uid": "r1", "children": [
                    {"string": "Qty", "uid": "r1b", "children": []}
                ]},
                {"string": "[[Apples]]", "uid": "r2", "children": [
                    {"string": "see ((tgt0))", "uid": "r2b", "children": []}
                ]},
                {"string": "pears", "uid": "r3", "children": [
                    {"string": "1|2", "uid": "r3b", "children": []}
                ]}
            ]},
            {"string": "{{[[table]]}}", "uid": "empty", "children": []}
        ]}
    ]"#;
    let (ws, report) = import_fixture(json);
    let page = read(&ws.root.join("pages/stock.md"));

    assert!(
        page.contains("- | Item | Qty |\n  | --- | --- |\n"),
        "page:\n{page}"
    );
    assert!(
        page.contains("  | [[Apples]] | see ((blk-"),
        "page:\n{page}"
    );
    assert!(page.contains("  | pears | 1\\|2 |\n"), "page:\n{page}");
    // A table with no rows is not a table yet; its marker stays.
    assert!(page.contains("- {{[[table]]}}\n"), "page:\n{page}");
    assert_eq!(report.tables_converted, 1);
    assert_eq!(report.blocks_folded_into_tables, 6);
    assert_eq!(report.refs_resolved, 1);
    let r = report
        .reconciliation
        .as_ref()
        .expect("roam reports source counts");
    assert_eq!(r.blocks_folded_into_tables, 6);
    assert_eq!(r.blocks_unaccounted, 0);
    assert!(r.balanced, "reconciliation: {r:?}");
}
