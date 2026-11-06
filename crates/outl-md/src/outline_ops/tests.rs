//! Unit tests for the `outline_ops` AST helpers.
//!
//! Split out of `outline_ops.rs` to keep the module under the
//! file-size guard — the same seam as `parse/tests.rs` and
//! `frontmatter/tests.rs`: implementation / tests, never a seam
//! inside the helpers themselves.
//!
//! `super` is `crate::outline_ops`, so every test keeps its exact
//! path and name.

use super::*;

fn block(text: &str) -> OutlineNode {
    OutlineNode {
        text: text.into(),
        ..Default::default()
    }
}

#[test]
fn count_todos_counts_doing_as_open_work() {
    // A started task is unfinished work: it belongs in the
    // denominator, never in the numerator. Reading `DOING` as
    // done would let the progress chip show 3/3 on a page where
    // nothing shipped.
    let blocks = vec![
        OutlineNode {
            text: "TODO write the RFC".into(),
            children: vec![block("DOING ship the parser")],
            ..Default::default()
        },
        block("DONE read the paper"),
        block("just a note"),
    ];
    assert_eq!(count_todos(&blocks), (1, 3));
}

#[test]
fn count_todos_ignores_a_marker_word_without_its_space() {
    assert_eq!(count_todos(&[block("DOINGs are piling up")]), (0, 0));
}

#[test]
fn count_todos_sees_a_marker_behind_a_quote_marker() {
    // `"> TODO foo"` is the legacy authoring order, and the TUI
    // already draws it as a checkbox (`split_block_prefixes` takes
    // the two prefixes in either order). The chip has to agree
    // with what is on screen, or the page shows four checkboxes
    // and counts two.
    let blocks = vec![
        block("> TODO write the RFC"),
        block("> [ ] file the issue"),
        block("> DONE read the paper"),
        block("TODO > canonical order"),
        block("> just a quote"),
    ];
    assert_eq!(count_todos(&blocks), (1, 4));
}

#[test]
fn count_todos_unwraps_only_one_quote_marker() {
    // "No nested quotes" — the inner `> ` stays part of the body,
    // so this is a quote of a quote, not a task.
    assert_eq!(count_todos(&[block("> > TODO foo")]), (0, 0));
}

#[test]
fn count_todos_counts_the_checkbox_spelling_too() {
    // A block written `- [ ] ship it` draws a checkbox, so the
    // progress chip has to see it as well — counting one spelling
    // and rendering the other is how 3/7 disagrees with the
    // screen (issue #230).
    let blocks = vec![
        block("[ ] write the RFC"),
        block("[/] ship the parser"),
        block("[x] read the paper"),
        block("[X] merge it"),
        block("[x](https://example.com) is a link, not a task"),
    ];
    assert_eq!(count_todos(&blocks), (2, 4));
}

#[test]
fn count_todos_counts_quote_first_tasks() {
    // `"> TODO foo"` is the legacy authoring order the TUI's
    // `split_block_prefixes` renders as a quoted checkbox, so the
    // progress chip has to count it — a checkbox on screen that
    // the chip skips disagrees with what the user sees. The
    // canonical order (`"TODO > foo"`) already matched.
    let blocks = vec![
        block("> TODO write the RFC"),
        block("> [ ] ship the parser"),
        block("> DONE read the paper"),
        block("TODO > canonical order"),
        block("> just a quote"),
        block("> > TODO nested quotes stay prose"),
    ];
    assert_eq!(count_todos(&blocks), (1, 4));
}

#[test]
fn flat_count_counts_nested_blocks() {
    let blocks = vec![
        OutlineNode {
            text: "a".into(),
            children: vec![block("a1"), block("a2")],
            ..Default::default()
        },
        block("b"),
    ];
    assert_eq!(flat_count(&blocks), 4);
}

#[test]
fn path_for_index_round_trips() {
    let blocks = vec![
        OutlineNode {
            text: "a".into(),
            children: vec![block("a1"), block("a2")],
            ..Default::default()
        },
        block("b"),
    ];
    for i in 0..flat_count(&blocks) {
        let path = path_for_index(&blocks, i).unwrap();
        let back = index_for_path(&blocks, &path).unwrap();
        assert_eq!(back, i, "round-trip failed at index {i}");
    }
}

#[test]
fn indent_makes_block_child_of_previous_sibling() {
    let mut blocks = vec![block("a"), block("b")];
    let new_path = indent_at_path(&mut blocks, &[1]).unwrap();
    assert_eq!(new_path, vec![0, 0]);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].children.len(), 1);
    assert_eq!(blocks[0].children[0].text, "b");
}

#[test]
fn indent_first_block_is_noop() {
    let mut blocks = vec![block("a")];
    assert!(indent_at_path(&mut blocks, &[0]).is_none());
}

#[test]
fn outdent_promotes_child_to_grandparent_level() {
    let mut blocks = vec![OutlineNode {
        text: "a".into(),
        children: vec![block("a1")],
        ..Default::default()
    }];
    let new_path = outdent_at_path(&mut blocks, &[0, 0]).unwrap();
    assert_eq!(new_path, vec![1]);
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[1].text, "a1");
}

#[test]
fn outdent_top_level_is_noop() {
    let mut blocks = vec![block("a")];
    assert!(outdent_at_path(&mut blocks, &[0]).is_none());
}

