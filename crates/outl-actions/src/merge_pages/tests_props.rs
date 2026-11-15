//! The property decisions of `merge_pages`, one test per row of the
//! table in the module docs.

use super::tests::{merge, page, prop, put, ws};
use super::*;
use outl_core::property::PropValue;

#[test]
fn aliases_union_case_insensitively_with_the_targets_first() {
    let (mut w, hlc) = ws();
    let alpha = page(&mut w, &hlc, "alpha", "Alpha");
    let beta = page(&mut w, &hlc, "beta", "Beta");
    put(&mut w, &hlc, beta, "alias", "Bee, Shared");
    put(&mut w, &hlc, alpha, "alias", "shared, Ay");

    let report = merge(&mut w, &hlc);

    let alias = prop(&w, beta, "alias").unwrap();
    assert!(alias.starts_with("Bee, Shared, "), "target first: {alias}");
    assert_eq!(report.target.aliases[..3], ["Bee", "Shared", "Ay"]);
    let shared = report
        .target
        .aliases
        .iter()
        .filter(|a| a.eq_ignore_ascii_case("shared"))
        .count();
    assert_eq!(shared, 1);
}

#[test]
fn tags_union_case_insensitively_in_the_targets_shape() {
    let (mut w, hlc) = ws();
    let alpha = page(&mut w, &hlc, "alpha", "Alpha");
    let beta = page(&mut w, &hlc, "beta", "Beta");
    put(&mut w, &hlc, beta, "tags", "#red #Blue");
    put(&mut w, &hlc, alpha, "tags", "#blue, #green");

    let report = merge(&mut w, &hlc);

    assert_eq!(prop(&w, beta, "tags").unwrap(), "#red #Blue #green");
    assert!(report.properties_written.contains(&"tags".to_string()));
}

#[test]
fn comma_separated_tags_stay_comma_separated() {
    let (mut w, hlc) = ws();
    let alpha = page(&mut w, &hlc, "alpha", "Alpha");
    let beta = page(&mut w, &hlc, "beta", "Beta");
    put(&mut w, &hlc, beta, "tags", "red, blue");
    put(&mut w, &hlc, alpha, "tags", "green");

    merge(&mut w, &hlc);

    assert_eq!(prop(&w, beta, "tags").unwrap(), "red, blue, green");
}

#[test]
fn a_list_valued_tags_property_stays_a_list() {
    let (mut w, hlc) = ws();
    let alpha = page(&mut w, &hlc, "alpha", "Alpha");
    let beta = page(&mut w, &hlc, "beta", "Beta");
    let list = |items: &[&str]| {
        let tags = items.iter().map(|s| PropValue::Tag(s.to_string()));
        PropValue::List(tags.collect())
    };
    set_property(&mut w, &hlc, beta, "tags", Some(list(&["red"]))).unwrap();
    set_property(&mut w, &hlc, alpha, "tags", Some(list(&["Red", "green"]))).unwrap();

    merge(&mut w, &hlc);

    match w.tree().property(beta, "tags") {
        Some(PropValue::List(items)) => assert_eq!(items.len(), 2),
        other => panic!("expected a list, got {other:?}"),
    }
    assert_eq!(prop(&w, beta, "tags").unwrap(), "red green");
}

#[test]
fn tags_the_target_already_holds_write_nothing() {
    let (mut w, hlc) = ws();
    let alpha = page(&mut w, &hlc, "alpha", "Alpha");
    let beta = page(&mut w, &hlc, "beta", "Beta");
    put(&mut w, &hlc, beta, "tags", "#red #blue");
    put(&mut w, &hlc, alpha, "tags", "#BLUE");

    let report = merge(&mut w, &hlc);

    assert_eq!(prop(&w, beta, "tags").unwrap(), "#red #blue");
    assert!(!report.properties_written.contains(&"tags".to_string()));
}

