//! What the quick switcher offers before you type anything.
//!
//! Its own file because `commands/page.rs` is at the size ceiling, and
//! because this is one question: the order of a mixed list of pages.
use outl_actions::{PageKind, PageMeta};
use outl_tauri_shared::commands::page::browse_order;

fn page(slug: &str, title: &str, kind: PageKind) -> PageMeta {
    PageMeta {
        id: slug.to_string(),
        slug: slug.to_string(),
        title: title.to_string(),
        kind,
        icon: None,
        pinned: false,
        page_type: None,
    }
}

/// The switcher's first screen is the one a user sees most, and it
/// used to open on the oldest journals in the workspace.
#[test]
fn browsing_puts_the_latest_journals_first_then_pages_a_to_z() {
    let got = browse_order(vec![
        page("1on1", "1on1", PageKind::Page),
        page("2022-02-14", "2022-02-14", PageKind::Journal),
        page("2026-10-24", "2026-10-24", PageKind::Journal),
        page("archive", "archive", PageKind::Page),
        page("2026-10-23", "2026-10-23", PageKind::Journal),
    ]);
    let order: Vec<&str> = got.iter().map(|p| p.slug.as_str()).collect();
    assert_eq!(
        order,
        ["2026-10-24", "2026-10-23", "2022-02-14", "1on1", "archive"]
    );
}

/// Journals and named pages need separate arms: one descending
/// comparison for both sorts every letter above every ISO date
/// (`a` is 0x61, `2` is 0x32), which is the shape of the bug the
/// task list had.
#[test]
fn named_pages_are_not_reversed_by_the_journal_arm() {
    let got = browse_order(vec![
        page("zebra", "zebra", PageKind::Page),
        page("apple", "apple", PageKind::Page),
    ]);
    let order: Vec<&str> = got.iter().map(|p| p.slug.as_str()).collect();
    assert_eq!(order, ["apple", "zebra"]);
}

/// A byte compare is not the alphabet. Every capitalised title sorts
/// above every lowercase one under `str::cmp`, so a graph that mixes
/// `Buser/Team` with `1on1` reads as two separate lists.
#[test]
fn a_to_z_ignores_case() {
    let got = browse_order(vec![
        page("zebra", "apple", PageKind::Page),
        page("alpha", "Banana", PageKind::Page),
        page("beta", "Zebra", PageKind::Page),
    ]);
    let order: Vec<&str> = got.iter().map(|p| p.title.as_str()).collect();
    assert_eq!(order, ["apple", "Banana", "Zebra"]);
}
