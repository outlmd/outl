//! `render(parse(md))` is semantically idempotent: the second parse
//! agrees with the first.
//!
//! Includes a property test over randomly generated outlines.

use outl_md::parse::{parse, OutlineNode, ParsedPage};
use outl_md::render::render;
use proptest::prelude::*;

#[test]
fn fixture_simple_outline() {
    let md = "- a\n- b\n- c\n";
    let p1 = parse(md);
    let rendered = render(&p1);
    let p2 = parse(&rendered);
    assert_eq!(p1, p2);
}

#[test]
fn fixture_nested_with_properties() {
    let md = "title:: doc\nstatus:: active\n\n- objective\n  priority:: high\n  - sub1\n  - sub2\n- riscos\n";
    let p1 = parse(md);
    let rendered = render(&p1);
    let p2 = parse(&rendered);
    assert_eq!(p1, p2);
}

#[test]
fn fixture_empty_block_marker() {
    let md = "-\n- next\n";
    let p1 = parse(md);
    let rendered = render(&p1);
    let p2 = parse(&rendered);
    assert_eq!(p1, p2);
}

#[test]
fn fixture_multi_line_block_via_continuation() {
    // Shift+Enter / Alt+Enter in the TUI produces a `\n` inside the
    // block's text. On disk it's a continuation line indented under
    // the bullet. Must roundtrip.
    let md = "- first line\n  second line\n  third line\n";
    let p1 = parse(md);
    let rendered = render(&p1);
    let p2 = parse(&rendered);
    assert_eq!(p1, p2);
    assert_eq!(p1.blocks[0].text, "first line\nsecond line\nthird line");
}

#[test]
fn fixture_fenced_code_block_inside_bullet() {
    // Code block written inline under a bullet — body preserved
    // literally between the fence markers, even when it would
    // otherwise look like markdown to the parser.
    let md = "- intro\n  ```lisp\n  (+ 1 2)\n  ```\n";
    let p1 = parse(md);
    let rendered = render(&p1);
    let p2 = parse(&rendered);
    assert_eq!(p1, p2);
    assert!(p1.blocks[0].text.contains("```lisp"));
    assert!(p1.blocks[0].text.contains("(+ 1 2)"));
}

#[test]
fn fixture_fence_on_bullet_line_does_not_swallow_next_block() {
    // Regression: when the bullet line itself opens a fence
    // (`- ```lisp`), the closing fence used to be misread as a new
    // opener and absorbed the next bullet.
    let md = "- ```lisp\n  (+ 1 2)\n  ```\n- next\n";
    let p1 = parse(md);
    let rendered = render(&p1);
    let p2 = parse(&rendered);
    assert_eq!(p1, p2);
    assert_eq!(p1.blocks.len(), 2);
    assert_eq!(p1.blocks[1].text, "next");
}

#[test]
fn fixture_result_subblock_under_code_block() {
    // The shape `outl-exec` produces — a code block with a
    // `> **result:**` child — must roundtrip without losing either
    // half.
    let md = "- ```lisp\n  (+ 1 2)\n  ```\n  - > **result:** `3`\n";
    let p1 = parse(md);
    let rendered = render(&p1);
    let p2 = parse(&rendered);
    assert_eq!(p1, p2);
    assert_eq!(p1.blocks[0].children.len(), 1);
    assert!(p1.blocks[0].children[0].text.starts_with("> **result:**"));
}

fn arb_text() -> impl Strategy<Value = String> {
    // ASCII word-ish content — avoids edge cases in `prop_::` parser
    // that aren't part of the markdown contract.
    "[a-z]{1,8}( [a-z]{1,8}){0,4}".prop_map(String::from)
}

/// Multi-line text: a few words on each of N lines, joined by `\n`.
/// Generates the kind of body Shift+Enter produces in the TUI.
fn arb_multi_line_text() -> impl Strategy<Value = String> {
    proptest::collection::vec("[a-z]{1,6}( [a-z]{1,6}){0,3}".prop_map(String::from), 1..4)
        .prop_map(|lines| lines.join("\n"))
}

/// A fenced code block body, including opener and closer. The body
/// is plain ASCII so we don't accidentally generate a nested fence.
fn arb_fenced_code() -> impl Strategy<Value = String> {
    (
        "[a-z]{2,6}",
        proptest::collection::vec("[a-z0-9 ()+\\-*/]{1,30}".prop_map(String::from), 1..4),
    )
        .prop_map(|(lang, body)| format!("```{lang}\n{}\n```", body.join("\n")))
}

fn arb_property() -> impl Strategy<Value = (String, String)> {
    ("[a-z]{2,8}".prop_map(String::from), arb_text())
}

fn arb_node(depth: u32) -> BoxedStrategy<OutlineNode> {
    // Block text can be single-line, multi-line (continuation), or a
    // full fenced code block. All three must roundtrip.
    let block_text = prop_oneof![
        4 => arb_text(),
        2 => arb_multi_line_text(),
        1 => arb_fenced_code(),
    ];

    let leaf = (
        block_text.clone(),
        proptest::collection::vec(arb_property(), 0..2),
    )
        .prop_map(|(text, properties)| OutlineNode {
            text,
            properties,
            children: vec![],
        });
    if depth == 0 {
        leaf.boxed()
    } else {
        let inner = (
            block_text,
            proptest::collection::vec(arb_property(), 0..2),
            proptest::collection::vec(arb_node(depth - 1), 0..3),
        )
            .prop_map(|(text, properties, children)| OutlineNode {
                text,
                properties,
                children,
            });
        prop_oneof![leaf, inner].boxed()
    }
}

/// A YAML frontmatter body, delimiters excluded — what
/// `ParsedPage::frontmatter` holds.
///
/// `key: value` lines only. A body carrying its own `---` or `...` line
/// is not representable (either one closes the fence, which is YAML's own
/// rule, not ours), so generating one would assert a roundtrip the format
/// does not offer.
fn arb_frontmatter() -> impl Strategy<Value = Option<String>> {
    proptest::option::of(
        proptest::collection::vec(
            ("[a-z]{2,8}", "[a-zA-Z0-9 ,\\[\\]/-]{0,20}")
                .prop_map(|(k, v): (String, String)| format!("{k}: {v}")),
            0..4,
        )
        .prop_map(|lines| lines.join("\n")),
    )
}

fn arb_page() -> impl Strategy<Value = ParsedPage> {
    (
        arb_frontmatter(),
        proptest::collection::vec(arb_property(), 0..3),
        proptest::collection::vec(arb_node(2), 1..4),
    )
        .prop_map(|(frontmatter, properties, blocks)| ParsedPage {
            frontmatter,
            properties,
            blocks,
            warnings: Vec::new(),
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn roundtrip_preserves_semantics(page in arb_page()) {
        let md = render(&page);
        let reparsed = parse(&md);
        prop_assert_eq!(page, reparsed);
    }
}
