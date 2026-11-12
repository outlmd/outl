//! Edge cases of the DB adapter: malformed trees, rarer statuses,
//! multi-valued properties, untagged pages and the schema-version
//! warning. Built on the fixture builder in `tests.rs`.

use super::tests::*;
use crate::ir::{BlockContent, ImportBlock, PageName, TaskState};

/// Header every edge fixture shares: the Page / Task classes and the
/// status property with its closed values.
fn base() -> Fx {
    let mut fx = Fx::default();
    for (id, name) in [
        (PAGE_CLASS, "Page"),
        (TAG_CLASS, "Tag"),
        (PROPERTY_CLASS, "Property"),
        (TASK_CLASS, "Task"),
    ] {
        fx.ent(id, Some(&format!("logseq.class/{name}")), Some(name));
    }
    fx.ent(STATUS, Some("logseq.property/status"), Some("Status"));
    for (id, name) in [(11, "in-review"), (12, "canceled"), (13, "doing")] {
        fx.ent(
            id,
            Some(&format!("logseq.property/status.{name}")),
            Some(name),
        );
        fx.raw(id, "block/closed-value-property", &STATUS.to_string());
    }
    fx.ent(PROJECT, None, Some("Page"));
    fx.tag(PROJECT, PAGE_CLASS);
    fx
}

fn count(blocks: &[ImportBlock]) -> usize {
    blocks.iter().map(|b| 1 + count(&b.children)).sum()
}

#[test]
fn a_parent_cycle_keeps_every_block_and_says_so() {
    let mut fx = base();
    fx.block(41, PROJECT, 42, "a0", "one");
    fx.block(42, PROJECT, 41, "a0", "two");
    fx.block(43, PROJECT, PROJECT, "a0", "plain");
    let (g, report) = parse_text(&fx.finish());
    assert_eq!(count(outline(named(&g, "Page"))), 3);
    assert_eq!(report.source_blocks, 3);
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.detail.contains("parent cycle")),
        "{:?}",
        report.warnings
    );
}

#[test]
fn a_block_whose_parent_is_unknown_hangs_off_the_page_root() {
    let mut fx = base();
    fx.block(41, PROJECT, 999, "a0", "orphan");
    let (g, report) = parse_text(&fx.finish());
    let blocks = outline(named(&g, "Page"));
    assert_eq!(blocks.len(), 1);
    assert_eq!(text(&blocks[0]), "orphan");
    assert!(report
        .warnings
        .iter()
        .any(|w| w.detail.contains("parent outside their page")));
}

#[test]
fn in_review_and_canceled_keep_the_original_in_a_state_property() {
    let mut fx = base();
    fx.block(41, PROJECT, PROJECT, "a0", "reviewing");
    fx.raw(41, "logseq.property/status", "11");
    fx.block(42, PROJECT, PROJECT, "a1", "dropped");
    fx.raw(42, "logseq.property/status", "12");
    fx.block(43, PROJECT, PROJECT, "a2", "working");
    fx.raw(43, "logseq.property/status", "13");
    let (g, report) = parse_text(&fx.finish());
    let b = outline(named(&g, "Page"));
    assert_eq!(b[0].task, Some(TaskState::Doing));
    assert_eq!(prop(&b[0], "state"), Some("in-review"));
    assert_eq!(b[1].task, Some(TaskState::Canceled));
    assert_eq!(b[2].task, Some(TaskState::Doing));
    assert_eq!(prop(&b[2], "state"), None);
    assert_eq!(report.tasks.get("IN-REVIEW"), Some(&1));
    assert_eq!(report.tasks.get("CANCELED"), Some(&1));
    assert_eq!(report.tasks.get("DOING"), Some(&1));
}

#[test]
fn a_many_valued_property_is_joined_and_an_unrenderable_one_is_counted() {
    let mut fx = base();
    let prop_ident = "user.property/topics-q1";
    fx.ent(20, Some(prop_ident), Some("Topics"));
    fx.tag(20, PROPERTY_CLASS);
    fx.raw(20, "logseq.property/type", ":default");
    for (id, title) in [(21, "rust"), (22, "crdt")] {
        fx.ent(id, None, Some(title));
        fx.raw(id, "logseq.property/created-from-property", "20");
    }
    fx.block(41, PROJECT, PROJECT, "a0", "reading list");
    fx.raw(41, prop_ident, "21");
    fx.raw(41, prop_ident, "22");
    fx.raw(41, "user.property/odd-z9", "{:a 1}");
    let (g, report) = parse_text(&fx.finish());
    let b = &outline(named(&g, "Page"))[0];
    assert_eq!(prop(b, "topics"), Some("rust, crdt"));
    assert!(report
        .warnings
        .iter()
        .any(|w| w.detail.contains("value not renderable")));
}

