//! Golden tests for the Logseq dialect — one case per fidelity-matrix
//! row that the outline parser owns.

use super::*;
use crate::ir::Inline;

fn parse_page(text: &str) -> (Vec<(String, String)>, Vec<ImportBlock>, ImportReport) {
    let mut report = ImportReport::new("logseq");
    let (props, blocks) = parse_outline(text, Path::new(""), &mut Vec::new(), &mut report);
    (props, blocks, report)
}

fn inline_text(b: &ImportBlock) -> String {
    match &b.content {
        BlockContent::Inline(toks) => toks
            .iter()
            .map(|t| match t {
                Inline::Text(s) => s.clone(),
                Inline::CodeSpan(s) => format!("`{s}`"),
                Inline::BlockRef { uid, .. } => format!("(({uid}))"),
                Inline::Embed(_) => "<embed>".to_string(),
                Inline::Component { raw, .. } => raw.clone(),
            })
            .collect(),
        other => panic!("expected inline, got {other:?}"),
    }
}

#[test]
fn id_lines_become_uids_not_text() {
    let (_, blocks, report) =
        parse_page("- first\n  id:: 6601a2c1-4f31-4a45-1c2c-3a5e6b7d8f90\n- second\n");
    assert_eq!(blocks.len(), 2);
    assert_eq!(
        blocks[0].uid.as_deref(),
        Some("6601a2c1-4f31-4a45-1c2c-3a5e6b7d8f90")
    );
    assert_eq!(inline_text(&blocks[0]), "first");
    assert!(report.artifacts_stripped >= 1);
}

#[test]
fn nested_bullets_build_a_tree() {
    let (_, blocks, _) = parse_page("- parent\n  - child\n    - grandchild\n- sibling\n");
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].children.len(), 1);
    assert_eq!(blocks[0].children[0].children.len(), 1);
    assert_eq!(
        inline_text(&blocks[0].children[0].children[0]),
        "grandchild"
    );
}

#[test]
fn tab_indentation_builds_the_same_tree() {
    let (_, blocks, _) = parse_page("- parent\n\t- child\n\t\t- grandchild\n");
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].children.len(), 1);
    assert_eq!(blocks[0].children[0].children.len(), 1);
}

#[test]
fn task_states_map_with_nuance_preserved() {
    let (_, blocks, report) = parse_page(
        "- TODO buy milk\n- DOING write spec\n- LATER read paper\n- CANCELED old idea\n",
    );
    assert_eq!(blocks[0].task, Some(TaskState::Todo));
    assert_eq!(blocks[1].task, Some(TaskState::Doing));
    assert_eq!(blocks[2].task, Some(TaskState::Later));
    assert_eq!(blocks[3].task, Some(TaskState::Canceled));
    assert_eq!(inline_text(&blocks[1]), "write spec");
    assert_eq!(report.tasks.get("DOING"), Some(&1));
}

#[test]
fn priority_marker_becomes_property() {
    let (_, blocks, _) = parse_page("- TODO [#A] urgent thing\n");
    assert_eq!(inline_text(&blocks[0]), "urgent thing");
    assert!(blocks[0]
        .props
        .contains(&("priority".to_string(), "A".to_string())));
}

#[test]
fn collapsed_property_sets_the_flag() {
    let (_, blocks, _) = parse_page("- folded\n  collapsed:: true\n  - hidden child\n");
    assert!(blocks[0].collapsed);
    assert!(
        blocks[0].props.is_empty(),
        "collapsed:: must not leak as a prop"
    );
}

#[test]
fn block_properties_carry_over() {
    let (_, blocks, _) = parse_page("- objective\n  priority:: high\n  owner:: [[avelino]]\n");
    assert_eq!(
        blocks[0].props,
        vec![
            ("priority".to_string(), "high".to_string()),
            ("owner".to_string(), "[[avelino]]".to_string())
        ]
    );
}

#[test]
fn logseq_internal_props_are_dropped() {
    let (_, blocks, report) = parse_page("- list\n  logseq.order-list-type:: number\n");
    assert!(blocks[0].props.is_empty());
    assert!(report.artifacts_stripped >= 1);
}

#[test]
fn scheduled_and_deadline_become_date_links() {
    let (_, blocks, report) = parse_page(
        "- DONE ship it\n  SCHEDULED: <2026-11-10 Tue>\n  DEADLINE: <2026-11-12 Thu .+1w>\n",
    );
    assert_eq!(
        inline_text(&blocks[0]),
        "ship it [[2026-11-10]] [[2026-11-12]]"
    );
    assert_eq!(report.org_dates_converted, 2);
}

