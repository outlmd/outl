//! Golden tests for the Roam dialect — one case per fidelity-matrix row.

use super::*;
use crate::ir::{BlockContent, EmbedTarget, Inline, TaskState};
use crate::report::ImportWarning;

mod report;

fn parse_one_block(s: &str) -> (ImportBlock, ImportReport) {
    let mut report = ImportReport::new("roam");
    let b = RoamBlock {
        string: s.to_string(),
        uid: "u1".to_string(),
        children: Vec::new(),
        heading: None,
        open: None,
        create_time: None,
        edit_time: None,
    };
    let out = convert_block(&b, "Test Page", Path::new(""), &mut Vec::new(), &mut report);
    (out, report)
}

fn inline_tokens(b: &ImportBlock) -> &[Inline] {
    match &b.content {
        BlockContent::Inline(toks) => toks,
        other => panic!("expected inline content, got {other:?}"),
    }
}

#[test]
fn block_ref_becomes_token() {
    let (b, _) = parse_one_block("see ((abc-123)) please");
    let toks = inline_tokens(&b);
    assert!(toks.contains(&Inline::BlockRef {
        uid: "abc-123".to_string(),
        alias: None,
    }));
}

#[test]
fn prose_double_parens_stay_text() {
    let (b, _) = parse_one_block("an aside ((not a uid, honest)) here");
    let toks = inline_tokens(&b);
    assert!(toks.iter().all(|t| !matches!(t, Inline::BlockRef { .. })));
}

#[test]
fn aliased_block_ref() {
    let (b, _) = parse_one_block("[the decision](((abc)))");
    let toks = inline_tokens(&b);
    assert!(toks.contains(&Inline::BlockRef {
        uid: "abc".to_string(),
        alias: Some("the decision".to_string()),
    }));
}

