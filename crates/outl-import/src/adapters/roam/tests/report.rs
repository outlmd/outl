//! Graph-level Roam tests: whole-backup fixtures and the import report's
//! counters and warnings.

use super::*;

/// Write a Roam backup fixture and parse it into IR + report.
fn parse_backup(json: &str) -> (crate::ir::ImportGraph, ImportReport, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("backup.json");
    std::fs::write(&src, json).expect("write fixture");
    let mut report = ImportReport::new("roam");
    let graph = RoamAdapter.parse(&src, &mut report).expect("parse");
    (graph, report, dir)
}

/// A `{{[[DONE]]}}` buried mid-text is kept exactly as written, but it
/// is not the block's task state: the block does not answer `outl
/// query --kind=task`. With 10k+ DONEs in a real graph, that has to be
/// counted, not silently dropped.
#[test]
fn midtext_task_marker_is_counted_as_literal() {
    let (b, report) = parse_one_block("shipped it {{[[DONE]]}} yesterday");
    assert_eq!(b.task, None, "mid-text marker must not set the task state");
    assert_eq!(
        inline_tokens(&b),
        &[Inline::Text("shipped it {{[[DONE]]}} yesterday".into())]
    );
    assert_eq!(report.tasks_midtext_literal, 1);
    // A head marker is a real task — it must NOT inflate the counter.
    let (b, report) = parse_one_block("{{[[DONE]]}} laundry");
    assert_eq!(b.task, Some(TaskState::Done));
    assert_eq!(report.tasks_midtext_literal, 0);
}

/// One aggregate warning for the whole import, never one per hit —
/// a 10k-DONE graph would otherwise bury every other finding.
#[test]
fn midtext_task_markers_emit_one_aggregate_warning() {
    let (_, report, _dir) = parse_backup(
        r#"[
            {"title": "Log", "children": [
                {"string": "a {{[[DONE]]}} b", "uid": "l1", "children": []},
                {"string": "c {{[[TODO]]}} d", "uid": "l2", "children": []},
                {"string": "e {{DONE}} f", "uid": "l3", "children": []}
            ]}
        ]"#,
    );
    assert_eq!(report.tasks_midtext_literal, 3);
    let hits: Vec<&ImportWarning> = report
        .warnings
        .iter()
        .filter(|w| w.detail.contains("mid-block TODO/DONE"))
        .collect();
    assert_eq!(hits.len(), 1, "one aggregate warning, not one per marker");
    assert!(
        hits[0]
            .detail
            .contains("3 mid-block TODO/DONE markers kept verbatim as text")
            && hits[0].detail.contains("task state not preserved"),
        "warning must name the count and the loss: {}",
        hits[0].detail
    );
}

/// An empty page title used to `continue` in silence, taking the whole
/// child subtree with it.
#[test]
fn empty_title_page_is_reported_as_skipped_with_its_blocks() {
    let (graph, report, _dir) = parse_backup(
        r#"[
            {"title": "   ", "children": [
                {"string": "orphaned", "uid": "e1", "children": [
                    {"string": "and its child", "uid": "e2", "children": []}
                ]}
            ]},
            {"title": "Kept", "children": [{"string": "y", "uid": "k1", "children": []}]}
        ]"#,
    );
    assert_eq!(
        graph.pages.len(),
        1,
        "the empty-title page is still dropped"
    );
    assert_eq!(report.skipped.len(), 1);
    let skipped = &report.skipped[0];
    assert!(skipped.path.contains("page[0]"), "path: {}", skipped.path);
    assert!(
        skipped.reason.contains("empty"),
        "reason: {}",
        skipped.reason
    );
    assert_eq!(skipped.blocks_dropped, 2, "the whole subtree is counted");
}

/// The source counts are the reconciliation's denominator — counted off
/// the JSON, so a block lost in the parse can't hide from both sides.
#[test]
fn source_counts_come_from_the_json_including_skipped_pages() {
    let (_, report, _dir) = parse_backup(
        r#"[
            {"title": "", "children": [{"string": "gone", "uid": "g1", "children": []}]},
            {"title": "A", "children": [
                {"string": "one", "uid": "a1", "children": [
                    {"string": "two", "uid": "a2", "children": []}
                ]}
            ]}
        ]"#,
    );
    assert_eq!(report.source_pages, 2);
    assert_eq!(report.source_blocks, 3);
}

/// A leading pure-attribute block legitimately stops being a block.
/// Counting it separately is what keeps the reconciliation from
/// false-alarming on a promotion.
#[test]
fn promoted_attribute_blocks_are_counted_as_lifted() {
    let (_, report, _dir) = parse_backup(
        r#"[
            {"title": "@Tonico", "children": [
                {"string": "icon:: 👤", "uid": "a1", "children": []},
                {"string": "page-type:: contact", "uid": "a2", "children": []},
                {"string": "actual note", "uid": "n1", "children": []}
            ]}
        ]"#,
    );
    assert_eq!(report.source_blocks, 3);
    assert_eq!(report.blocks_lifted_to_props, 2);
}
