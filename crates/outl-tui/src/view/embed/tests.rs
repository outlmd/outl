//! Tests for the embed expansion in the parent module.
//!
//! Their own file because `embed.rs` now carries both embed spellings
//! and the tests are the half that keeps growing — the same split
//! `view::outline` made at the file-size ratchet.

use super::*;
use crate::theme::default_theme;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use outl_md::block_index::IdentifiedNode;
use outl_md::index::PageEntry;
use outl_md::parse::OutlineNode as Node;
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn embed_only_handle_detects_bare_token() {
    assert_eq!(embed_only_handle("!((blk-r6s4a1))"), Some("blk-r6s4a1"));
}

#[test]
fn embed_only_handle_ignores_surrounding_whitespace() {
    assert_eq!(embed_only_handle("  !((blk-r6s4a1))  "), Some("blk-r6s4a1"));
}

#[test]
fn embed_only_handle_rejects_mixed_text() {
    assert_eq!(embed_only_handle("see !((blk-r6s4a1)) context"), None);
}

#[test]
fn embed_only_handle_rejects_inline_ref() {
    // `((blk-X))` (no leading `!`) is a ref, not an embed —
    // must not trigger expansion.
    assert_eq!(embed_only_handle("((blk-r6s4a1))"), None);
}

#[test]
fn embed_only_handle_rejects_two_embeds_on_one_block() {
    // Two embeds in the same block is ambiguous (which one expands
    // first?) — for now keeps the rule strict: exactly one token,
    // surrounded by whitespace.
    assert_eq!(embed_only_handle("!((blk-aaaaaa)) !((blk-bbbbbb))"), None);
}

// ---- `![[page]]` (issue #190) ----------------------------------

#[test]
fn page_embed_only_name_detects_bare_token() {
    assert_eq!(page_embed_only_name("![[Notes]]"), Some("Notes"));
}

#[test]
fn page_embed_only_name_ignores_surrounding_whitespace() {
    assert_eq!(page_embed_only_name("  ![[Notes]]  "), Some("Notes"));
}

#[test]
fn page_embed_only_name_rejects_mixed_text() {
    // Mirror of `embed_only_handle_rejects_mixed_text`: prose
    // around the token keeps the inline chip, no expansion.
    assert_eq!(page_embed_only_name("see ![[Notes]] context"), None);
}

#[test]
fn page_embed_only_name_rejects_a_plain_page_ref() {
    // `[[Notes]]` (no leading `!`) is a link, not an embed.
    assert_eq!(page_embed_only_name("[[Notes]]"), None);
}

#[test]
fn page_embed_only_name_rejects_two_page_embeds_on_one_block() {
    assert_eq!(page_embed_only_name("![[One]] ![[Two]]"), None);
}

#[test]
fn the_two_bare_embed_predicates_never_both_answer() {
    // They share a call site that asks one then the other, so a
    // text answering both would make the order load-bearing.
    for text in [
        "![[Notes]]",
        "!((blk-r6s4a1))",
        "![[Notes]] !((blk-r6s4a1))",
        "plain",
    ] {
        assert!(
            page_embed_only_name(text).is_none() || embed_only_handle(text).is_none(),
            "{text:?} answered both predicates",
        );
    }
}

// ---- `![[page]]`: resolution, chip and expansion ----------------

fn node(text: &str, children: Vec<IdentifiedNode>) -> IdentifiedNode {
    IdentifiedNode {
        id: NodeId::new(),
        text: text.to_string(),
        properties: Vec::new(),
        children,
    }
}

/// An index holding exactly one page: slug `slug`, `title::`
/// `title`, and `roots` as its top-level blocks. Built through the
/// tree-side population path so the fixture needs no `.md` and no
/// sidecar on disk.
fn index_with_page(
    slug: &str,
    title: &str,
    icon: Option<&str>,
    roots: Vec<IdentifiedNode>,
) -> WorkspaceIndex {
    let path = PathBuf::from(format!("pages/{slug}.md"));
    let mut idx = WorkspaceIndex::default();
    idx.insert_page(PageEntry {
        path: path.clone(),
        slug: slug.to_string(),
        title: title.to_string(),
        icon: icon.map(str::to_string),
        is_journal: false,
        pinned: false,
        page_type: None,
        aliases: Vec::new(),
    });
    idx.collect_page_blocks_from_tree(slug, &path, &roots);
    idx
}

