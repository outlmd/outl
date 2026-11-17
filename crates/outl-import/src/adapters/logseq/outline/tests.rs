//! Golden tests for the Logseq outline parser — one case per
//! fidelity-matrix row the parser owns.

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

/// Every marker Logseq accepts is a task. These three used to keep
/// the word as text and import as an untracked block.
#[test]
fn the_rest_of_logseqs_markers_are_tasks_too() {
    let (_, blocks, report) =
        parse_page("- WAIT on review\n- IN-PROGRESS refactor\n- STARTED draft\n");
    assert_eq!(blocks[0].task, Some(TaskState::Waiting));
    assert_eq!(inline_text(&blocks[0]), "on review");
    assert_eq!(blocks[1].task, Some(TaskState::Doing));
    assert_eq!(inline_text(&blocks[1]), "refactor");
    assert_eq!(blocks[2].task, Some(TaskState::Doing));
    assert_eq!(report.tasks.get("DOING"), Some(&2));
}

/// A word that merely starts with a marker is not a task.
#[test]
fn a_marker_needs_its_own_word() {
    let (_, blocks, _) = parse_page("- TODOs for later\n- DONEness\n");
    assert_eq!(blocks[0].task, None);
    assert_eq!(inline_text(&blocks[0]), "TODOs for later");
    assert_eq!(blocks[1].task, None);
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

/// A ref in a property value is a ref. It used to be written out
/// verbatim and stay inert text, because a property never passes
/// through the inline tokenizer.
#[test]
fn a_ref_in_a_property_value_becomes_a_placeholder() {
    let (_, blocks, report) =
        parse_page("- item\n  related:: ((6601a2c1-4f31-4a45-1c2c-3a5e6b7d8f90))\n");
    let (_, v) = &blocks[0].props[0];
    assert_eq!(
        v, "((outl-import:6601a2c1-4f31-4a45-1c2c-3a5e6b7d8f90))",
        "the resolve pass rewrites this to a real handle"
    );
    assert_eq!(report.refs_in_prop_values, 1);
}

/// Prose double-parentheticals in a property value stay text.
#[test]
fn a_non_uid_parenthetical_in_a_property_value_is_left_alone() {
    let (_, blocks, report) = parse_page("- item\n  note:: ((an aside, really))\n");
    assert_eq!(blocks[0].props[0].1, "((an aside, really))");
    assert_eq!(report.refs_in_prop_values, 0);
}

#[test]
fn logseq_internal_props_are_dropped() {
    let (_, blocks, report) = parse_page("- list\n  logseq.order-list-type:: number\n");
    assert!(blocks[0].props.is_empty());
    assert!(report.artifacts_stripped >= 1);
}

/// The date link stays (issue #63) and the kind is now recoverable
/// (issue #578), time of day and repeater included.
#[test]
fn scheduled_and_deadline_keep_their_kind() {
    let (_, blocks, report) = parse_page(
        "- DONE ship it\n  SCHEDULED: <2026-11-10 Tue>\n  DEADLINE: <2026-11-12 Thu .+1w>\n",
    );
    assert_eq!(
        inline_text(&blocks[0]),
        "ship it [[2026-11-10]] [[2026-11-12]]"
    );
    assert_eq!(
        blocks[0].props,
        vec![
            ("scheduled".to_string(), "[[2026-11-10]]".to_string()),
            ("deadline".to_string(), "[[2026-11-12]] .+1w".to_string()),
        ]
    );
    assert_eq!(report.org_dates_converted, 2);
}

#[test]
fn a_stamp_keeps_its_time_of_day() {
    let (_, blocks, _) = parse_page("- TODO call\n  SCHEDULED: <2026-11-10 Tue 09:30>\n");
    assert_eq!(blocks[0].props[0].1, "[[2026-11-10]] 09:30");
}

/// One key per property: a second stamp of the same kind joins the
/// value instead of writing a duplicate key the op log collapses.
#[test]
fn two_stamps_of_one_kind_join_one_property() {
    let (_, blocks, _) =
        parse_page("- TODO twice\n  SCHEDULED: <2026-11-10 Tue>\n  SCHEDULED: <2026-11-11 Wed>\n");
    assert_eq!(blocks[0].props.len(), 1);
    assert_eq!(blocks[0].props[0].1, "[[2026-11-10]], [[2026-11-11]]");
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

/// `#+BEGIN_SRC` is a code block in Logseq, and the `logseq-db`
/// adapter already imports the same construct as a fence.
#[test]
fn an_org_src_block_becomes_a_fenced_code_block() {
    let (_, blocks, report) =
        parse_page("- #+BEGIN_SRC clojure\n  (+ 1 2)\n  #+END_SRC\n- after\n");
    match &blocks[0].content {
        BlockContent::Code { lang, body } => {
            assert_eq!(lang.as_deref(), Some("clojure"));
            assert_eq!(body, "(+ 1 2)");
        }
        other => panic!("expected code, got {other:?}"),
    }
    assert_eq!(blocks.len(), 2);
    assert_eq!(report.org_blocks_converted, 1, "one block, one count");
}

#[test]
fn an_org_quote_block_becomes_quote_lines() {
    let (_, blocks, _) = parse_page("- #+BEGIN_QUOTE\n  a line\n  another\n  #+END_QUOTE\n");
    assert_eq!(inline_text(&blocks[0]), "> a line\n> another");
}

/// An org block inside a markdown fence is the code someone is
/// quoting, not a construct to translate.
#[test]
fn an_org_block_inside_a_fence_is_left_alone() {
    let (_, blocks, _) = parse_page("- ```org\n  #+BEGIN_SRC js\n  x\n  #+END_SRC\n  ```\n");
    match &blocks[0].content {
        BlockContent::Code { body, .. } => assert!(body.contains("#+BEGIN_SRC js"), "{body}"),
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
fn multiline_continuation_text_stays_with_the_block() {
    let (_, blocks, _) = parse_page("- first line\n  second line of same block\n- next\n");
    assert_eq!(blocks.len(), 2);
    assert_eq!(
        inline_text(&blocks[0]),
        "first line\nsecond line of same block"
    );
}

/// Text before the first bullet is content. It used to be dropped
/// line by line with nothing in the report to show for it.
#[test]
fn free_text_before_the_first_bullet_becomes_a_block() {
    let (props, blocks, report) =
        parse_page("type:: project\n\nSome preamble.\nSecond line.\n\n- a bullet\n");
    assert!(props.contains(&("type".to_string(), "project".to_string())));
    assert_eq!(blocks.len(), 2);
    assert_eq!(inline_text(&blocks[0]), "Some preamble.\nSecond line.");
    assert_eq!(inline_text(&blocks[1]), "a bullet");
    assert_eq!(report.blocks_from_pre_text, 1);
}

/// A file with no bullets at all (written outside Logseq) is one
/// block, not an empty page.
#[test]
fn a_file_with_no_bullets_is_not_empty() {
    let (_, blocks, _) = parse_page("just prose\nand more prose\n");
    assert_eq!(blocks.len(), 1);
    assert_eq!(inline_text(&blocks[0]), "just prose\nand more prose");
}

#[test]
fn a_properties_only_page_has_no_pre_block() {
    let (props, blocks, report) = parse_page("type:: project\nalias:: other\n");
    assert_eq!(props.len(), 2);
    assert!(blocks.is_empty());
    assert_eq!(report.blocks_from_pre_text, 0);
}

/// An org block the source never closed. The content is what matters:
/// a translation that loses a line is worse than no translation.
#[test]
fn an_unclosed_org_block_keeps_every_line() {
    let (_, blocks, _) = parse_page("- #+BEGIN_SRC python\n  x = 1\n  y = 2\n- next\n");
    let text = match &blocks[0].content {
        BlockContent::Code { body, .. } => body.clone(),
        BlockContent::Verbatim(s) => s.clone(),
        BlockContent::Inline(_) => inline_text(&blocks[0]),
    };
    assert!(text.contains("x = 1"), "{text}");
    assert!(text.contains("y = 2"), "{text}");
    assert_eq!(blocks.len(), 2, "the next bullet is still its own block");
}

/// A blank line inside an org src block is part of the code.
#[test]
fn a_blank_line_inside_an_org_src_block_survives() {
    let (_, blocks, _) = parse_page("- #+BEGIN_SRC python\n  a = 1\n\n  b = 2\n  #+END_SRC\n");
    match &blocks[0].content {
        BlockContent::Code { body, .. } => {
            assert!(body.contains("a = 1"), "{body}");
            assert!(body.contains("b = 2"), "{body}");
        }
        other => panic!("expected code, got {other:?}"),
    }
}

/// The body of an org src block is code, all of it: a `k:: v` line is
/// not a property, a `- x` line is not a child, and blank lines stay
/// where they were. Each of those used to be classified before the org
/// conversion ran, so the fence came out with lines lost or moved.
#[test]
fn an_org_src_body_is_kept_verbatim() {
    let (_, blocks, _) = parse_page(
        "- #+BEGIN_SRC text\n  id:: example\n\n  - literal\n  key:: value\n  #+END_SRC\n- after\n",
    );
    assert_eq!(blocks.len(), 2, "{blocks:?}");
    assert!(blocks[0].children.is_empty(), "{:?}", blocks[0].children);
    assert!(blocks[0].props.is_empty(), "{:?}", blocks[0].props);
    assert_eq!(blocks[0].uid, None);
    match &blocks[0].content {
        BlockContent::Code { lang, body } => {
            assert_eq!(lang.as_deref(), Some("text"));
            assert_eq!(body, "id:: example\n\n- literal\nkey:: value");
        }
        other => panic!("expected code, got {other:?}"),
    }
}

/// A stamp with no angle brackets is not a stamp — it stays the text
/// the user wrote rather than vanishing into a property.
#[test]
fn a_malformed_stamp_stays_text() {
    let (_, blocks, report) = parse_page("- TODO something\n  SCHEDULED: soon\n");
    assert_eq!(inline_text(&blocks[0]), "something\nSCHEDULED: soon");
    assert_eq!(report.org_dates_converted, 0);
    assert!(blocks[0].props.is_empty());
}

/// An empty file is an empty page, not a panic.
#[test]
fn an_empty_file_parses() {
    let (props, blocks, _) = parse_page("");
    assert!(props.is_empty());
    assert!(blocks.is_empty());
}

/// A bare bullet with nothing after it is still a block.
#[test]
fn a_bare_bullet_is_a_block() {
    let (_, blocks, _) = parse_page("-\n- after\n");
    assert_eq!(blocks.len(), 2);
}
