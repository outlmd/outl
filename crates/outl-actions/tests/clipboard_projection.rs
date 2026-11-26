//! `copy_markdown` reads the property map once per selection, and says
//! what it said before.
//!
//! It used to build each node with a per-node property read
//! (`Tree::properties_of`, which filters the whole workspace-wide map
//! on every call), so a Visual range of N blocks with M descendants paid
//! `O((N + M) × properties)` synchronously under the workspace lock.
//! It now projects through `journal::render::project_blocks`, the same
//! projection `render_block_md` uses, off one `group_properties_of`
//! pass.
//!
//! Invariant 15 asks a performance fix for three tests:
//!
//! - *does it still do the right thing* — `the_projection_matches_the_per_node_reader`
//!   pins the new output against the old algorithm, re-implemented here
//!   over the public API, on text properties (the only kind the old one
//!   kept);
//! - *is it still cheap* — `clipboard.rs` is in `outline_property_pass.rs`'s
//!   ratchet, which fails on any `properties_of` ident in that file;
//! - *is it cheap once* — `copy_markdown` hands the whole selection to
//!   one `project_blocks` call; there is no per-root loop around it to
//!   count.
//!
//! The deliberate behaviour change is pinned too: `PageRef` / `Tag`
//! values now survive a copy, because `render_block_md` always kept
//! them and `yy` / `Cmd+C` must write the same bytes.

use outl_actions::page::SLUG_KEY;
use outl_actions::tree::{is_page_model_key, sort_in_presentation_order};
use outl_actions::{
    append_block, children_of, copy_markdown, copy_markdown_nodes, open_or_create_page,
    render_block_md, set_property, PageKind,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;
use outl_md::parse::OutlineNode;

fn ws() -> (Workspace, HlcGenerator) {
    let actor = ActorId::new();
    (
        Workspace::open_in_memory(actor).expect("in-memory workspace"),
        HlcGenerator::new(actor),
    )
}

fn block(ws: &mut Workspace, hlc: &HlcGenerator, parent: NodeId, text: &str) -> NodeId {
    append_block(ws, hlc, Some(parent), Some(text)).expect("block is created")
}

fn text_prop(ws: &mut Workspace, hlc: &HlcGenerator, node: NodeId, key: &str, value: &str) {
    set_property(ws, hlc, node, key, Some(PropValue::Text(value.to_string())))
        .expect("property is set");
}

/// The serializer as it was: per-node property read, `Text` only, page
/// model keys hidden on every node. Kept here, not in `src/`, so the
/// ratchet keeps the per-node read out of the crate.
fn per_node_reference(ws: &Workspace, roots: &[NodeId]) -> String {
    fn node(ws: &Workspace, id: NodeId) -> OutlineNode {
        let mut properties: Vec<(String, String)> = ws
            .tree()
            .properties_of(id)
            .filter(|(k, _)| !is_page_model_key(k))
            .filter_map(|(k, v)| match v {
                PropValue::Text(s) => Some((k.to_string(), s.clone())),
                _ => None,
            })
            .collect();
        sort_in_presentation_order(&mut properties);
        OutlineNode {
            text: ws.block_text(id).unwrap_or_default(),
            properties,
            children: children_of(ws, id)
                .into_iter()
                .map(|(child, _)| node(ws, child))
                .collect(),
        }
    }
    let nodes: Vec<OutlineNode> = roots.iter().map(|&id| node(ws, id)).collect();
    copy_markdown_nodes(&nodes)
}

/// A page with two roots, nested children three deep, text properties
/// at every level and in reverse alphabetical order, task and quote
/// markers, plus a block outside the selection carrying properties of
/// its own (the pass is scoped, so those must not leak in).
fn fixture() -> (Workspace, Vec<NodeId>) {
    let (mut ws, hlc) = ws();
    let page = open_or_create_page(&mut ws, &hlc, "copy", "copy", PageKind::Page).expect("page");
    let a = block(&mut ws, &hlc, page, "TODO first root");
    text_prop(&mut ws, &hlc, a, "zeta", "1");
    text_prop(&mut ws, &hlc, a, "alpha", "2");
    let a1 = block(&mut ws, &hlc, a, "> quoted child");
    text_prop(&mut ws, &hlc, a1, "owner", "me");
    let a11 = block(&mut ws, &hlc, a1, "grandchild");
    text_prop(&mut ws, &hlc, a11, "deep", "yes");
    block(&mut ws, &hlc, a, "second child");
    let b = block(&mut ws, &hlc, page, "DONE second root");
    text_prop(&mut ws, &hlc, b, "when", "today");
    let outside = block(&mut ws, &hlc, page, "not selected");
    text_prop(&mut ws, &hlc, outside, "leak", "no");
    (ws, vec![a, b])
}

#[test]
fn the_projection_matches_the_per_node_reader() {
    let (ws, roots) = fixture();
    let expected = per_node_reference(&ws, &roots);
    assert!(
        expected.contains("deep:: yes") && expected.contains("alpha:: 2"),
        "the fixture has to exercise properties at depth, or this proves nothing:\n{expected}"
    );
    assert_eq!(copy_markdown(&ws, &roots), expected);
}

#[test]
fn one_block_copies_the_same_bytes_as_render_block_md() {
    let (ws, roots) = fixture();
    for root in roots {
        assert_eq!(copy_markdown(&ws, &[root]), render_block_md(&ws, root));
    }
}

/// The behaviour change, stated: a `PageRef` / `Tag` value is a string
/// the `.md` dialect can spell, so a copy keeps it like the renderer
/// always did. A `List` has no syntax and stays out of both.
#[test]
fn page_ref_and_tag_values_survive_a_copy_and_a_list_does_not() {
    let (mut ws, hlc) = ws();
    let page = open_or_create_page(&mut ws, &hlc, "p", "p", PageKind::Page).expect("page");
    let b = block(&mut ws, &hlc, page, "with refs");
    for (key, value) in [
        ("who", PropValue::PageRef("Avelino".into())),
        ("kind", PropValue::Tag("meeting".into())),
        (
            "many",
            PropValue::List(vec![
                PropValue::Text("x".into()),
                PropValue::Text("y".into()),
            ]),
        ),
    ] {
        set_property(&mut ws, &hlc, b, key, Some(value)).expect("property is set");
    }

    let copied = copy_markdown(&ws, &[b]);
    assert_eq!(copied, "- with refs\n  kind:: meeting\n  who:: Avelino\n");
    assert_eq!(copied, render_block_md(&ws, b));
}

/// Copying a page root must not leak `page-slug` / `page-kind` into the
/// clipboard: pasted back, they would make an ordinary block claim a
/// slug. The user's own page properties ride along.
#[test]
fn a_selected_page_root_leaves_its_book_keeping_behind() {
    let (mut ws, hlc) = ws();
    let page = open_or_create_page(&mut ws, &hlc, "pg", "pg", PageKind::Page).expect("page");
    text_prop(&mut ws, &hlc, page, "icon", "x");
    block(&mut ws, &hlc, page, "child");
    assert!(ws.tree().property(page, SLUG_KEY).is_some());

    let copied = copy_markdown(&ws, &[page]);
    assert!(!copied.contains("page-slug"), "{copied}");
    assert!(!copied.contains("page-kind"), "{copied}");
    assert!(copied.contains("icon:: x"), "{copied}");
    assert!(copied.contains("  - child"), "{copied}");
}
