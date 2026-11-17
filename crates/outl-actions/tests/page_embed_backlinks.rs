//! `![[page]]` is a page reference, so it produces a backlink — issue
//! #190.
//!
//! The contract for the page embed says so in one line ("backlink from
//! the embedding block to the page (it IS a page reference)"), and the
//! reason it needs a test rather than a comment is that the embed is the
//! *strongest* kind of mention a block can make: transcluding a whole
//! page is a harder statement than linking to it. If the embed were
//! inert, the one place a user pulls a page into another page would be
//! the one place that page learns nothing about it.
//!
//! Asserted on **both** builders — the workspace walk
//! (`build_backlink_index`) and the from-disk walk
//! (`build_backlink_index_from_disk`) — for the reason
//! `namespace_backlinks.rs` states: a rule that holds on one and not the
//! other reaches the user as "it works in the TUI but not on the
//! desktop". Both route through `backlinks_keys::mentions_of`, which is
//! exactly the single owner this would break if someone later made
//! `mentions::collect` treat `PageEmbed` as a wrapper.

use std::path::Path;

use outl_actions::{
    append_block, apply_page_md_with_sidecar, build_backlink_index, build_backlink_index_from_disk,
    extract_refs, find_by_slug, list_pages, open_or_create_page, page_meta, PageKind, PageMeta,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use tempfile::TempDir;

/// A workspace with one page per `(slug, block text)` pair, projected to
/// `.md` + sidecar and reopened from disk so both index builders see the
/// same content.
fn workspace_with(root: &Path, notes: &[(&str, &str)]) -> (Workspace, Vec<PageMeta>) {
    let ops_dir = root.join("ops");
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    {
        let storage = JsonlStorage::open(ops_dir.clone(), actor).unwrap();
        let mut w =
            Workspace::open_with_storage(actor, Box::new(storage), Some(root.to_path_buf()))
                .unwrap();
        for (slug, text) in notes {
            let page = open_or_create_page(&mut w, &hlc, slug, slug, PageKind::Page).unwrap();
            append_block(&mut w, &hlc, Some(page), Some(text)).unwrap();
        }
        for meta in list_pages(&w) {
            let id = find_by_slug(&w, &meta.slug).unwrap();
            apply_page_md_with_sidecar(&w, root, id).unwrap();
        }
    }
    let storage = JsonlStorage::open(ops_dir, actor).unwrap();
    let w =
        Workspace::open_with_storage(actor, Box::new(storage), Some(root.to_path_buf())).unwrap();
    let metas = list_pages(&w);
    (w, metas)
}

/// Texts of the blocks that back-link to `slug`, under both builders,
/// asserted to agree. Sorted so the assertion doesn't depend on walk
/// order.
fn backlink_texts(w: &Workspace, metas: &[PageMeta], root: &Path, slug: &str) -> Vec<String> {
    let id = find_by_slug(w, slug).expect("page exists");
    let meta = page_meta(w, id).expect("page meta");
    let mut from_tree: Vec<String> = build_backlink_index(w, root)
        .for_page(w, &meta)
        .into_iter()
        .map(|b| b.block_text)
        .collect();
    let mut from_disk: Vec<String> = build_backlink_index_from_disk(metas, root)
        .for_page(w, &meta)
        .into_iter()
        .map(|b| b.block_text)
        .collect();
    from_tree.sort();
    from_disk.sort();
    assert_eq!(
        from_tree, from_disk,
        "the workspace and from-disk builders disagree about `{slug}`"
    );
    from_tree
}

#[test]
fn a_page_embed_backlinks_the_page_it_embeds() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let (w, metas) = workspace_with(
        root,
        &[
            ("roadmap", "the embedded page"),
            ("weekly", "this week: ![[roadmap]]"),
        ],
    );

    assert_eq!(
        backlink_texts(&w, &metas, root, "roadmap"),
        vec!["this week: ![[roadmap]]".to_string()],
        "`![[roadmap]]` must reach the `roadmap` page"
    );
}

/// The embed is not a *weaker* mention than the link, so the two are
/// collected the same way and a block holding both is one backlink, not
/// two of different kinds.
#[test]
fn an_embed_and_a_link_to_the_same_page_are_both_mentions() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let (w, metas) = workspace_with(
        root,
        &[
            ("roadmap", "the embedded page"),
            ("linked", "see [[roadmap]]"),
            ("embedded", "![[roadmap]]"),
        ],
    );

    let mut hits = backlink_texts(&w, &metas, root, "roadmap");
    hits.sort();
    assert_eq!(
        hits,
        vec!["![[roadmap]]".to_string(), "see [[roadmap]]".to_string()]
    );
}

/// Root `CLAUDE.md` invariant 2: the embed is expanded at render time,
/// so the `.md` carries the literal `![[roadmap]]` and never a copy of
/// the target page's text.
#[test]
fn the_md_keeps_the_literal_embed_and_never_the_targets_text() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let (_w, _metas) = workspace_with(
        root,
        &[
            ("roadmap", "ship the thing"),
            ("weekly", "this week: ![[roadmap]]"),
        ],
    );

    let weekly = std::fs::read_to_string(root.join("pages/weekly.md")).unwrap();
    assert!(weekly.contains("![[roadmap]]"), "{weekly}");
    assert!(
        !weekly.contains("ship the thing"),
        "the target's text must not be projected into the embedding page: {weekly}"
    );
}

/// The mention channel itself, without the index: `extract_refs` is what
/// `mentions_of` reads, and it is the function whose exhaustive `match`
/// forced this verdict to be made.
#[test]
fn extract_refs_reads_a_page_embed_as_a_ref() {
    assert_eq!(extract_refs("a ![[one]] b [[two]]"), vec!["one", "two"]);
    // A code span is opaque to both spellings, and the `!` alone is not
    // a mention of anything.
    assert!(extract_refs("`![[one]]`").is_empty());
}
