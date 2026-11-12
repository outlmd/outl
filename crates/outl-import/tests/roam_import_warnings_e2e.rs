//! End-to-end: the import's warnings say what actually happened, once.

mod common;

use common::read;
use common::roam::import_fixture;
use outl_import::ImportReport;

fn warnings_with<'a>(report: &'a ImportReport, needle: &str) -> Vec<&'a str> {
    report
        .warnings
        .iter()
        .filter(|w| w.detail.contains(needle))
        .map(|w| w.detail.as_str())
        .collect()
}

#[test]
fn missing_local_files_are_one_warning_naming_the_first_paths() {
    let blocks: Vec<String> = (0..7)
        .map(|i| format!(r#"{{"string": "![](gone-{i}.png)", "uid": "g{i}", "children": []}}"#))
        .collect();
    let json = format!(
        r#"[{{"title": "Pics", "children": [{}]}}]"#,
        blocks.join(",")
    );
    let (ws, report) = import_fixture(&json);

    assert_eq!(report.assets_missing, 7);
    let hits = warnings_with(&report, "local file(s) linked from the source");
    assert_eq!(hits.len(), 1, "one warning, not one per file: {hits:?}");
    assert!(hits[0].starts_with("7 local file(s)"), "{}", hits[0]);
    assert!(
        hits[0].contains("gone-0.png") && hits[0].contains("gone-4.png"),
        "{}",
        hits[0]
    );
    assert!(!hits[0].contains("gone-5.png"), "{}", hits[0]);
    assert!(hits[0].contains("and 2 more"), "{}", hits[0]);
    // The links themselves are untouched.
    let page = read(&ws.root.join("pages/pics.md"));
    assert!(page.contains("- ![](gone-6.png)\n"), "page:\n{page}");
}

/// A merged title with a comma is kept as a bracketed alias, the one
/// form an `alias::` list can carry it in, and it resolves to the page.
#[test]
fn a_merged_title_with_a_comma_is_kept_as_a_bracketed_alias() {
    let json = r#"[
        {"title": "Plan v2", "children": [
            {"string": "a", "uid": "a1", "children": []},
            {"string": "b", "uid": "a2", "children": []}
        ]},
        {"title": "Plan, v2", "children": [{"string": "c", "uid": "c1", "children": []}]},
        {"title": "PLAN V2", "children": []}
    ]"#;
    let (ws, report) = import_fixture(json);
    let page = read(&ws.root.join("pages/plan-v2.md"));
    assert!(page.contains("title:: Plan v2\n"), "page:\n{page}");
    assert!(
        page.contains("alias:: [[Plan, v2]], PLAN V2\n"),
        "page:\n{page}"
    );
    assert!(page.contains("- c\n"), "every block is kept:\n{page}");

    let idx = outl_md::index::WorkspaceIndex::build(&ws.root);
    assert_eq!(idx.by_title("Plan, v2").unwrap().slug, "plan-v2");
    assert_eq!(idx.by_title("PLAN V2").unwrap().slug, "plan-v2");

    for other in ["Plan, v2", "PLAN V2"] {
        let hits: Vec<_> = report.warnings.iter().filter(|w| w.page == other).collect();
        assert_eq!(hits.len(), 1, "{:?}", report.warnings);
        assert!(
            hits[0].detail.contains("merged and kept as an alias"),
            "{}",
            hits[0].detail
        );
    }
    assert_eq!(report.slug_collisions, 2);
}