#[test]
fn logbook_drawers_are_dropped() {
    let (_, blocks, report) = parse_page(
        "- DONE task\n  :LOGBOOK:\n  CLOCK: [2026-05-01 Fri 10:00]--[2026-05-01 Fri 11:00] =>  01:00\n  :END:\n- next\n",
    );
    assert_eq!(blocks.len(), 2);
    assert_eq!(inline_text(&blocks[0]), "task");
    assert!(report.artifacts_stripped >= 1);
}

#[test]
fn page_props_and_directives_lift_off_the_body() {
    let (props, blocks, _) = parse_page("#+title: Real Title\ntype:: project\n\n- first block\n");
    assert!(props.contains(&("title".to_string(), "Real Title".to_string())));
    assert!(props.contains(&("type".to_string(), "project".to_string())));
    assert_eq!(blocks.len(), 1);
}

#[test]
fn fence_block_is_typed_code() {
    let (_, blocks, _) = parse_page("- ```clojure\n  (+ 1 2)\n  ```\n- after\n");
    match &blocks[0].content {
        BlockContent::Code { lang, body } => {
            assert_eq!(lang.as_deref(), Some("clojure"));
            assert_eq!(body, "(+ 1 2)");
        }
        other => panic!("expected code, got {other:?}"),
    }
    assert_eq!(blocks.len(), 2);
}

#[test]
fn fence_body_never_parses_as_props_or_bullets() {
    let (_, blocks, _) = parse_page("- ```text\n  key:: value\n  - not a bullet\n  ```\n");
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].props.is_empty());
    match &blocks[0].content {
        BlockContent::Code { body, .. } => {
            assert!(body.contains("key:: value"));
            assert!(body.contains("- not a bullet"));
        }
        other => panic!("expected code, got {other:?}"),
    }
}

#[test]
fn embeds_and_refs_tokenize() {
    let (_, blocks, _) =
        parse_page("- see ((6601-abc)) and {{embed ((6601-def))}} and {{embed [[Other Page]]}}\n");
    let toks = match &blocks[0].content {
        BlockContent::Inline(t) => t,
        other => panic!("expected inline, got {other:?}"),
    };
    assert!(toks.contains(&Inline::BlockRef {
        uid: "6601-abc".to_string(),
        alias: None
    }));
    assert!(toks.contains(&Inline::Embed(crate::ir::EmbedTarget::Block(
        "6601-def".into()
    ))));
    assert!(toks.contains(&Inline::Embed(crate::ir::EmbedTarget::Page(
        "Other Page".into()
    ))));
}

#[test]
fn multiword_tag_becomes_page_ref() {
    let (_, blocks, report) = parse_page("- see #[[My Project]] now\n");
    assert_eq!(inline_text(&blocks[0]), "see [[My Project]] now");
    assert_eq!(report.tags_multiword_to_page_ref, 1);
}

#[test]
fn underscores_are_not_translated() {
    // Logseq is CommonMark: `__x__` already means bold — unlike Roam.
    let (_, blocks, _) = parse_page("- keep __bold__ as is\n");
    assert_eq!(inline_text(&blocks[0]), "keep __bold__ as is");
}

#[test]
fn page_name_decoding() {
    assert_eq!(decode_page_name("foo%2Fbar"), "foo/bar");
    assert_eq!(decode_page_name("meu___projeto"), "meu projeto");
}

#[test]
fn multiline_continuation_text_stays_with_the_block() {
    let (_, blocks, _) = parse_page("- first line\n  second line of same block\n- next\n");
    assert_eq!(blocks.len(), 2);
    assert_eq!(
        inline_text(&blocks[0]),
        "first line\nsecond line of same block"
    );
}

/// A DB-version graph directory holds `db.sqlite` and none of the file
/// layout; scanning it as a file graph reported 0 pages with no error.
#[test]
fn a_logseq_db_graph_directory_is_refused_instead_of_importing_zero_pages() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("db.sqlite"), b"SQLite format 3\0").expect("write");
    let mut report = ImportReport::new("logseq");
    let err = LogseqAdapter
        .parse(dir.path(), &mut report)
        .expect_err("a DB graph is refused")
        .to_string();
    assert!(err.contains("Export EDN file"), "{err}");
    assert!(err.contains("outl import logseq-db"), "{err}");
}
