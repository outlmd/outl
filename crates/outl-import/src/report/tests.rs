//! Tests for the report's accounting: what closes the books and what
//! has to break the balance.

use super::*;

/// A free-markdown adapter reconciles pages and says, rather than
/// implies, that it has no block denominator. Counting its blocks
/// as `0` would read as "every block vanished".
#[test]
fn an_unmeasured_block_count_accuses_nobody() {
    let mut r = ImportReport::new("obsidian");
    r.count_source_pages(2);
    r.pages = 2;
    r.blocks = 40;
    r.finalize();
    let rec = r.reconciliation.expect("page counts present");
    assert!(!rec.source_blocks_measured);
    assert_eq!(rec.blocks_unaccounted, 0);
    assert!(rec.balanced, "{rec:?}");
}

/// …and the page half still has to close.
#[test]
fn an_unmeasured_block_count_still_balances_pages() {
    let mut r = ImportReport::new("obsidian");
    r.count_source_pages(3);
    r.pages = 2;
    r.finalize();
    let rec = r.reconciliation.expect("page counts present");
    assert_eq!(rec.pages_unaccounted, 1);
    assert!(!rec.balanced, "{rec:?}");
}

/// A lifted template is a page the import added, not a page that
/// appeared out of nowhere.
#[test]
fn added_template_pages_keep_the_books_closed() {
    let mut r = ImportReport::new("logseq");
    r.count_source(1, 3);
    r.pages = 2; // the source page + one lifted template page
    r.blocks = 5; // 3 source blocks + 2 copied into the template
    r.pages_added_as_templates = 1;
    r.blocks_copied_into_templates = 2;
    r.finalize();
    let rec = r.reconciliation.expect("source counts present");
    assert_eq!(rec.pages_unaccounted, 0);
    assert_eq!(rec.blocks_unaccounted, 0);
    assert!(rec.balanced, "{rec:?}");
}

/// An adapter that reports no source counts gets no reconciliation —
/// a fabricated "0/0, balanced" would be worse than saying nothing.
#[test]
fn finalize_is_a_no_op_without_source_counts() {
    let mut r = ImportReport::new("logseq");
    r.pages = 3;
    r.blocks = 40;
    r.finalize();
    assert!(r.reconciliation.is_none());
}

/// Blocks that vanished with no reported reason are exactly what the
/// reconciliation exists to surface.
#[test]
fn unexplained_loss_breaks_the_balance() {
    let mut r = ImportReport::new("roam");
    r.count_source(2, 10);
    r.pages = 2;
    r.blocks = 7; // three blocks went missing on the way out
    r.finalize();
    let rec = r.reconciliation.expect("source counts present");
    assert_eq!(rec.blocks_unaccounted, 3);
    assert_eq!(rec.pages_unaccounted, 0);
    assert!(!rec.balanced);
}

/// Named reducers close the books instead of raising a false alarm.
#[test]
fn named_reducers_keep_the_books_closed() {
    let mut r = ImportReport::new("roam");
    r.count_source(4, 10);
    r.pages = 1;
    r.journals = 1;
    r.pages_merged = 1;
    r.skip("backup.json#page[0]", "page title is empty", 3);
    r.blocks = 6;
    r.blocks_lifted_to_props = 1;
    r.finalize();
    let rec = r.reconciliation.expect("source counts present");
    assert_eq!(rec.pages_skipped, 1);
    assert_eq!(rec.blocks_skipped, 3);
    assert_eq!(rec.blocks_unaccounted, 0);
    assert!(rec.balanced, "{rec:?}");
}

/// Parse → render adding up says nothing about the write. Blocks the
/// renderer produced that never reached the op log have to break the
/// balance too, or `balanced: true` keeps promising something it
/// never measured.
#[test]
fn blocks_that_never_landed_break_the_balance() {
    let mut r = ImportReport::new("roam");
    r.count_source(1, 10);
    r.pages = 1;
    r.blocks = 10; // the renderer emitted everything…
    r.landing_measured = true;
    r.landed_blocks = 7; // …but only 7 are in the op log
    r.finalize();
    let rec = r.reconciliation.expect("source counts present");
    assert_eq!(rec.blocks_unaccounted, 0, "parse → render is clean");
    assert_eq!(rec.blocks_not_landed, 3);
    assert!(!rec.balanced, "{rec:?}");
}

/// A dry-run writes nothing, so it must report the landing as
/// unmeasured instead of as zero-loss — and its balance stays about
/// what it can actually check.
#[test]
fn an_unmeasured_landing_never_invents_a_gap() {
    let mut r = ImportReport::new("roam");
    r.count_source(1, 10);
    r.pages = 1;
    r.blocks = 10;
    r.finalize();
    let rec = r.reconciliation.expect("source counts present");
    assert!(!rec.landing_measured);
    assert_eq!(rec.landed_blocks, 0);
    assert_eq!(rec.blocks_not_landed, 0, "no measurement, no accusation");
    assert!(rec.balanced, "{rec:?}");
}
