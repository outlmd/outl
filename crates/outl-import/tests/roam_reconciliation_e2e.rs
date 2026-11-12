//! End-to-end: the Roam import report's reconciliation, the
//! source-vs-emitted balance and the landing measurement taken off the
//! sidecars, over a real workspace.

mod common;

use common::open_test_ws;
use common::roam::{import_fixture, FIXTURE};
use outl_import::adapters::RoamAdapter;
use outl_import::{dry_run, ImportOptions};
use std::fs;

/// The report's own counters only prove what the pipeline *knows* it
/// emitted. The reconciliation adds the denominator, taken straight off
/// the source JSON, so a block lost anywhere in between shows up.
#[test]
fn reconciliation_balances_on_a_clean_import() {
    let (_ws, report) = import_fixture(FIXTURE);
    let r = report.reconciliation.expect("roam reports source counts");

    // 3 source pages → 3 files, nothing merged or skipped.
    assert_eq!(r.source_pages, 3);
    assert_eq!(r.emitted_pages, 3);
    assert_eq!(r.pages_skipped, 0);
    assert_eq!(r.pages_merged, 0);
    // 8 source blocks: 2 on Source, 5 on Referrer, 1 on the journal.
    assert_eq!(r.source_blocks, 8);
    assert_eq!(r.emitted_blocks, 8);
    assert_eq!(r.blocks_unaccounted, 0);
    assert!(r.balanced, "clean import must reconcile: {r:?}");
}

/// Every legitimate reducer (page skipped, block promoted to a page
/// property) is subtracted by name, so the books still close — the
/// unaccounted counters stay reserved for real silent loss.
#[test]
fn reconciliation_accounts_for_skipped_pages_and_lifted_props() {
    let json = r#"[
        {"title": "  ", "children": [
            {"string": "dropped", "uid": "d1", "children": [
                {"string": "dropped child", "uid": "d2", "children": []}
            ]}
        ]},
        {"title": "@Tonico", "children": [
            {"string": "icon:: 👤", "uid": "a1", "children": []},
            {"string": "the note", "uid": "n1", "children": []}
        ]},
        {"title": "May 25th, 2026", "children": [{"string": "x", "uid": "j1", "children": []}]},
        {"title": "2026-05-25", "children": [{"string": "y", "uid": "j2", "children": []}]}
    ]"#;
    let (_ws, report) = import_fixture(json);
    let r = report.reconciliation.expect("reconciliation present");

    assert_eq!(r.source_pages, 4);
    assert_eq!(r.emitted_pages, 2, "one skipped, two journals merged");
    assert_eq!(r.pages_skipped, 1);
    assert_eq!(r.pages_merged, 1);
    assert_eq!(r.pages_unaccounted, 0);

    assert_eq!(r.source_blocks, 6);
    assert_eq!(r.emitted_blocks, 3, "2 dropped with the page, 1 lifted");
    assert_eq!(r.blocks_skipped, 2);
    assert_eq!(r.blocks_lifted_to_props, 1);
    assert_eq!(r.blocks_unaccounted, 0);
    assert!(r.balanced, "explained losses must still balance: {r:?}");

    // The skipped page is named in the report, not silently gone.
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].blocks_dropped, 2);
}

/// Every counter except the landing ones is bumped in `render`, before
/// a byte hits disk — they prove the parser and the renderer agree, not
/// that the content is in a workspace. The landing numbers are summed
/// off the sidecars `reconcile_md` stamps from the materialized tree, so
/// they are the only ones that answer "did it actually arrive".
#[test]
fn the_report_confirms_the_blocks_reached_the_op_log() {
    let (_ws, report) = import_fixture(FIXTURE);
    assert!(report.landing_measured, "a real import measures landing");
    assert_eq!(report.landed_pages, 3);
    assert_eq!(
        report.landed_blocks, report.blocks,
        "every emitted block must be confirmed in the op log"
    );

    let r = report.reconciliation.expect("reconciliation present");
    assert!(r.landing_measured);
    assert_eq!(r.landed_blocks, 8);
    assert_eq!(r.blocks_not_landed, 0);
    assert!(r.balanced, "{r:?}");
}

/// A dry-run answers "what would I lose" — the reconciliation has to be
/// part of that answer, not only of the real run. But it writes nothing,
/// so it must say the landing was **not** measured rather than imply the
/// content is somewhere.
#[test]
fn dry_run_reports_the_reconciliation_too() {
    let src_dir = tempfile::tempdir().expect("tempdir");
    let src = src_dir.path().join("backup.json");
    fs::write(&src, FIXTURE).expect("write fixture");

    let report = dry_run(&RoamAdapter, &src, &ImportOptions::default()).expect("dry run");
    let r = report.reconciliation.expect("reconciliation present");
    assert_eq!(r.source_blocks, 8);
    assert!(r.balanced, "{r:?}");
    assert!(
        !r.landing_measured,
        "a dry-run writes nothing — it cannot claim the blocks landed"
    );
    assert_eq!(r.landed_blocks, 0);
    assert_eq!(r.blocks_not_landed, 0);
}

/// The landing check has to notice a page that never reached the tree,
/// not repeat the renderer's optimism. Blocking one page's sidecar (a
/// directory sits where the file must go) makes `reconcile_md` fail for
/// exactly that page — the same shape as the write/reconcile failures
/// the real pipeline can hit at block 40k of 66k, where the in-memory
/// counters happily report success.
#[test]
fn a_page_that_never_reconciled_breaks_the_balance() {
    let mut ws = open_test_ws();
    let src_dir = tempfile::tempdir().expect("tempdir");
    let src = src_dir.path().join("backup.json");
    fs::write(&src, FIXTURE).expect("write fixture");
    // `Source` (2 blocks) can never get a sidecar written or read.
    fs::create_dir_all(ws.root.join("pages/source.outl")).expect("block the sidecar");

    let report = common::import_into(&RoamAdapter, &src, &mut ws, &ImportOptions::default());
    let r = report.reconciliation.expect("reconciliation present");

    assert_eq!(r.emitted_blocks, 8, "the renderer still emitted all 8");
    assert_eq!(r.blocks_unaccounted, 0, "parse → render still adds up");
    assert_eq!(
        r.blocks_not_landed, 2,
        "the two Source blocks are not confirmed anywhere: {r:?}"
    );
    assert!(
        !r.balanced,
        "content that never reached the op log must not report as balanced: {r:?}"
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.detail.contains("NOT confirmed in the op log")),
        "the failure is named per page: {:?}",
        report.warnings
    );
}
