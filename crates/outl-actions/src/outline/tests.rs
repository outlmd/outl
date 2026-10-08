//! Unit tests for the outline projection.
//!
//! Split out of `outline.rs` for the file-size ratchet, the same way
//! `parse/tests.rs` and `frontmatter/tests.rs` were: the seam is the
//! `#[cfg(test)]` boundary, never a line inside the projection itself.

use super::*;
use crate::todo::TodoState;

fn node(text: &str, children: Vec<OutlineNode>) -> OutlineNode {
    OutlineNode {
        id: format!("test-{text}"),
        text: text.into(),
        todo: None,
        collapsed: false,
        properties: Vec::new(),
        tokens: Vec::new(),
        table: None,
        children,
    }
}

fn leaf(text: &str) -> OutlineNode {
    node(text, Vec::new())
}

#[test]
fn flatten_subtree_paths_returns_dfs_preorder() {
    // Mirrors the previous `outl_md::outline_ops::flatten_backlink_subtree`
    // coverage so behaviour stays identical after the move.
    let root = node(
        "root",
        vec![node("a", vec![leaf("a1"), leaf("a2")]), leaf("b")],
    );
    assert_eq!(
        flatten_subtree_paths(&root),
        vec![
            Vec::<usize>::new(), // root
            vec![0],             // a
            vec![0, 0],          // a1
            vec![0, 1],          // a2
            vec![1],             // b
        ]
    );
}

#[test]
fn flatten_subtree_paths_leaf_returns_just_root() {
    let only = leaf("only-me");
    assert_eq!(flatten_subtree_paths(&only), vec![Vec::<usize>::new()]);
}

fn parsed(text: &str, children: Vec<ParsedOutlineNode>) -> ParsedOutlineNode {
    ParsedOutlineNode {
        text: text.into(),
        properties: Vec::new(),
        children,
    }
}

#[test]
fn project_parsed_subtree_attaches_tokens_and_recurses() {
    // The embed subtree (`!((blk-…))` expansion) rides this: a parsed
    // subtree with inline markup + a nested child must come back as
    // wire nodes carrying tokens, or the client renders empty rows.
    let tree = vec![parsed(
        "parent **bold**",
        vec![parsed("TODO child", Vec::new())],
    )];

    let wire = project_parsed_subtree(&tree);

    assert_eq!(wire.len(), 1);
    let parent = &wire[0];
    assert_eq!(parent.text, "parent **bold**");
    // Tokens are attached (not the empty vec a bare clone would leave).
    assert!(
        parent.tokens.len() > 1,
        "expected tokenized inline markup, got {:?}",
        parent.tokens
    );
    // The child recurses, and its TODO prefix is split off text into `todo`.
    assert_eq!(parent.children.len(), 1);
    let child = &parent.children[0];
    assert_eq!(child.todo, Some(TodoState::Todo));
    assert_eq!(child.text, "child");
    assert!(!child.tokens.is_empty());
}

#[test]
fn prop_value_to_string_covers_every_variant() {
    // `Text` is what `outl-md` actually emits today; the other
    // variants are surfaced for forward-compat. The helper still
    // has to behave sensibly on each so a future indexer doesn't
    // crash on a non-Text page property.
    assert_eq!(
        prop_value_to_string(&PropValue::Text("high".into())),
        "high"
    );
    assert_eq!(
        prop_value_to_string(&PropValue::PageRef("Avelino".into())),
        "Avelino"
    );
    assert_eq!(
        prop_value_to_string(&PropValue::Tag("urgent".into())),
        "urgent"
    );
    assert_eq!(
        prop_value_to_string(&PropValue::List(vec![
            PropValue::Tag("a".into()),
            PropValue::Tag("b".into()),
        ])),
        "a b"
    );
}

fn with_id(text: &str, id: NodeId, children: Vec<OutlineNode>) -> OutlineNode {
    OutlineNode {
        id: id.to_string(),
        text: text.into(),
        todo: None,
        collapsed: false,
        properties: Vec::new(),
        tokens: Vec::new(),
        table: None,
        children,
    }
}

#[test]
fn flat_index_for_block_walks_dfs_preorder() {
    // Layout (DFS pre-order indices in parens):
    //   a (0)
    //     a1 (1)
    //     a2 (2)
    //   b (3)
    // Every node id is exercised so an off-by-one in either the
    // `*counter += 1` or the recursive descent would flip at least
    // one expected index.
    let a = NodeId::new();
    let a1 = NodeId::new();
    let a2 = NodeId::new();
    let b = NodeId::new();
    let outline = vec![
        with_id(
            "a",
            a,
            vec![with_id("a1", a1, vec![]), with_id("a2", a2, vec![])],
        ),
        with_id("b", b, vec![]),
    ];

    assert_eq!(flat_index_for_block(&outline, a), Some(0));
    assert_eq!(flat_index_for_block(&outline, a1), Some(1));
    assert_eq!(flat_index_for_block(&outline, a2), Some(2));
    assert_eq!(flat_index_for_block(&outline, b), Some(3));
}

#[test]
fn flat_index_for_block_traverses_deep_nesting() {
    // Single chain four levels deep: catches a counter that resets
    // when recursing (would make `d` land on 0 instead of 3).
    let a = NodeId::new();
    let b = NodeId::new();
    let c = NodeId::new();
    let d = NodeId::new();
    let outline = vec![with_id(
        "a",
        a,
        vec![with_id(
            "b",
            b,
            vec![with_id("c", c, vec![with_id("d", d, vec![])])],
        )],
    )];

    assert_eq!(flat_index_for_block(&outline, a), Some(0));
    assert_eq!(flat_index_for_block(&outline, b), Some(1));
    assert_eq!(flat_index_for_block(&outline, c), Some(2));
    assert_eq!(flat_index_for_block(&outline, d), Some(3));
}

#[test]
fn flat_index_for_block_returns_none_for_unknown_id() {
    // The block was never in this forest. Caller surfaces as a
    // soft "outline drifted" error; we must not return a stale
    // index from a sibling.
    let known = NodeId::new();
    let outline = vec![with_id("only", known, vec![])];
    let stranger = NodeId::new();
    assert_eq!(flat_index_for_block(&outline, stranger), None);
}

#[test]
fn flat_index_for_block_returns_none_for_empty_forest() {
    let stranger = NodeId::new();
    assert_eq!(flat_index_for_block(&[], stranger), None);
}

#[test]
fn flat_index_for_block_finds_first_match_only() {
    // Same NodeId planted twice (impossible in a real workspace,
    // but the function should not panic and should pick the first
    // DFS hit). Locks in the contract.
    let dup = NodeId::new();
    let outline = vec![
        with_id("first", dup, vec![]),
        with_id("second-with-same-id", dup, vec![]),
    ];
    assert_eq!(flat_index_for_block(&outline, dup), Some(0));
}

#[test]
fn outline_node_carries_todo_text_and_properties() {
    // Smoke that the DTO surface a backlink hands the renderer
    // exposes the fields the TUI uses. We don't go through the
    // workspace here — that's covered by `backlinks` tests.
    let n = OutlineNode {
        id: "x".into(),
        text: "ship it".into(),
        todo: Some(TodoState::Done),
        collapsed: false,
        properties: vec![("priority".into(), "high".into())],
        tokens: Vec::new(),
        table: None,
        children: vec![leaf("child")],
    };
    assert_eq!(n.text, "ship it");
    assert_eq!(n.todo, Some(TodoState::Done));
    assert_eq!(n.properties[0], ("priority".into(), "high".into()));
    assert_eq!(n.children.len(), 1);
}