#[test]
fn insert_sibling_after_inserts_at_correct_position() {
    let mut blocks = vec![block("a"), block("b")];
    insert_sibling_after(&mut blocks, &[0]);
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0].text, "a");
    assert_eq!(blocks[1].text, "");
    assert_eq!(blocks[2].text, "b");
}

/// Regression for issue #55: the TUI falls back to `vec![0]` when
/// `path_for_index` returns `None` (typical when the page parses to
/// zero blocks — e.g. the seeded journal starts with `# heading`,
/// which is not a block marker). The previous implementation
/// computed `pos = last + 1 = 1` against an empty Vec and panicked
/// with "insertion index (is 1) should be <= len (is 0)".
#[test]
fn insert_sibling_after_clamps_when_blocks_empty() {
    let mut blocks: Vec<OutlineNode> = Vec::new();
    insert_sibling_after(&mut blocks, &[0]);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].text, "");
}

#[test]
fn insert_sibling_before_clamps_when_blocks_empty() {
    let mut blocks: Vec<OutlineNode> = Vec::new();
    insert_sibling_before(&mut blocks, &[0]);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].text, "");
}

/// OUTL-4: a stale *parent* segment (not just the leaf) used to
/// panic inside `siblings_mut`'s unchecked `current[idx]` — "index
/// out of bounds: the len is 0 but the index is 13" on an empty
/// outline with a nested path like `[13, 0]`. The parent segment
/// (13) is whatever stale path a caller held (an `Insert`'s
/// `block_path`, a backlink's `source_block_path` + `sub_path`),
/// not a magic constant.
#[test]
fn insert_sibling_after_clamps_when_parent_segment_out_of_range() {
    let mut blocks: Vec<OutlineNode> = Vec::new();
    insert_sibling_after(&mut blocks, &[13, 0]);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].text, "");
}

#[test]
fn insert_sibling_after_with_text_clamps_when_parent_segment_out_of_range() {
    let mut blocks: Vec<OutlineNode> = Vec::new();
    insert_sibling_after_with_text(&mut blocks, &[13, 0], "tail".to_string());
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].text, "tail");
}

#[test]
fn insert_sibling_before_clamps_when_parent_segment_out_of_range() {
    let mut blocks: Vec<OutlineNode> = Vec::new();
    insert_sibling_before(&mut blocks, &[13, 0]);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].text, "");
}

#[test]
fn delete_at_path_noops_when_parent_segment_out_of_range() {
    let mut blocks: Vec<OutlineNode> = Vec::new();
    delete_at_path(&mut blocks, &[13, 0]);
    assert_eq!(blocks.len(), 0);
}

#[test]
fn indent_at_path_noops_when_parent_segment_out_of_range() {
    let mut blocks = vec![block("a"), block("b")];
    assert!(indent_at_path(&mut blocks, &[13, 0]).is_none());
    assert_eq!(blocks.len(), 2);
}

#[test]
fn outdent_at_path_noops_when_parent_segment_out_of_range() {
    let mut blocks = vec![block("a"), block("b")];
    assert!(outdent_at_path(&mut blocks, &[13, 0, 0]).is_none());
    assert_eq!(blocks.len(), 2);
}

#[test]
fn move_up_at_path_noops_when_parent_segment_out_of_range() {
    let mut blocks = vec![block("a"), block("b")];
    assert!(move_up_at_path(&mut blocks, &[13, 0]).is_none());
    assert_eq!(blocks[0].text, "a");
    assert_eq!(blocks[1].text, "b");
}

#[test]
fn move_down_at_path_noops_when_parent_segment_out_of_range() {
    let mut blocks = vec![block("a"), block("b")];
    assert!(move_down_at_path(&mut blocks, &[13, 0]).is_none());
    assert_eq!(blocks[0].text, "a");
    assert_eq!(blocks[1].text, "b");
}

#[test]
fn insert_sibling_after_with_text_seeds_the_new_block() {
    // The TUI block-split path: "hello world" split at the cursor
    // leaves the head behind and drops the tail into a new sibling.
    let mut blocks = vec![block("hello"), block("b")];
    insert_sibling_after_with_text(&mut blocks, &[0], " world".to_string());
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0].text, "hello");
    assert_eq!(blocks[1].text, " world");
    assert_eq!(blocks[2].text, "b");
}

#[test]
fn insert_sibling_after_with_text_clamps_when_blocks_empty() {
    let mut blocks: Vec<OutlineNode> = Vec::new();
    insert_sibling_after_with_text(&mut blocks, &[0], "tail".to_string());
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].text, "tail");
}

#[test]
fn delete_removes_block_and_descendants() {
    let mut blocks = vec![
        OutlineNode {
            text: "a".into(),
            children: vec![block("a1")],
            ..Default::default()
        },
        block("b"),
    ];
    delete_at_path(&mut blocks, &[0]);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].text, "b");
}

// `flatten_backlink_subtree` moved to
// `outl_actions::flatten_subtree_paths` along with the rest of the
// backlinks pipeline. Coverage now lives in `outl-actions::outline`
// tests next to the helper it operates on.

#[test]
fn descendants_count_handles_nested() {
    let blocks = vec![OutlineNode {
        text: "a".into(),
        children: vec![block("a1"), block("a2")],
        ..Default::default()
    }];
    assert_eq!(descendants_count_at_path(&blocks, &[0]), 2);
    assert_eq!(descendants_count_at_path(&blocks, &[0, 0]), 0);
}
