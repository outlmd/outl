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

/// The refactor to `human_summary` moved 54 format strings by hand, and
/// "the text is unchanged" was asserted by nothing. These four pin the
/// branches a user acts on: the two loud banners, the warning cap, and
/// the arm that exists so `blocks: 0` never reads as total loss.
///
/// A free-markdown source renders text, not blocks, so its own counter
/// is `0` while the sidecars know the real number. The line has to say
/// where the number came from, or it reads as a different measurement.
#[test]
fn the_free_markdown_block_count_says_where_it_came_from() {
    let mut r = ImportReport::new("obsidian");
    r.pages = 2;
    r.blocks = 0;
    r.landed_blocks = 37;
    let text = r.human_summary();
    assert!(
        text.contains("  blocks:              37 (counted in the op log"),
        "{text}"
    );
    assert!(
        !text.contains("  blocks:              0\n"),
        "the bare zero is the line this arm exists to replace:\n{text}"
    );
}

/// Unexplained loss is the one thing the report exists to make
/// unmissable, so its banner and both numbers are asserted verbatim.
#[test]
fn unaccounted_content_is_unmissable_in_the_summary() {
    let mut r = ImportReport::new("roam");
    r.count_source(4, 10);
    r.pages = 2;
    r.blocks = 7;
    r.finalize();
    let text = r.human_summary();
    assert!(
        text.contains("  !!  UNACCOUNTED CONTENT — the import does not add up  !!"),
        "{text}"
    );
    assert!(
        text.contains("      2 page(s) and 3 block(s) from the source are unaccounted for."),
        "{text}"
    );
    assert!(text.contains("this is silent loss."), "{text}");
}

/// The other loud banner: render added up but the write did not, which
/// is a different accusation and must not borrow the first one's words.
#[test]
fn blocks_that_never_landed_are_named_in_the_summary() {
    let mut r = ImportReport::new("roam");
    r.count_source(1, 10);
    r.pages = 1;
    r.blocks = 10;
    r.landing_measured = true;
    r.landed_blocks = 7;
    r.finalize();
    let text = r.human_summary();
    assert!(
        text.contains("  !!  CONTENT NEVER REACHED THE OP LOG  !!"),
        "{text}"
    );
    assert!(
        text.contains("      3 of 10 emitted block(s) are not in the destination's op log."),
        "{text}"
    );
    assert!(
        !text.contains("UNACCOUNTED CONTENT"),
        "parse → render was clean; only the landing failed:\n{text}"
    );
}

/// A 66k-block import can emit thousands of warnings. The cap is only
/// honest while the line says how many it is hiding and where to read
/// the rest.
#[test]
fn a_capped_warning_list_says_how_many_are_hidden() {
    let mut r = ImportReport::new("logseq");
    for i in 0..print::WARNINGS_SHOWN + 5 {
        r.warn(&format!("pages/p{i}"), format!("warning {i}"));
    }
    let text = r.human_summary();
    let total = print::WARNINGS_SHOWN + 5;
    assert!(
        text.contains(&format!(
            "  warnings:            {total} ({} shown below)",
            print::WARNINGS_SHOWN
        )),
        "{text}"
    );
    assert_eq!(
        text.matches("    [pages/p").count(),
        print::WARNINGS_SHOWN,
        "exactly the cap is spelled out:\n{text}"
    );
    assert!(text.contains("    … 5 more warning(s) hidden"), "{text}");
    assert!(text.contains(&format!("read all {total}")), "{text}");
}

/// The summary is plain text the caller prints: it opens with an empty
/// line, names the source and carries the counters.
#[test]
fn the_human_summary_is_text_with_header_and_counters() {
    let mut r = ImportReport::new("obsidian");
    r.pages = 2;
    r.journals = 1;
    r.blocks = 40;
    let text = r.human_summary();
    assert!(text.starts_with("\nImport summary (obsidian):\n"), "{text}");
    assert!(text.contains("  pages:               2\n"), "{text}");
    assert!(text.contains("  journals:            1\n"), "{text}");
    assert!(text.contains("  blocks:              40\n"), "{text}");
    assert!(text.ends_with('\n'), "{text}");
}