fn app_with(index: WorkspaceIndex, blocks: Vec<&str>) -> (App, TempDir) {
    let dir = TempDir::new().unwrap();
    let actor = ActorId::new();
    let ws = Workspace::open_in_memory(actor).unwrap();
    let mut app =
        App::new_for_tests(dir.path().to_path_buf(), ws, actor, default_theme(), false).unwrap();
    app.index = index;
    app.page.blocks = blocks
        .into_iter()
        .map(|t| Node {
            text: t.to_string(),
            properties: Vec::new(),
            children: Vec::new(),
        })
        .collect();
    app.flat_len = app.page.blocks.len();
    app.selected = 0;
    (app, dir)
}

fn joined(spans: &[Span<'static>]) -> String {
    spans.iter().map(|s| s.content.as_ref()).collect()
}

fn rows(lines: &[Line<'static>]) -> Vec<String> {
    lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect()
}

#[test]
fn a_resolvable_page_renders_its_title() {
    let idx = index_with_page("os-linux", "os/linux", None, vec![node("kernel", vec![])]);
    let chip = page_embed_chip("os/linux", &default_theme(), &idx);
    assert_eq!(joined(&chip), "↳ os/linux");
}

#[test]
fn a_resolvable_page_leads_with_its_icon() {
    let idx = index_with_page("notes", "Notes", Some("📓"), vec![]);
    let chip = page_embed_chip("Notes", &default_theme(), &idx);
    assert_eq!(joined(&chip), "📓 ↳ Notes");
}

/// An unknown name keeps the literal visible, exactly like an
/// orphan `!((blk-XXXXXX))` handle. A blank row would tell the
/// user nothing and leave `outl doctor` nothing to point at.
#[test]
fn an_unresolvable_page_name_renders_the_literal_chip() {
    let idx = WorkspaceIndex::default();
    let chip = page_embed_chip("Nope", &default_theme(), &idx);
    assert_eq!(joined(&chip), "![[Nope]]");
}

/// The cursor-bearing render has to reproduce the source byte for
/// byte, or the visible cursor lands on the wrong character while
/// the user edits the token.
#[test]
fn the_cursor_bearing_render_reproduces_the_source() {
    let raw = page_embed_raw("os/linux", &default_theme());
    assert_eq!(joined(&raw), "![[os/linux]]");
}

#[test]
fn page_embed_roots_reads_the_pages_top_level_blocks() {
    let idx = index_with_page(
        "notes",
        "Notes",
        None,
        vec![
            node("first", vec![node("first child", vec![])]),
            node("second", vec![]),
        ],
    );
    let roots = page_embed_roots(&idx, "Notes");
    assert_eq!(
        roots.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(),
        vec!["first", "second"],
        "top-level blocks must come back in document order",
    );
    assert_eq!(roots[0].children.len(), 1, "subtrees ride along");
}

#[test]
fn page_embed_roots_is_empty_for_an_unknown_page() {
    let idx = WorkspaceIndex::default();
    assert!(page_embed_roots(&idx, "Nope").is_empty());
}

#[test]
fn a_bare_page_embed_expands_the_pages_blocks() {
    let idx = index_with_page(
        "notes",
        "Notes",
        None,
        vec![node("first", vec![]), node("second", vec![])],
    );
    let (app, _dir) = app_with(idx, vec![]);
    let mut out = Vec::new();
    emit_block_embed("![[Notes]]", 0, &app, &mut out, 0);
    let painted = rows(&out);
    assert_eq!(painted.len(), 2, "expected both blocks, got {painted:?}");
    assert!(painted[0].contains("first"), "{painted:?}");
    assert!(painted[1].contains("second"), "{painted:?}");
    assert!(
        painted.iter().all(|r| r.contains('↳')),
        "every borrowed row carries the ↳ marker: {painted:?}",
    );
}

/// Mirror of `embed_only_handle_rejects_mixed_text` on the render
/// side: prose around the token keeps the inline chip and expands
/// nothing underneath.
#[test]
fn a_page_embed_mixed_with_prose_expands_nothing() {
    let idx = index_with_page("notes", "Notes", None, vec![node("first", vec![])]);
    let (app, _dir) = app_with(idx, vec![]);
    let mut out = Vec::new();
    emit_block_embed("see ![[Notes]] context", 0, &app, &mut out, 0);
    assert!(out.is_empty(), "mixed prose expanded: {:?}", rows(&out));
}

#[test]
fn an_unresolvable_page_embed_expands_nothing() {
    let (app, _dir) = app_with(WorkspaceIndex::default(), vec![]);
    let mut out = Vec::new();
    emit_block_embed("![[Nope]]", 0, &app, &mut out, 0);
    assert!(out.is_empty(), "{:?}", rows(&out));
}

/// The depth cap is the guard (see the module doc). A page nested
/// deeper than [`EMBED_MAX_DEPTH`] is truncated instead of
/// flooding the outline — and because the cap counts from the
/// carrying row, the deepest row drawn is `EMBED_MAX_DEPTH` levels
/// below the page's top-level blocks' parent.
#[test]
fn the_depth_cap_truncates_a_deep_page() {
    // A single chain `d1 → d2 → … → d7`, deeper than the cap.
    let mut chain = node("d7", vec![]);
    for level in (1..7).rev() {
        chain = node(&format!("d{level}"), vec![chain]);
    }
    let idx = index_with_page("deep", "Deep", None, vec![chain]);
    let (app, _dir) = app_with(idx, vec![]);
    let mut out = Vec::new();
    emit_block_embed("![[Deep]]", 0, &app, &mut out, 0);
    let painted = rows(&out);
    assert_eq!(
        painted.len(),
        EMBED_MAX_DEPTH as usize,
        "expected exactly the capped depth, got {painted:?}",
    );
    assert!(
        painted.iter().any(|r| r.contains("d4")),
        "the last level inside the cap must be drawn: {painted:?}",
    );
    assert!(
        !painted.iter().any(|r| r.contains("d5")),
        "past the cap must be dropped: {painted:?}",
    );
}

/// The pure tests above pass whether or not `render_block` calls
/// `emit_block_embed`, so this is what pins the wiring in
/// `view::outline`.
#[test]
fn the_outline_renderer_expands_a_bare_page_embed() {
    let idx = index_with_page("notes", "Notes", None, vec![node("borrowed", vec![])]);
    // The embed block is second so the cursor (on block 0) doesn't
    // put it in a raw render — the chip is what we want to read.
    let (app, _dir) = app_with(idx, vec!["own block", "![[Notes]]"]);
    let (lines, _, _) = crate::view::outline::render_outline(&app.page, &app, 0);
    let painted = rows(&lines);
    assert!(
        painted.iter().any(|r| r.contains("↳ Notes")),
        "the carrying row must show the page title: {painted:?}",
    );
    assert!(
        painted.iter().any(|r| r.contains("borrowed")),
        "the outline renderer never expanded the page: {painted:?}",
    );
}

// `page_embed_chip` / `page_embed_raw` are tested above on their own,
// and those tests pass whether or not `view::inline`'s two match arms
// call them. These two pin the wiring, which is why they live beside
// the functions rather than in `inline.rs` — that file sits at the
// file-size ratchet.

/// Prose around the token keeps the inline chip: the reader still
/// sees which page is meant, even though nothing expands below.
#[test]
fn the_pretty_render_shows_the_title_inside_prose() {
    let idx = index_with_page("notes", "Notes", None, vec![node("borrowed", vec![])]);
    let spans = crate::view::inline::render_markdown_inline(
        "see ![[Notes]] context",
        &default_theme(),
        &idx,
        &crate::icons::IconSet::default(),
    );
    assert_eq!(joined(&spans), "see ↳ Notes context");
}

/// The cursor-bearing render has to reproduce the source byte for
/// byte — a glyph or a title here would put the visible cursor on the
/// wrong character while the user edits the token.
#[test]
fn the_cursor_bearing_render_keeps_the_source_verbatim() {
    let spans = crate::view::inline::highlight_inline("see ![[Notes]] here", &default_theme());
    assert_eq!(joined(&spans), "see ![[Notes]] here");
}
