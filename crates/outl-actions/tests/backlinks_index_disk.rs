//! Regression guard: the client-facing backlinks index must be built
//! **from disk** and must NOT materialize the whole workspace.
//!
//! Reading every block's text through `Workspace::block_text` to find
//! mentions forces the entire vault to materialize on a lazy full-replay
//! boot (#179) and holds the workspace lock across an `O(blocks)` walk —
//! together, the "opening the journal / pressing Esc freezes for
//! seconds" bug. `build_backlink_index_from_disk` reads the projected
//! `.md` files instead, so it touches no `Workspace` and materializes
//! nothing. These tests fail loudly if a future change routes the
//! client index back through the workspace.

use std::path::Path;

use outl_actions::{
    append_block, apply_page_md_with_sidecar, build_backlink_index, build_backlink_index_from_disk,
    edit_text, find_by_slug, list_pages, open_or_create_page, page_meta, PageKind, PageMeta,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use tempfile::TempDir;

/// Create pages under `root`, project their `.md` + sidecar to disk, then
/// reopen from disk so block text is lazy (a full-replay boot, #179).
/// Returns the reopened workspace (nothing materialized yet) + its metas.
fn workspace_on_disk(
    root: &Path,
    notes: usize,
    blocks_per_note: usize,
) -> (Workspace, Vec<PageMeta>) {
    let ops_dir = root.join("ops");
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);

    {
        let storage = JsonlStorage::open(ops_dir.clone(), actor).unwrap();
        let mut w =
            Workspace::open_with_storage(actor, Box::new(storage), Some(root.to_path_buf()))
                .unwrap();
        open_or_create_page(&mut w, &hlc, "hub", "hub", PageKind::Page).unwrap();
        for i in 0..notes {
            let slug = format!("note-{i}");
            let note = open_or_create_page(&mut w, &hlc, &slug, &slug, PageKind::Page).unwrap();
            for b in 0..blocks_per_note {
                let text = if b % 3 == 0 {
                    format!("block {b} sees [[hub]] and #project")
                } else {
                    format!("block {b}: filler with no reference")
                };
                append_block(&mut w, &hlc, Some(note), Some(&text)).unwrap();
            }
        }
        // Project each page's `.md` AND `.outl` sidecar — the sidecar
        // carries the stable block ids the from-disk build reads, matching
        // what `finish_in_page` writes in production. `apply_all_pages_md`
        // writes only the `.md`, so it would leave the from-disk build with
        // position-derived ids that don't match the workspace.
        for meta in list_pages(&w) {
            let id = find_by_slug(&w, &meta.slug).unwrap();
            apply_page_md_with_sidecar(&w, root, id).unwrap();
        }
    } // drop: flush to disk

    let storage = JsonlStorage::open(ops_dir, actor).unwrap();
    let w =
        Workspace::open_with_storage(actor, Box::new(storage), Some(root.to_path_buf())).unwrap();
    let metas = list_pages(&w);
    (w, metas)
}

#[test]
fn from_disk_build_does_not_materialize_workspace() {
    let dir = TempDir::new().unwrap();
    let (w, metas) = workspace_on_disk(dir.path(), 40, 12);

    // Fresh full-replay boot: block text is lazy, nothing resident yet.
    let before = w.resident_text_count();

    let idx = build_backlink_index_from_disk(&metas, dir.path());
    let after_disk = w.resident_text_count();

    assert!(
        !idx.is_empty(),
        "the from-disk build should find the hub backlinks"
    );
    // The load-bearing assertion: reading the index from disk must not
    // have forced a single block to materialize in the workspace.
    assert_eq!(
        after_disk, before,
        "build_backlink_index_from_disk materialized workspace text \
         (before={before}, after={after_disk}) — this is the #179 freeze \
         regression; the client index must read .md, never Workspace::block_text",
    );
}

#[test]
fn workspace_build_materializes_everything_the_disk_build_avoids() {
    // Control: the workspace-based build (kept for one-shot CLI / tests)
    // DOES materialize the vault. This is exactly why the clients must
    // use the from-disk builder instead.
    let dir = TempDir::new().unwrap();
    let (w, _metas) = workspace_on_disk(dir.path(), 40, 12);

    let before = w.resident_text_count();
    let _ = build_backlink_index(&w, dir.path());
    let after = w.resident_text_count();

    assert!(
        after > before + 100,
        "control: the workspace build should materialize the vault \
         (before={before}, after={after})",
    );
}

#[test]
fn from_disk_matches_workspace_build() {
    // Correctness: both builders find the same backlinks for a page.
    let dir = TempDir::new().unwrap();
    let (w, metas) = workspace_on_disk(dir.path(), 25, 9);
    let hub = metas.iter().find(|m| m.slug == "hub").unwrap();

    let from_ws = build_backlink_index(&w, dir.path()).for_page(&w, hub);
    let from_disk = build_backlink_index_from_disk(&metas, dir.path()).for_page(&w, hub);

    let ids = |links: &[outl_actions::Backlink]| {
        let mut v: Vec<String> = links.iter().map(|b| b.block_id.clone()).collect();
        v.sort();
        v
    };
    assert!(!from_disk.is_empty());
    assert_eq!(
        ids(&from_ws),
        ids(&from_disk),
        "from-disk and from-workspace builds must return the same backlinks"
    );
}

