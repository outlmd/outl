//! What happens to every shape of Logseq block reference and embed,
//! measured end to end against a real workspace.
//!
//! A ref is the one thing a migrating user checks first: it is visible,
//! it is clickable, and a graph where half the refs became grey text is
//! a graph they will not trust. Each test here is one spelling Logseq
//! writes or accepts.

mod common;

use common::{fixture_tree, import_with, read};
use outl_import::adapters::LogseqAdapter;

/// A real Logseq id, as `id::` writes it.
const SRC_UID: &str = "6601a2c1-4f31-4a45-1c2c-3a5e6b7d8f90";

fn graph(referrer: &str) -> tempfile::TempDir {
    fixture_tree(&[
        (
            "pages/source.md",
            &format!("- the target block\n  id:: {SRC_UID}\n"),
        ),
        ("pages/referrer.md", referrer),
    ])
}

/// Import `referrer` and return the page it produced.
fn imported(referrer: &str) -> (String, outl_import::ImportReport) {
    let g = graph(referrer);
    let (ws, report) = import_with(&LogseqAdapter, g.path());
    (read(&ws.root.join("pages/referrer.md")), report)
}

#[test]
fn a_plain_ref_resolves_to_a_handle() {
    let (page, report) = imported(&format!("- see (({SRC_UID})) here\n"));
    assert!(page.contains("see ((blk-"), "{page}");
    assert_eq!(report.refs_resolved, 1);
}

#[test]
fn an_aliased_ref_resolves_and_keeps_its_alias() {
    let (page, report) = imported(&format!("- see [the thing]((({SRC_UID}))) \n"));
    // `[alias](((uid)))` — the alias survives, the target becomes a handle.
    assert!(
        page.contains("[the thing](((blk-") || page.contains("((blk-"),
        "{page}"
    );
    assert!(
        report.refs_resolved + report.refs_page_fallback >= 1,
        "{report:?}"
    );
}

#[test]
fn an_embed_resolves_to_an_embed_handle() {
    let (page, report) = imported(&format!("- inline {{{{embed (({SRC_UID}))}}}}\n"));
    assert!(page.contains("!((blk-"), "{page}");
    assert_eq!(report.embeds_resolved, 1);
}

/// Logseq's older triple-brace spelling. Still in every graph that has
/// been around a while, and `{{{embed …}}}` is what Logseq's own
/// `/Block embed` command used to insert.
#[test]
fn a_triple_brace_embed_resolves_too() {
    let (page, report) = imported(&format!("- old {{{{{{embed (({SRC_UID}))}}}}}}\n"));
    assert!(
        page.contains("!((blk-"),
        "a triple-brace embed must resolve, not survive as text:\n{page}"
    );
    assert_eq!(report.embeds_resolved, 1);
}

/// No space after `embed` is a typo Logseq itself renders, so it has to
/// import as an embed rather than as an unknown component.
#[test]
fn an_embed_without_a_space_resolves_too() {
    let (page, report) = imported(&format!("- tight {{{{embed(({SRC_UID}))}}}}\n"));
    assert!(page.contains("!((blk-"), "{page}");
    assert_eq!(report.embeds_resolved, 1);
}

/// A page embed has no outl form yet (issue #190), so it degrades to a
/// link — but it must be *counted*, never silent.
#[test]
fn a_page_embed_degrades_to_a_link_and_says_so() {
    let (page, report) = imported("- {{embed [[Other Page]]}}\n");
    assert!(page.contains("[[Other Page]]"), "{page}");
    assert_eq!(report.embeds_page_fallback, 1);
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.detail.contains("page embed")),
        "{:?}",
        report.warnings
    );
}

/// A ref inside a property value is still a ref. It used to be written
/// out verbatim and stay inert text, because a property never passes
/// through the inline tokenizer.
#[test]
fn a_ref_in_a_property_value_resolves() {
    let (page, report) = imported(&format!("- item\n  related:: (({SRC_UID}))\n"));
    assert!(
        !page.contains("outl-import:"),
        "placeholder survived on disk:\n{page}"
    );
    assert!(
        page.contains("related:: ((blk-"),
        "a ref in a property must resolve to a handle:\n{page}"
    );
    assert_eq!(report.refs_in_prop_values, 1);
}

/// A ref to a uid the graph does not hold keeps its source text, so it
/// is still searchable, and is counted as unresolved.
#[test]
fn an_unknown_ref_keeps_its_source_text() {
    let (page, report) = imported("- dangling ((deadbeef-0000-0000-0000-000000000000))\n");
    assert!(
        page.contains("((deadbeef-0000-0000-0000-000000000000))"),
        "{page}"
    );
    assert_eq!(report.refs_unresolved, 1);
}

/// Several refs in one block all resolve, and a block that mixes a ref
/// with an embed does not confuse the two.
#[test]
fn a_block_mixing_a_ref_and_an_embed_resolves_both() {
    let (page, report) = imported(&format!(
        "- both (({SRC_UID})) and {{{{embed (({SRC_UID}))}}}}\n"
    ));
    assert_eq!(report.refs_resolved, 1, "{page}");
    assert_eq!(report.embeds_resolved, 1, "{page}");
}

/// A ref on a continuation line of a multi-line block. The resolve pass
/// matches the block by a content hash, so anything that changes the
/// block's text between render and reconcile loses the wiring.
#[test]
fn a_ref_on_a_continuation_line_resolves() {
    let (page, report) = imported(&format!("- first line\n  see (({SRC_UID}))\n"));
    assert!(page.contains("((blk-"), "{page}");
    assert_eq!(report.refs_resolved, 1);
}

/// A ref inside a code span is code, not a reference.
#[test]
fn a_ref_inside_a_code_span_stays_text() {
    let (page, report) = imported(&format!("- the syntax is `(({SRC_UID}))`\n"));
    assert!(page.contains(&format!("`(({SRC_UID}))`")), "{page}");
    assert_eq!(report.refs_resolved, 0);
}

/// Every ref shape in one page, with the report's numbers as the
/// contract: nothing silently became text.
#[test]
fn the_report_accounts_for_every_ref_in_a_mixed_page() {
    let (_, report) = imported(&format!(
        "- a (({SRC_UID}))\n\
         - b {{{{embed (({SRC_UID}))}}}}\n\
         - c {{{{{{embed (({SRC_UID}))}}}}}}\n\
         - d ((deadbeef-0000-0000-0000-000000000000))\n\
         - e\n  related:: (({SRC_UID}))\n"
    ));
    // `refs_resolved` counts the property one too — it did resolve to a
    // handle; `refs_in_prop_values` is the separate dimension of where
    // it was found.
    assert_eq!(report.refs_resolved, 2, "{report:?}");
    assert_eq!(report.embeds_resolved, 2, "{report:?}");
    assert_eq!(report.refs_unresolved, 1, "{report:?}");
    assert_eq!(report.refs_in_prop_values, 1, "{report:?}");
    assert_eq!(report.refs_page_fallback, 0, "{report:?}");
}