#[test]
fn a_scalar_conflict_keeps_the_target_and_reports_the_dropped_value() {
    let (mut w, hlc) = ws();
    let alpha = page(&mut w, &hlc, "alpha", "Alpha");
    let beta = page(&mut w, &hlc, "beta", "Beta");
    put(&mut w, &hlc, alpha, "icon", "A");
    put(&mut w, &hlc, beta, "icon", "B");

    let report = merge(&mut w, &hlc);

    assert_eq!(prop(&w, beta, "icon").unwrap(), "B");
    assert_eq!(
        report.property_conflicts,
        vec![PropertyConflict {
            key: "icon".into(),
            kept: "B".into(),
            dropped: "A".into(),
        }]
    );
}

#[test]
fn an_absent_scalar_is_copied_and_an_equal_one_is_left_alone() {
    let (mut w, hlc) = ws();
    let alpha = page(&mut w, &hlc, "alpha", "Alpha");
    let beta = page(&mut w, &hlc, "beta", "Beta");
    put(&mut w, &hlc, alpha, "icon", "A");
    put(&mut w, &hlc, alpha, "owner", "sam");
    put(&mut w, &hlc, beta, "owner", "sam");

    let report = merge(&mut w, &hlc);

    assert_eq!(prop(&w, beta, "icon").unwrap(), "A");
    assert!(report.properties_written.contains(&"icon".to_string()));
    assert!(!report.properties_written.contains(&"owner".to_string()));
    assert!(report.property_conflicts.is_empty());
}

#[test]
fn the_sources_identity_keys_never_reach_the_target() {
    let (mut w, hlc) = ws();
    let alpha = page(&mut w, &hlc, "alpha", "Alpha");
    let beta = page(&mut w, &hlc, "beta", "Beta");
    put(&mut w, &hlc, alpha, "page-source", "/tmp/alpha.md");
    put(&mut w, &hlc, alpha, "page-frontmatter", "title: x");

    let report = merge(&mut w, &hlc);

    assert_eq!(prop(&w, beta, "page-slug").unwrap(), "beta");
    assert_eq!(prop(&w, beta, "title").unwrap(), "Beta");
    assert_eq!(prop(&w, beta, "page-source"), None);
    assert_eq!(prop(&w, beta, "page-frontmatter"), None);
    assert!(report.property_conflicts.is_empty());
}

#[test]
fn a_template_name_is_reported_never_copied() {
    let (mut w, hlc) = ws();
    let alpha = page(&mut w, &hlc, "alpha", "Alpha");
    let beta = page(&mut w, &hlc, "beta", "Beta");
    put(&mut w, &hlc, alpha, "template", "standup");
    put(&mut w, &hlc, alpha, "params", "who");

    let report = merge(&mut w, &hlc);

    assert_eq!(prop(&w, beta, "template"), None);
    assert_eq!(prop(&w, beta, "params"), None);
    assert_eq!(
        report.property_conflicts,
        vec![PropertyConflict {
            key: "template".into(),
            kept: String::new(),
            dropped: "standup".into(),
        }]
    );
}

#[test]
fn an_equal_template_name_is_no_conflict() {
    let (mut w, hlc) = ws();
    let alpha = page(&mut w, &hlc, "alpha", "Alpha");
    let beta = page(&mut w, &hlc, "beta", "Beta");
    put(&mut w, &hlc, alpha, "template", "standup");
    put(&mut w, &hlc, beta, "template", "standup");

    let report = merge(&mut w, &hlc);

    assert!(report.property_conflicts.is_empty());
}

#[test]
fn a_name_holding_a_comma_is_written_in_brackets_and_reads_back_whole() {
    let (mut w, hlc) = ws();
    page(&mut w, &hlc, "alpha", "Alpha, the first");
    let beta = page(&mut w, &hlc, "beta", "Beta");
    put(&mut w, &hlc, beta, "alias", "Bee");

    let report = merge(&mut w, &hlc);

    assert_eq!(
        prop(&w, beta, "alias").unwrap(),
        "Bee, alpha, [[Alpha, the first]]"
    );
    assert_eq!(
        report.target.aliases,
        vec!["Bee", "alpha", "Alpha, the first"]
    );
}