#[test]
fn alias_page_link_passes_verbatim() {
    let (b, _) = parse_one_block("[alias]([[Some Page]]) end");
    let toks = inline_tokens(&b);
    let text: String = toks
        .iter()
        .filter_map(|t| match t {
            Inline::Text(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    assert!(text.contains("[alias]([[Some Page]])"));
}

#[test]
fn embed_block_and_page() {
    let (b, _) = parse_one_block("{{[[embed]]: ((CLGp3LXxM))}}");
    assert!(inline_tokens(&b).contains(&Inline::Embed(EmbedTarget::Block("CLGp3LXxM".into()))));

    let (b, _) = parse_one_block("{{embed: [[buser/team]]}}");
    assert!(inline_tokens(&b).contains(&Inline::Embed(EmbedTarget::Page("buser/team".into()))));
}

#[test]
fn todo_done_markers_set_task_state() {
    let (b, report) = parse_one_block("{{[[TODO]]}} buy milk");
    assert_eq!(b.task, Some(TaskState::Todo));
    assert_eq!(inline_tokens(&b), &[Inline::Text("buy milk".into())]);
    assert_eq!(report.tasks.get("TODO"), Some(&1));

    let (b, _) = parse_one_block("{{[[DONE]]}} laundry");
    assert_eq!(b.task, Some(TaskState::Done));
}

#[test]
fn multiword_tag_is_kept_as_written() {
    // outl reads `#[[My Project]]` as `#` + the `[[My Project]]` page
    // ref, so the link survives without rewriting the text.
    let (b, report) = parse_one_block("see #[[My Project]] today");
    let toks = inline_tokens(&b);
    assert_eq!(toks, &[Inline::Text("see #[[My Project]] today".into())]);
    assert_eq!(report.tags_multiword_to_page_ref, 0);
    // Atomic: a `((…))` inside the tag name is not a block ref.
    let (b, _) = parse_one_block("#[[a ((b)) c]]");
    assert_eq!(inline_tokens(&b), &[Inline::Text("#[[a ((b)) c]]".into())]);
}

#[test]
fn single_tag_passes_through() {
    let (b, _) = parse_one_block("#1on1 with [[@Someone]]");
    assert_eq!(
        inline_tokens(&b),
        &[Inline::Text("#1on1 with [[@Someone]]".into())]
    );
}

#[test]
fn roam_italic_underscores_become_stars() {
    let (b, _) = parse_one_block("so __important__ indeed");
    assert_eq!(
        inline_tokens(&b),
        &[Inline::Text("so *important* indeed".into())]
    );
}

#[test]
fn highlight_becomes_double_equals() {
    let (b, _) = parse_one_block("mark ^^this^^ well");
    assert_eq!(
        inline_tokens(&b),
        &[Inline::Text("mark ==this== well".into())]
    );
}

#[test]
fn code_span_protects_dialect_rewrites() {
    let (b, _) = parse_one_block("`__not italic__` but __this is__");
    let toks = inline_tokens(&b);
    assert_eq!(
        toks,
        &[
            Inline::CodeSpan("__not italic__".into()),
            Inline::Text(" but *this is*".into()),
        ]
    );
}

#[test]
fn nested_page_ref_consumed_atomically() {
    let (b, _) = parse_one_block("[[a [[b]] c]] tail");
    assert_eq!(
        inline_tokens(&b),
        &[Inline::Text("[[a [[b]] c]] tail".into())]
    );
}

#[test]
fn latex_is_verbatim_component() {
    let (b, report) = parse_one_block("energy $$e = mc^2$$ done");
    let toks = inline_tokens(&b);
    assert!(toks.iter().any(|t| matches!(
        t,
        Inline::Component { raw, .. } if raw == "$$e = mc^2$$"
    )));
    assert_eq!(report.components_verbatim.get("latex"), Some(&1));
}

#[test]
fn unknown_component_verbatim_and_counted() {
    let (b, report) = parse_one_block("{{word-count}} here");
    let toks = inline_tokens(&b);
    assert!(toks.iter().any(|t| matches!(
        t,
        Inline::Component { raw, .. } if raw == "{{word-count}}"
    )));
    assert_eq!(report.components_verbatim.get("word-count"), Some(&1));
}

#[test]
fn pure_fence_becomes_code() {
    let (b, _) = parse_one_block("```clojure\n(+ 1 2)\n```");
    match &b.content {
        BlockContent::Code { lang, body } => {
            assert_eq!(lang.as_deref(), Some("clojure"));
            assert_eq!(body, "(+ 1 2)");
        }
        other => panic!("expected code, got {other:?}"),
    }
}

#[test]
fn mixed_fence_stays_verbatim() {
    let raw = "intro text\n```sql\nselect 1\n```";
    let (b, _) = parse_one_block(raw);
    match &b.content {
        BlockContent::Verbatim(s) => assert_eq!(s, raw),
        other => panic!("expected verbatim, got {other:?}"),
    }
}

#[test]
fn org_deadline_becomes_date_link() {
    let (b, report) = parse_one_block("finish it DEADLINE: <2025-11-12 Wed>");
    assert_eq!(
        inline_tokens(&b),
        &[Inline::Text("finish it DEADLINE: [[2025-11-12]]".into())]
    );
    assert_eq!(report.org_dates_converted, 1);
}

/// The keyword says what the date is, and a time or repeater in the
/// stamp is information the link alone does not carry.
#[test]
fn org_stamp_keeps_keyword_time_and_repeater() {
    let (b, report) = parse_one_block(
        "SCHEDULED: <2025-11-12 Wed 10:00 +1w>\nDEADLINE:<2025-11-13 thu>\nDEADLINE: <2025-11-14 Mon>",
    );
    assert_eq!(
        inline_tokens(&b),
        &[Inline::Text(
            "SCHEDULED: [[2025-11-12]] 10:00 +1w\nDEADLINE:[[2025-11-13]]\nDEADLINE: [[2025-11-14]] Mon"
                .into()
        )]
    );
    assert_eq!(report.org_dates_converted, 3);
    // Not a stamp: left alone.
    let (b, _) = parse_one_block("DEADLINE: soon");
    assert_eq!(inline_tokens(&b), &[Inline::Text("DEADLINE: soon".into())]);
}

#[test]
fn simple_query_translates_to_fence() {
    let (b, report) = parse_one_block("{{[[query]]: {and: [[TODO]] [[Be Happy]]}}}");
    match &b.content {
        BlockContent::Code { lang, body } => {
            assert_eq!(lang.as_deref(), Some("query"));
            assert!(body.contains("status: todo"));
            assert!(body.contains("text: [[Be Happy]]"));
        }
        other => panic!("expected query fence, got {other:?}"),
    }
    assert_eq!(report.queries_translated, 1);
}

#[test]
fn nested_query_stays_verbatim_component() {
    let raw = "{{[[query]]: {and: [[x]] {between: [[May 1st, 2026]] [[May 31st, 2026]]}}}}";
    let (b, report) = parse_one_block(raw);
    let toks = inline_tokens(&b);
    assert!(toks.iter().any(|t| matches!(
        t,
        Inline::Component {
            kind: ComponentKind::Query,
            ..
        }
    )));
    assert_eq!(report.components_verbatim.get("query"), Some(&1));
    assert_eq!(report.queries_translated, 0);
}

#[test]
fn collapsed_and_heading_and_timestamps_carry_over() {
    let mut report = ImportReport::new("roam");
    let b = RoamBlock {
        string: "a heading".to_string(),
        uid: "h1".to_string(),
        children: Vec::new(),
        heading: Some(2),
        open: Some(false),
        create_time: Some(1_700_000_000_000),
        edit_time: None,
    };
    let out = convert_block(&b, "P", Path::new(""), &mut Vec::new(), &mut report);
    assert_eq!(out.heading, Some(2));
    assert!(out.collapsed);
    assert!(out.created.is_some());
}

#[test]
fn text_plus_prop_block_keeps_attribute_line_for_the_parser() {
    // A block that mixes prose with a `key:: value` line: the adapter
    // leaves the attribute in the text as a continuation line, so outl's
    // own parser lifts it into a block property — and resolves any ref in
    // its value through the placeholder pass. Extracting it here would
    // bypass that resolution (the Omnivore `note:: ((uid))` shape).
    let (b, _) = parse_one_block("[[2023-01-31]] #1on1\nwork:: [[buser]]");
    assert!(b.props.is_empty());
    let text: String = inline_tokens(&b)
        .iter()
        .filter_map(|t| match t {
            Inline::Text(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    assert!(text.contains("[[2023-01-31]] #1on1"));
    assert!(text.contains("work:: [[buser]]"));
}

#[test]
fn multiple_property_lines_all_carry_over() {
    let (b, _) = parse_one_block("icon:: 🏢\npage-type:: company\nsite:: https://x.com");
    // Pure-prop block: no visible text, every attribute captured.
    assert!(inline_tokens(&b)
        .iter()
        .all(|t| matches!(t, Inline::Text(s) if s.trim().is_empty())));
    assert_eq!(
        b.props,
        vec![
            ("icon".to_string(), "🏢".to_string()),
            ("page-type".to_string(), "company".to_string()),
            ("site".to_string(), "https://x.com".to_string()),
        ]
    );
}

#[test]
fn inline_collapsed_prop_sets_fold_flag_not_a_property() {
    let (b, _) = parse_one_block("#buser some note\ncollapsed:: true");
    assert!(b.collapsed);
    assert!(b.props.is_empty());
    assert_eq!(
        inline_tokens(&b),
        &[Inline::Text("#buser some note".into())]
    );
}

#[test]
fn property_with_empty_value_and_no_space_is_recognized() {
    // `key::` (no trailing space, empty value) is a property in outl's
    // own parser, so the importer must classify it the same way instead
    // of leaving it as text.
    let (b, _) = parse_one_block("icon::\nrelated:: [[x]]");
    assert!(inline_tokens(&b)
        .iter()
        .all(|t| matches!(t, Inline::Text(s) if s.trim().is_empty())));
    assert_eq!(
        b.props,
        vec![
            ("icon".to_string(), String::new()),
            ("related".to_string(), "[[x]]".to_string()),
        ]
    );
}

#[test]
fn mid_page_attribute_block_is_not_promoted() {
    // Only the LEADING run of pure-attribute blocks promotes to page
    // props. A pure-prop block after real content stays in the outline,
    // or promoting it would reorder content out of place.
    let adapter = RoamAdapter;
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("backup.json");
    std::fs::write(
        &src,
        r#"[
            {"title": "Note", "children": [
                {"string": "icon:: 📌", "uid": "a1", "children": []},
                {"string": "real content here", "uid": "n1", "children": []},
                {"string": "status:: done", "uid": "a2", "children": []}
            ]}
        ]"#,
    )
    .expect("write fixture");
    let mut report = ImportReport::new("roam");
    let graph = adapter.parse(&src, &mut report).expect("parse");
    let page = &graph.pages[0];
    // Only the leading `icon::` promotes; `status::` stays in the body.
    assert_eq!(page.props, vec![("icon".to_string(), "📌".to_string())]);
    match &page.body {
        PageBody::Outline(blocks) => assert_eq!(blocks.len(), 2),
        other => panic!("expected outline, got {other:?}"),
    }
}

/// `id::` is in the Roam block's own text (Logseq residue Roam shows
/// as an attribute). It is kept, not stripped: outl's parser lifts it
/// into a block property like any other `key:: value` line.
#[test]
fn id_line_is_kept() {
    let (b, report) = parse_one_block("real content\nid:: qnkrWK1ba");
    assert!(b.props.is_empty());
    assert_eq!(
        inline_tokens(&b),
        &[Inline::Text("real content\nid:: qnkrWK1ba".into())]
    );
    assert_eq!(report.artifacts_stripped, 0);
    // `collapsed::` is still structural.
    let (b, _) = parse_one_block("x\ncollapsed:: true\nid:: 6908fc01-aaaa");
    assert!(b.collapsed);
    assert_eq!(
        inline_tokens(&b),
        &[Inline::Text("x\nid:: 6908fc01-aaaa".into())]
    );
}

#[test]
fn code_fence_with_colons_keeps_props_untouched() {
    // `::` inside a fence body must not be mistaken for a property.
    let (b, _) = parse_one_block("```rust\nlet x = foo::bar();\n```");
    assert!(b.props.is_empty());
    match &b.content {
        BlockContent::Code { body, .. } => assert!(body.contains("foo::bar()")),
        other => panic!("expected code, got {other:?}"),
    }
}

#[test]
fn attribute_block_with_children_is_not_promoted() {
    // A block that is only attribute lines but HAS children stays in the
    // outline (promotion drops the block, which would orphan its subtree).
    // Its attribute becomes a block property; the children survive.
    let mut report = ImportReport::new("roam");
    let b = RoamBlock {
        string: "related:: [[x]]".to_string(),
        uid: "p1".to_string(),
        children: vec![RoamBlock {
            string: "a child note".to_string(),
            uid: "c1".to_string(),
            children: Vec::new(),
            heading: None,
            open: None,
            create_time: None,
            edit_time: None,
        }],
        heading: None,
        open: None,
        create_time: None,
        edit_time: None,
    };
    let out = convert_block(&b, "P", Path::new(""), &mut Vec::new(), &mut report);
    assert_eq!(
        out.props,
        vec![("related".to_string(), "[[x]]".to_string())]
    );
    assert_eq!(out.children.len(), 1);
}

#[test]
fn top_level_attribute_blocks_promote_to_page_props() {
    let adapter = RoamAdapter;
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("backup.json");
    std::fs::write(
        &src,
        r#"[
            {"title": "@Tonico", "children": [
                {"string": "icon:: 👤\nrelated:: [[triathlon]]", "uid": "a1", "children": []},
                {"string": "page-type:: contact", "uid": "a2", "children": []},
                {"string": "actual note about Tonico", "uid": "n1", "children": []}
            ]}
        ]"#,
    )
    .expect("write fixture");
    let mut report = ImportReport::new("roam");
    let graph = adapter.parse(&src, &mut report).expect("parse");
    let page = &graph.pages[0];
    // The two pure-attribute blocks are lifted into the page header;
    // only the real note remains in the outline.
    assert_eq!(
        page.props,
        vec![
            ("icon".to_string(), "👤".to_string()),
            ("related".to_string(), "[[triathlon]]".to_string()),
            ("page-type".to_string(), "contact".to_string()),
        ]
    );
    match &page.body {
        PageBody::Outline(blocks) => assert_eq!(blocks.len(), 1),
        other => panic!("expected outline, got {other:?}"),
    }
}

#[test]
fn journal_titles_parse_as_dates() {
    let adapter = RoamAdapter;
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("backup.json");
    std::fs::write(
        &src,
        r#"[
            {"title": "May 25th, 2026", "children": [{"string": "x", "uid": "j1", "children": []}]},
            {"title": "Projects", "children": [{"string": "y", "uid": "p1", "children": []}]}
        ]"#,
    )
    .expect("write fixture");
    let mut report = ImportReport::new("roam");
    let graph = adapter.parse(&src, &mut report).expect("parse");
    assert_eq!(graph.pages.len(), 2);
    assert!(matches!(graph.pages[0].name, PageName::Journal(_)));
    assert!(matches!(graph.pages[1].name, PageName::Named(_)));
}
