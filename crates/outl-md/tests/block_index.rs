//! End-to-end tests for `BlockIndex`. Moved out of `src/block_index/`
//! to keep that module under the file-size-guard. Every test here
//! exercises only the public API surface — same contract any UI
//! surface (TUI today, Tauri / mobile later) consumes.

use outl_core::id::NodeId;
use outl_md::block_index::BlockIndex;
use outl_md::parse::parse;
use outl_md::sidecar::{derive_ref_handle, SidecarBlock};
use std::path::PathBuf;

fn sb(id: NodeId, text: &str, line: usize, indent: u32) -> SidecarBlock {
    SidecarBlock::from_text(id, line, indent, text)
}

#[test]
fn collect_populates_handle_to_block() {
    let md = "- alpha\n- beta\n";
    let ast = parse(md);
    let id_a = NodeId::new();
    let id_b = NodeId::new();
    let sidecar_blocks = vec![sb(id_a, "alpha", 1, 0), sb(id_b, "beta", 2, 0)];

    let mut idx = BlockIndex::default();
    idx.collect_page(
        "page",
        &PathBuf::from("pages/page.md"),
        &ast.blocks,
        &sidecar_blocks,
    );

    assert_eq!(idx.block_count(), 2);
    let handle_a = derive_ref_handle(id_a);
    let resolved = idx.resolve(&handle_a).expect("resolve a");
    assert_eq!(resolved.id, id_a);
    assert_eq!(resolved.text, "alpha");
}

#[test]
fn reverse_refs_track_citing_blocks() {
    let id_target = NodeId::new();
    let target_handle = derive_ref_handle(id_target);

    let mut idx = BlockIndex::default();
    let md_a = "- decide backend\n";
    let ast_a = parse(md_a);
    let sidecar_a = vec![sb(id_target, "decide backend", 1, 0)];
    idx.collect_page("a", &PathBuf::from("pages/a.md"), &ast_a.blocks, &sidecar_a);

    let id_cite = NodeId::new();
    let md_b = format!("- see (({target_handle})) for context\n");
    let ast_b = parse(&md_b);
    let sidecar_b = vec![sb(
        id_cite,
        &format!("see (({target_handle})) for context"),
        1,
        0,
    )];
    idx.collect_page("b", &PathBuf::from("pages/b.md"), &ast_b.blocks, &sidecar_b);

    let refs = idx.refs_to(id_target);
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].source_slug, "b");
}

#[test]
fn reverse_refs_track_embed_too() {
    let id_target = NodeId::new();
    let target_handle = derive_ref_handle(id_target);

    let mut idx = BlockIndex::default();
    let sidecar_a = vec![sb(id_target, "decide backend", 1, 0)];
    idx.collect_page(
        "a",
        &PathBuf::from("pages/a.md"),
        &parse("- decide backend\n").blocks,
        &sidecar_a,
    );

    let id_cite = NodeId::new();
    let md_b = format!("- !(({target_handle}))\n");
    let ast_b = parse(&md_b);
    let sidecar_b = vec![sb(id_cite, &format!("!(({target_handle}))"), 1, 0)];
    idx.collect_page("b", &PathBuf::from("pages/b.md"), &ast_b.blocks, &sidecar_b);

    let refs = idx.refs_to(id_target);
    assert_eq!(refs.len(), 1, "embed should produce a reverse ref");
}

#[test]
fn forget_page_drops_blocks_handles_and_reverse_refs() {
    let id = NodeId::new();
    let handle = derive_ref_handle(id);
    let md = "- alpha\n";
    let ast = parse(md);
    let sidecar = vec![sb(id, "alpha", 1, 0)];

    let mut idx = BlockIndex::default();
    idx.collect_page("a", &PathBuf::from("pages/a.md"), &ast.blocks, &sidecar);
    assert_eq!(idx.block_count(), 1);
    assert!(idx.resolve(&handle).is_some());

    idx.forget_page("a");
    assert_eq!(idx.block_count(), 0);
    assert!(idx.resolve(&handle).is_none());
}

#[test]
fn at_location_returns_block_for_slug_and_path() {
    let id = NodeId::new();
    let md = "- alpha\n  - beta\n";
    let ast = parse(md);
    let id_beta = NodeId::new();
    let sidecar = vec![sb(id, "alpha", 1, 0), sb(id_beta, "beta", 2, 1)];

    let mut idx = BlockIndex::default();
    idx.collect_page("p", &PathBuf::from("pages/p.md"), &ast.blocks, &sidecar);

    let found = idx.at_location("p", &[0, 0]).expect("nested block");
    assert_eq!(found.id, id_beta);
}

#[test]
fn search_text_ranks_prefix_match_before_middle_match() {
    let mut idx = BlockIndex::default();
    let id_prefix = NodeId::new();
    let id_middle = NodeId::new();
    let md = "- decide backend\n- maybe decide database\n";
    let ast = parse(md);
    let sidecar = vec![
        sb(id_prefix, "decide backend", 1, 0),
        sb(id_middle, "maybe decide database", 2, 0),
    ];
    idx.collect_page("p", &PathBuf::from("pages/p.md"), &ast.blocks, &sidecar);

    let hits = idx.search_text("decide", 5);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, id_prefix, "prefix match must rank first");
    assert_eq!(hits[1].id, id_middle);
}

