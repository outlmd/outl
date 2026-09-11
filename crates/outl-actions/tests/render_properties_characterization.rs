//! Characterization of the property side of the `tree → .md`
//! projection.
//!
//! `journal::render` used to ask [`outl_core::tree::Tree::properties_of`]
//! once per node, which re-filters the workspace-wide property map on
//! every call. Replacing that with a single grouped pass over
//! `Tree::iter_properties()` is a pure performance change — and the
//! thing most likely to go wrong silently is **order**, because both
//! sources iterate a `HashMap`.
//!
//! Property lines that reorder are a diff in every page on the next
//! projection, so these tests pin the exact rendered bytes rather than
//! asserting "contains".

use outl_actions::{
    append_block, open_or_create_page, render_block_md, render_page_md, set_property, PageKind,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;

fn workspace() -> (Workspace, HlcGenerator) {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let ws = Workspace::open_in_memory(actor).expect("in-memory workspace opens");
    (ws, hlc)
}

fn text(value: &str) -> Option<PropValue> {
    Some(PropValue::Text(value.to_string()))
}

fn block(ws: &mut Workspace, hlc: &HlcGenerator, parent: NodeId, body: &str) -> NodeId {
    append_block(ws, hlc, Some(parent), Some(body)).expect("append_block succeeds")
}

/// The whole surface in one page, pinned byte for byte:
///
/// * a block with several properties (order is alpha on the key),
/// * properties on a nested block (the recursion carries them),
/// * a block with no properties at all,
/// * a `PropValue::List`, which has no `.md` render syntax and is
///   dropped — a block holding only a list renders bare,
/// * unicode in both key and value,
/// * two spellings of one key differing only in case, which the tree
///   stores as two distinct properties and the renderer must emit as
///   two lines in byte order (`ASCII` uppercase sorts before lowercase).
#[test]
fn render_page_md_property_projection_is_byte_stable() {
    let (mut ws, hlc) = workspace();
    let page =
        open_or_create_page(&mut ws, &hlc, "props", "Props", PageKind::Page).expect("page opens");

    // Page-level: a user property, a unicode key, and the two
    // book-keeping keys the page model owns (which must not render).
    set_property(&mut ws, &hlc, page, "icon", text("🦀")).unwrap();
    set_property(&mut ws, &hlc, page, "área", text("núcleo")).unwrap();
    set_property(&mut ws, &hlc, page, "type", text("person")).unwrap();

    // Multiple properties on one block, inserted in non-alpha order.
    let multi = block(&mut ws, &hlc, page, "multi");
    set_property(&mut ws, &hlc, multi, "zeta", text("last")).unwrap();
    set_property(&mut ws, &hlc, multi, "alpha", text("first")).unwrap();
    set_property(&mut ws, &hlc, multi, "mid", text("middle")).unwrap();

    // A block with no properties, carrying a nested block that has some.
    let bare = block(&mut ws, &hlc, page, "bare");
    let nested = block(&mut ws, &hlc, bare, "nested");
    set_property(&mut ws, &hlc, nested, "depth", text("2")).unwrap();
    set_property(&mut ws, &hlc, nested, "чей", text("наш")).unwrap();

    // `List` has no render syntax; this block must come out bare.
    let listed = block(&mut ws, &hlc, page, "listed");
    set_property(
        &mut ws,
        &hlc,
        listed,
        "tags",
        Some(PropValue::List(vec![
            PropValue::Text("a".into()),
            PropValue::Text("b".into()),
        ])),
    )
    .unwrap();

    // Same key, two casings: distinct properties in the tree, so two
    // rendered lines. Also a `PageRef` and a `Tag`, which render as
    // their string form.
    let cased = block(&mut ws, &hlc, page, "cased");
    set_property(&mut ws, &hlc, cased, "Remind", text("3pm")).unwrap();
    set_property(&mut ws, &hlc, cased, "remind", text("4pm")).unwrap();
    set_property(
        &mut ws,
        &hlc,
        cased,
        "rel",
        Some(PropValue::PageRef("[[other]]".into())),
    )
    .unwrap();
    set_property(
        &mut ws,
        &hlc,
        cased,
        "topic",
        Some(PropValue::Tag("#rust".into())),
    )
    .unwrap();

    // A block whose only property is page-model book-keeping: on a
    // *block* those are ordinary user properties and must render.
    let bookkeeping = block(&mut ws, &hlc, page, "bookkeeping");
    set_property(&mut ws, &hlc, bookkeeping, "page-slug", text("not-a-page")).unwrap();

    let md = render_page_md(&ws, page);
    assert_eq!(
        md,
        concat!(
            "icon:: 🦀\n",
            "title:: Props\n",
            "type:: person\n",
            "área:: núcleo\n",
            "\n",
            "- multi\n",
            "  alpha:: first\n",
            "  mid:: middle\n",
            "  zeta:: last\n",
            "- bare\n",
            "  - nested\n",
            "    depth:: 2\n",
            "    чей:: наш\n",
            "- listed\n",
            "- cased\n",
            "  Remind:: 3pm\n",
            "  rel:: [[other]]\n",
            "  remind:: 4pm\n",
            "  topic:: #rust\n",
            "- bookkeeping\n",
            "  page-slug:: not-a-page\n",
        ),
        "rendered page bytes drifted:\n{md}"
    );
}

/// Rendering the same workspace twice must produce identical bytes.
/// Both property sources iterate a `HashMap`, whose order is seeded per
/// process but stable within one — so this catches a sort that was
/// dropped, not one that merely moved.
#[test]
fn render_page_md_is_stable_across_repeated_calls() {
    let (mut ws, hlc) = workspace();
    let page =
        open_or_create_page(&mut ws, &hlc, "many", "Many", PageKind::Page).expect("page opens");
    for i in 0..40 {
        let node = block(&mut ws, &hlc, page, &format!("block {i}"));
        for k in 0..6 {
            set_property(
                &mut ws,
                &hlc,
                node,
                &format!("key{k}"),
                text(&format!("v{i}-{k}")),
            )
            .unwrap();
        }
    }

    let first = render_page_md(&ws, page);
    for _ in 0..5 {
        assert_eq!(render_page_md(&ws, page), first, "render is not idempotent");
    }
}

/// Key order does not depend on insertion order: two workspaces built
/// with the same properties in opposite orders render identically.
#[test]
fn render_page_md_key_order_is_independent_of_insertion_order() {
    let keys = ["delta", "Alpha", "charlie", "bravo", "épsilon", "alpha"];

    let render_with = |order: Vec<&str>| {
        let (mut ws, hlc) = workspace();
        let page = open_or_create_page(&mut ws, &hlc, "ord", "Ord", PageKind::Page).unwrap();
        let node = block(&mut ws, &hlc, page, "body");
        for key in order {
            set_property(&mut ws, &hlc, node, key, text("v")).unwrap();
        }
        render_page_md(&ws, page)
    };

    let forward = render_with(keys.to_vec());
    let backward = render_with(keys.iter().rev().copied().collect());
    assert_eq!(forward, backward, "property order followed insertion order");
    assert_eq!(
        forward,
        concat!(
            "title:: Ord\n",
            "\n",
            "- body\n",
            "  Alpha:: v\n",
            "  alpha:: v\n",
            "  bravo:: v\n",
            "  charlie:: v\n",
            "  delta:: v\n",
            "  épsilon:: v\n",
        ),
        "property order drifted:\n{forward}"
    );
}

/// `render_block_md` shares the same projection, so the property rules
/// (and their order) have to match what the page render emits — that is
/// the whole point of the "copy block reads like the `.md`" contract.
#[test]
fn render_block_md_property_projection_is_byte_stable() {
    let (mut ws, hlc) = workspace();
    let page =
        open_or_create_page(&mut ws, &hlc, "copy", "Copy", PageKind::Page).expect("page opens");

    let src = block(&mut ws, &hlc, page, "src");
    set_property(&mut ws, &hlc, src, "zeta", text("z")).unwrap();
    set_property(&mut ws, &hlc, src, "página", text("sim")).unwrap();
    set_property(&mut ws, &hlc, src, "alpha", text("a")).unwrap();
    // Dropped: no render syntax.
    set_property(
        &mut ws,
        &hlc,
        src,
        "list",
        Some(PropValue::List(vec![PropValue::Text("x".into())])),
    )
    .unwrap();

    let kid = block(&mut ws, &hlc, src, "kid");
    set_property(&mut ws, &hlc, kid, "k", text("v")).unwrap();
    // No properties at all, one level deeper.
    block(&mut ws, &hlc, kid, "grandkid");

    let md = render_block_md(&ws, src);
    assert_eq!(
        md,
        concat!(
            "- src\n",
            "  alpha:: a\n",
            "  página:: sim\n",
            "  zeta:: z\n",
            "  - kid\n",
            "    k:: v\n",
            "    - grandkid\n",
        ),
        "rendered block bytes drifted:\n{md}"
    );
}

/// A node with no properties at all must not gain a line, and a page
/// with only book-keeping properties must render no property header —
/// the empty-input edge of the grouped lookup (a missing map entry has
/// to behave exactly like an empty one).
#[test]
fn render_page_md_emits_nothing_for_nodes_without_properties() {
    let (mut ws, hlc) = workspace();
    let page = open_or_create_page(&mut ws, &hlc, "empty", "", PageKind::Page).expect("page opens");
    // `open_or_create` sets `title::`; clear it so only `page-slug` /
    // `page-kind` remain, and neither of those renders.
    set_property(&mut ws, &hlc, page, "title", None).unwrap();

    let a = block(&mut ws, &hlc, page, "a");
    block(&mut ws, &hlc, a, "b");

    let md = render_page_md(&ws, page);
    assert_eq!(md, "- a\n  - b\n", "bare page grew a property line:\n{md}");
}
