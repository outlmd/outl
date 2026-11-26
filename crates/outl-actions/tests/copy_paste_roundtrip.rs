//! `copy_markdown` -> `paste_markdown` is a fixpoint on real-shaped pages.
//!
//! The source tree is built from a `.md` through `outl_md::parse` and the
//! plain block / property actions, never through the paste path, so the
//! paste under test is not also its own oracle. Three things are pinned
//! per page:
//!
//! 1. the pasted subtree has the same text, properties and shape as the
//!    copied one (compared as the fragment each side projects, and as raw
//!    block text);
//! 2. a second copy of the pasted subtree is byte-identical to the first
//!    (copy and paste are inverses, not merely close);
//! 3. the destination page renders -> parses -> renders to itself, and no
//!    rendered content line is missing from the logged block texts (the
//!    `unlogged` guard would otherwise refuse the file).

use outl_actions::{
    append_block, children_of, copy_markdown, open_or_create_page, paste_markdown, render_page_md,
    set_property, PageKind, PasteAnchor,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;
use outl_md::parse::{parse, OutlineNode};

fn ws() -> (Workspace, HlcGenerator) {
    let actor = ActorId::new();
    (
        Workspace::open_in_memory(actor).expect("workspace"),
        HlcGenerator::new(actor),
    )
}

/// What the importer stores: a lone `[[x]]` / `#x` is a typed value.
fn typed(value: &str) -> PropValue {
    if let Some(name) = value.strip_prefix("[[").and_then(|v| v.strip_suffix("]]")) {
        if !name.contains("]]") {
            return PropValue::PageRef(name.to_string());
        }
    }
    if let Some(tag) = value.strip_prefix('#') {
        if !tag.is_empty() && !tag.contains(char::is_whitespace) {
            return PropValue::Tag(tag.to_string());
        }
    }
    PropValue::Text(value.to_string())
}

fn materialize(ws: &mut Workspace, hlc: &HlcGenerator, parent: NodeId, nodes: &[OutlineNode]) {
    for n in nodes {
        let id = append_block(ws, hlc, Some(parent), Some(&n.text)).expect("block");
        for (k, v) in &n.properties {
            set_property(ws, hlc, id, k, Some(typed(v))).expect("prop");
        }
        materialize(ws, hlc, id, &n.children);
    }
}

fn load(md: &str) -> (Workspace, HlcGenerator, NodeId) {
    let (mut ws, hlc) = ws();
    let page = open_or_create_page(&mut ws, &hlc, "src", "src", PageKind::Page).expect("page");
    let parsed = parse(md);
    assert!(
        !parsed.blocks.is_empty(),
        "fixture parsed to nothing:\n{md}"
    );
    materialize(&mut ws, &hlc, page, &parsed.blocks);
    (ws, hlc, page)
}

fn roots(ws: &Workspace, page: NodeId) -> Vec<NodeId> {
    children_of(ws, page)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

fn texts(ws: &Workspace, ids: &[NodeId], out: &mut Vec<String>) {
    for &id in ids {
        out.push(ws.block_text(id).unwrap_or_default());
        let kids = roots(ws, id);
        texts(ws, &kids, out);
    }
}

fn check(name: &str, md: &str) {
    let (mut ws, hlc, src) = load(md);
    let src_roots = roots(&ws, src);
    let copied = copy_markdown(&ws, &src_roots);

    let dst = open_or_create_page(&mut ws, &hlc, "dst", "dst", PageKind::Page).expect("dst");
    let out = paste_markdown(&mut ws, &hlc, PasteAnchor::AsLastChildOf(dst), &copied)
        .unwrap_or_else(|e| panic!("[{name}] paste failed: {e}"));
    assert!(out.last_root.is_some(), "[{name}] paste created nothing");

    let dst_roots = roots(&ws, dst);
    assert_eq!(
        dst_roots.len(),
        src_roots.len(),
        "[{name}] root count\n{copied}"
    );

    let (mut a, mut b) = (Vec::new(), Vec::new());
    texts(&ws, &src_roots, &mut a);
    texts(&ws, &dst_roots, &mut b);
    assert_eq!(a, b, "[{name}] block texts diverged\ncopied:\n{copied}");

    let recopied = copy_markdown(&ws, &dst_roots);
    assert_eq!(copied, recopied, "[{name}] copy(paste(copy)) != copy");

    let rendered = render_page_md(&ws, dst);
    let reparsed = parse(&rendered);
    let again = outl_md::render::render(&reparsed);
    assert_eq!(
        rendered, again,
        "[{name}] page render is not a parse fixpoint"
    );
    let missing = outl_md::unlogged::content_lines_missing_from_texts(
        &rendered,
        b.iter().map(String::as_str),
    );
    assert!(
        missing.is_empty(),
        "[{name}] unlogged lines: {missing:?}\n{rendered}"
    );
    let mut c = Vec::new();
    let reparsed_roots: Vec<String> = reparsed.blocks.iter().map(|n| n.text.clone()).collect();
    texts(&ws, &dst_roots, &mut c);
    assert_eq!(
        reparsed_roots,
        dst_roots
            .iter()
            .map(|&id| ws.block_text(id).unwrap_or_default())
            .collect::<Vec<_>>(),
        "[{name}] reparsed roots differ from the tree"
    );
}

#[test]
fn fences_with_syntax_that_normalize_would_rewrite() {
    check(
        "fence-syntax",
        "- setup\n  - ```clojure\n    {{[[TODO]]}} not a task\n    - [ ] not a checkbox\n    id:: abc-123\n    [[April 22nd, 2026]]\n    ```\n  - after\n",
    );
}

#[test]
fn fence_inside_fence_levels_and_unclosed_inner_fence() {
    check(
        "nested-fence",
        "- doc\n  - ```md\n    ~~~js\n    {{video: x}}\n    ~~~\n    - [x] done\n    ```\n  - ~~~\n    {{[[DONE]]}}\n    ~~~\n  - tail\n",
    );
}

#[test]
fn nested_children_and_properties_with_refs_and_tags() {
    check(
        "props",
        "- TODO ship it\n  who:: [[Avelino]]\n  kind:: #meeting\n  note:: plain value\n  - DOING child\n    due:: [[2026-11-30]]\n    - DONE grandchild\n      priority:: high\n- second root\n  tags:: #a #b\n",
    );
}

#[test]
fn multi_line_text_quotes_and_tables() {
    check(
        "multiline",
        "- first line\n  continued line\n  third\n- > quoted\n- | a | b |\n  | - | - |\n  | 1 | 2 |\n- after table\n",
    );
}

#[test]
fn task_markers_and_odd_text() {
    check(
        "markers",
        "- TODO a\n- DOING b\n- DONE c\n- LATER d\n- [[page]] and #tag mid-line\n- text with\ttab inside\n- ==hl== and **bold**\n",
    );
}

#[test]
fn fence_with_blank_lines_and_trailing_blank_block() {
    check(
        "blank-in-fence",
        "- ```sh\n  echo a\n\n  echo b\n  ```\n- next\n",
    );
}

#[test]
fn single_block_with_children_and_props_alone() {
    check(
        "single",
        "- only\n  k:: v\n  - child one\n  - child two\n    - deep\n",
    );
}

#[test]
fn dashes_and_fragments_that_look_like_structure() {
    check(
        "dashes",
        "- ---\n- before\n- ```\n  - [ ] x\n  ---\n  ```\n",
    );
}

/// Not a roundtrip: copy -> paste deliberately rewrites foreign syntax that
/// sits in an outl block's text outside a fence. Pinned so a change to it is
/// a decision (the same bytes inside a fence are covered above).
#[test]
fn foreign_syntax_outside_a_fence_is_normalized_on_paste() {
    let (mut ws, hlc, src) = load("- a {{[[DONE]]}} ^^hl^^\n- [x] boxed\n");
    let copied = copy_markdown(&ws, &roots(&ws, src));
    let dst = open_or_create_page(&mut ws, &hlc, "dst", "dst", PageKind::Page).expect("dst");
    paste_markdown(&mut ws, &hlc, PasteAnchor::AsLastChildOf(dst), &copied).expect("paste");
    let mut got = Vec::new();
    texts(&ws, &roots(&ws, dst), &mut got);
    assert_eq!(got, ["a DONE ==hl==", "DONE boxed"]);
}