#[test]
fn both_builds_carry_the_same_crumb_tokens() {
    // Every GUI client reads the from-disk index, and the breadcrumb is
    // rendered from `tokens` (no client runs a tokenizer). The two
    // builders take different routes to that field — the from-disk walk
    // clones what the parse already produced, the workspace walk
    // tokenizes the text it materialized — so they are a pair that can
    // drift, and only one of them is on the path a user sees.
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let ops_dir = root.join("ops");
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let branch = "#alinhamento [[hub]] internet 0800";

    {
        let storage = JsonlStorage::open(ops_dir.clone(), actor).unwrap();
        let mut w =
            Workspace::open_with_storage(actor, Box::new(storage), Some(root.to_path_buf()))
                .unwrap();
        open_or_create_page(&mut w, &hlc, "hub", "hub", PageKind::Page).unwrap();
        let note = open_or_create_page(&mut w, &hlc, "note", "note", PageKind::Page).unwrap();
        let parent = append_block(&mut w, &hlc, Some(note), Some(branch)).unwrap();
        append_block(&mut w, &hlc, Some(parent), Some("segue no [[hub]]")).unwrap();
        for meta in list_pages(&w) {
            let id = find_by_slug(&w, &meta.slug).unwrap();
            apply_page_md_with_sidecar(&w, root, id).unwrap();
        }
    }

    let storage = JsonlStorage::open(ops_dir, actor).unwrap();
    let w =
        Workspace::open_with_storage(actor, Box::new(storage), Some(root.to_path_buf())).unwrap();
    let metas = list_pages(&w);
    let hub = metas.iter().find(|m| m.slug == "hub").unwrap();

    let crumbs = |links: Vec<outl_actions::Backlink>| {
        links
            .into_iter()
            .filter(|b| !b.ancestors.is_empty())
            .map(|b| b.ancestors)
            .collect::<Vec<_>>()
    };
    let from_ws = crumbs(build_backlink_index(&w, root).for_page(&w, hub));
    let from_disk = crumbs(build_backlink_index_from_disk(&metas, root).for_page(&w, hub));

    assert_eq!(from_ws.len(), 1, "one nested reference, one breadcrumb");
    assert_eq!(from_disk.len(), from_ws.len());
    assert_eq!(from_disk[0][0].text, branch);
    assert_eq!(
        from_disk[0][0].tokens,
        outl_md::tokenize_owned(branch),
        "the from-disk crumb carries the branch's own tokens"
    );
    assert_eq!(
        from_ws[0][0].tokens, from_disk[0][0].tokens,
        "the two builders cannot disagree about how a crumb reads"
    );
}

/// Keep the meta helper honest so an unused-import lint never masks a
/// broken fixture.
#[test]
fn fixture_projects_pages_to_disk() {
    let dir = TempDir::new().unwrap();
    let (w, metas) = workspace_on_disk(dir.path(), 3, 4);
    assert!(page_meta(&w, outl_core::id::NodeId::root()).is_none());
    assert!(metas.iter().any(|m| m.slug == "hub"));
    assert!(dir.path().join("pages").join("hub.md").exists());
}

#[test]
fn reindex_page_reflects_an_edit_incrementally() {
    // The commit-path optimization: after editing one page, re-indexing
    // just that page's `.md` updates the whole index correctly — no full
    // workspace rescan needed.
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let storage = JsonlStorage::open(root.join("ops"), actor).unwrap();
    let mut w =
        Workspace::open_with_storage(actor, Box::new(storage), Some(root.to_path_buf())).unwrap();

    let hub = open_or_create_page(&mut w, &hlc, "hub", "hub", PageKind::Page).unwrap();
    let note = open_or_create_page(&mut w, &hlc, "note", "note", PageKind::Page).unwrap();
    let block = append_block(&mut w, &hlc, Some(note), Some("see [[hub]] here")).unwrap();
    for meta in list_pages(&w) {
        let id = find_by_slug(&w, &meta.slug).unwrap();
        apply_page_md_with_sidecar(&w, root, id).unwrap();
    }
    let hub_meta = page_meta(&w, hub).unwrap();
    let note_meta = page_meta(&w, note).unwrap();

    let mut index = build_backlink_index_from_disk(&list_pages(&w), root);
    assert_eq!(
        index.for_page(&w, &hub_meta).len(),
        1,
        "hub starts with one backlink from note"
    );

    // Edit the note to drop the ref, reproject its `.md`, reindex ONLY it.
    edit_text(&mut w, &hlc, block, "no reference anymore").unwrap();
    apply_page_md_with_sidecar(&w, root, note).unwrap();
    index.reindex_page_from_disk(&note_meta, root);

    assert_eq!(
        index.for_page(&w, &hub_meta).len(),
        0,
        "incremental reindex of note dropped hub's backlink"
    );
}