#[test]
fn a_page_alias_becomes_an_alias_property() {
    let mut fx = base();
    fx.ent(60, None, Some("Other Name"));
    fx.tag(60, PAGE_CLASS);
    fx.raw(PROJECT, "block/alias", "60");
    let (g, _) = parse_text(&fx.finish());
    let p = named(&g, "Page");
    assert!(
        p.props
            .contains(&("alias".to_string(), "[[Other Name]]".to_string())),
        "{:?}",
        p.props
    );
}

#[test]
fn a_page_without_the_page_class_is_still_imported_when_blocks_point_at_it() {
    let mut fx = base();
    fx.ent(70, None, Some("Untagged"));
    fx.block(71, 70, 70, "a0", "kept");
    let (g, report) = parse_text(&fx.finish());
    assert_eq!(text(&outline(named(&g, "Untagged"))[0]), "kept");
    assert_eq!(report.source_blocks, 1);
}

#[test]
fn a_hidden_page_is_skipped_by_name() {
    let mut fx = base();
    fx.ent(72, None, Some("Secret"));
    fx.tag(72, PAGE_CLASS);
    fx.raw(72, "logseq.property/hide?", "true");
    let (g, report) = parse_text(&fx.finish());
    assert!(g
        .pages
        .iter()
        .all(|p| p.name != PageName::Named("Secret".into())));
    assert!(report.skipped.iter().any(|s| s.path.contains("Secret")));
}

#[test]
fn a_foreign_schema_major_warns_but_imports() {
    let edn = base().finish().replace(":major 65", ":major 70");
    let (g, report) = parse_text(&edn);
    assert_eq!(g.pages.len(), 1);
    assert!(report
        .warnings
        .iter()
        .any(|w| w.detail.contains("schema version 70.34")));
}

#[test]
fn an_empty_export_is_a_valid_empty_graph() {
    let (g, report) = parse_text(&Fx::default().finish());
    assert!(g.pages.is_empty());
    assert_eq!(report.source_pages, 0);
}

#[test]
fn heading_levels_clamp_to_three() {
    let mut fx = base();
    fx.block(41, PROJECT, PROJECT, "a0", "big");
    fx.raw(41, "logseq.property/heading", "2");
    fx.block(42, PROJECT, PROJECT, "a1", "bigger");
    fx.raw(42, "logseq.property/heading", "5");
    let (g, _) = parse_text(&fx.finish());
    let b = outline(named(&g, "Page"));
    assert_eq!(b[0].heading, Some(2));
    assert_eq!(b[1].heading, Some(3));
}

// --- display-type without the matching class tag --------------------------------

#[test]
fn a_block_with_only_a_code_display_type_is_a_code_block() {
    let mut fx = base();
    fx.block(41, PROJECT, PROJECT, "a0", "fn main() {}");
    fx.raw(41, "logseq.property.node/display-type", ":code");
    fx.raw(41, "logseq.property.code/lang", "\"rust\"");
    let (g, _) = parse_text(&fx.finish());
    match &outline(named(&g, "Page"))[0].content {
        BlockContent::Code { lang, body } => {
            assert_eq!(lang.as_deref(), Some("rust"));
            assert_eq!(body, "fn main() {}");
        }
        other => panic!("expected code, got {other:?}"),
    }
}

#[test]
fn a_closed_value_code_display_type_is_a_code_block() {
    let mut fx = base();
    fx.ent(
        50,
        Some("logseq.property.node/display-type.code"),
        Some("Code"),
    );
    fx.block(41, PROJECT, PROJECT, "a0", "let x = 1;");
    fx.raw(41, "logseq.property.node/display-type", "50");
    let (g, _) = parse_text(&fx.finish());
    assert!(matches!(
        outline(named(&g, "Page"))[0].content,
        BlockContent::Code { .. }
    ));
}

#[test]
fn quote_and_math_display_types_behave_like_their_classes() {
    let mut fx = base();
    fx.block(41, PROJECT, PROJECT, "a0", "to be");
    fx.raw(41, "logseq.property.node/display-type", ":quote");
    fx.block(42, PROJECT, PROJECT, "a1", "x^2");
    fx.raw(42, "logseq.property.node/display-type", ":math");
    let (g, report) = parse_text(&fx.finish());
    let b = outline(named(&g, "Page"));
    assert_eq!(text(&b[0]), "> to be");
    assert_eq!(text(&b[1]), "x^2");
    assert_eq!(report.components_verbatim.get("quote"), Some(&1));
    assert_eq!(report.components_verbatim.get("math"), Some(&1));
}