#[test]
fn search_text_is_case_insensitive() {
    let mut idx = BlockIndex::default();
    let id = NodeId::new();
    let md = "- Decide Backend\n";
    let ast = parse(md);
    let sidecar = vec![sb(id, "Decide Backend", 1, 0)];
    idx.collect_page("p", &PathBuf::from("pages/p.md"), &ast.blocks, &sidecar);

    assert_eq!(idx.search_text("DECIDE", 5).len(), 1);
    assert_eq!(idx.search_text("decide", 5).len(), 1);
    assert_eq!(idx.search_text("backend", 5).len(), 1);
}

#[test]
fn search_text_empty_query_returns_nothing() {
    let mut idx = BlockIndex::default();
    let id = NodeId::new();
    let sidecar = vec![sb(id, "x", 1, 0)];
    idx.collect_page(
        "p",
        &PathBuf::from("pages/p.md"),
        &parse("- x\n").blocks,
        &sidecar,
    );
    assert!(idx.search_text("", 5).is_empty());
}

#[test]
fn nested_blocks_record_correct_dfs_path() {
    let md = "- root\n  - child\n";
    let ast = parse(md);
    let id_root = NodeId::new();
    let id_child = NodeId::new();
    let sidecar = vec![sb(id_root, "root", 1, 0), sb(id_child, "child", 2, 1)];

    let mut idx = BlockIndex::default();
    idx.collect_page("x", &PathBuf::from("pages/x.md"), &ast.blocks, &sidecar);

    let child = idx.get(id_child).expect("child indexed");
    assert_eq!(child.source_block_path, vec![0, 0]);
}

/// Build a (smaller, larger) pair of `NodeId`s. The smaller id is the
/// deterministic winner of any handle collision — see
/// `BlockIndex::assign_handle`. Helpers here use it to write tests
/// whose outcome is stable regardless of which ULID `NodeId::new()`
/// happens to produce first.
fn ordered_pair() -> (NodeId, NodeId) {
    let a = NodeId::new();
    let b = NodeId::new();
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

#[test]
fn collision_expands_handle_for_loser_and_keeps_winner_resolvable() {
    let mut idx = BlockIndex::default();
    // Smaller NodeId wins the base handle deterministically.
    let (id_winner, id_loser) = ordered_pair();
    let base = "blk-aaaaaa".to_string();
    let mut sb_w = sb(id_winner, "winner", 1, 0);
    let mut sb_l = sb(id_loser, "loser", 1, 0);
    sb_w.ref_handle = base.clone();
    sb_l.ref_handle = base.clone();

    // Insert the loser first to prove the winner displaces it
    // regardless of insertion order.
    idx.collect_page(
        "loser",
        &PathBuf::from("pages/loser.md"),
        &parse("- loser\n").blocks,
        &[sb_l],
    );
    idx.collect_page(
        "winner",
        &PathBuf::from("pages/winner.md"),
        &parse("- winner\n").blocks,
        &[sb_w],
    );

    let w = idx
        .resolve(&base)
        .expect("winner owns base after dethroning");
    assert_eq!(w.id, id_winner);

    let loser_entry = idx.get(id_loser).expect("loser still indexed");
    assert_ne!(loser_entry.ref_handle, base);
    let l = idx
        .resolve(&loser_entry.ref_handle)
        .expect("loser resolvable via expanded handle");
    assert_eq!(l.id, id_loser);
}

#[test]
fn forget_page_does_not_unresolve_winner_on_collision_removal() {
    let mut idx = BlockIndex::default();
    let (id_winner, id_loser) = ordered_pair();
    let base = "blk-bbbbbb".to_string();
    let mut sb_w = sb(id_winner, "winner", 1, 0);
    let mut sb_l = sb(id_loser, "loser", 1, 0);
    sb_w.ref_handle = base.clone();
    sb_l.ref_handle = base.clone();

    idx.collect_page(
        "winner",
        &PathBuf::from("pages/winner.md"),
        &parse("- winner\n").blocks,
        &[sb_w],
    );
    idx.collect_page(
        "loser",
        &PathBuf::from("pages/loser.md"),
        &parse("- loser\n").blocks,
        &[sb_l],
    );

    idx.forget_page("loser");

    let w = idx.resolve(&base).expect("winner intact after loser drop");
    assert_eq!(w.id, id_winner);
}

#[test]
fn mismatched_content_hash_skips_block_indexing() {
    let mut idx = BlockIndex::default();
    let id = NodeId::new();
    let md = "- alpha edited\n";
    let ast = parse(md);
    let stale = vec![sb(id, "alpha", 1, 0)];

    idx.collect_page("p", &PathBuf::from("pages/p.md"), &ast.blocks, &stale);
    assert_eq!(idx.block_count(), 0, "stale sidecar entry must not index");
}
