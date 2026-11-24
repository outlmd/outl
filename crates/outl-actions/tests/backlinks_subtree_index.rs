//! A backlink's children live in a side table on the from-disk index, and
//! the three things a performance change owes (root `CLAUDE.md`
//! invariant 15) are each asserted separately here, because none of them
//! covers another:
//!
//! 1. **Still right** — the stored subtree equals the one a fresh read of
//!    the page produces, capped at `SUBTREE_MAX_DEPTH`.
//! 2. **Still cheap, on every branch** — a lookup returns a shallow leaf,
//!    `subtree()` is a refcount bump rather than a copy, and the
//!    workspace builder (CLI / MCP) never has one.
//! 3. **Cheap once** is the caller's census, in `outl-tauri-shared`.

use std::path::Path;
use std::sync::Arc;

use outl_actions::{
    append_block, apply_page_md_with_sidecar, build_backlink_index, build_backlink_index_from_disk,
    clone_capped, edit_text, find_by_slug, list_pages, open_or_create_page, page_meta,
    read_page_outline, OutlineNode, PageKind, PageMeta, SUBTREE_MAX_DEPTH,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use tempfile::TempDir;

struct Fixture {
    w: Workspace,
    hlc: HlcGenerator,
    root: std::path::PathBuf,
    note: NodeId,
    /// The citing block with a six-level chain under it.
    deep: NodeId,
    /// A citing block with no children.
    leaf: NodeId,
    /// First link of the chain, the one an edit test rewrites.
    first_child: NodeId,
}

fn project_all(w: &Workspace, root: &Path) {
    for meta in list_pages(w) {
        let id = find_by_slug(w, &meta.slug).unwrap();
        apply_page_md_with_sidecar(w, root, id).unwrap();
    }
}

fn fixture(dir: &TempDir) -> Fixture {
    let root = dir.path().to_path_buf();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let storage = JsonlStorage::open(root.join("ops"), actor).unwrap();
    let mut w = Workspace::open_with_storage(actor, Box::new(storage), Some(root.clone())).unwrap();
    open_or_create_page(&mut w, &hlc, "hub", "hub", PageKind::Page).unwrap();
    let note = open_or_create_page(&mut w, &hlc, "note", "note", PageKind::Page).unwrap();

    let deep = append_block(&mut w, &hlc, Some(note), Some("deep cites [[hub]]")).unwrap();
    let mut parent = deep;
    let mut first_child = None;
    for level in 1..=6 {
        parent = append_block(&mut w, &hlc, Some(parent), Some(&format!("level {level}"))).unwrap();
        first_child.get_or_insert(parent);
    }
    let leaf = append_block(&mut w, &hlc, Some(note), Some("leaf cites [[hub]]")).unwrap();
    project_all(&w, &root);

    Fixture {
        w,
        hlc,
        root,
        note,
        deep,
        leaf,
        first_child: first_child.unwrap(),
    }
}

fn find<'a>(nodes: &'a [OutlineNode], id: &str) -> Option<&'a OutlineNode> {
    nodes.iter().find_map(|n| {
        if n.id == id {
            Some(n)
        } else {
            find(&n.children, id)
        }
    })
}

fn depth(nodes: &[OutlineNode]) -> usize {
    nodes
        .iter()
        .map(|n| 1 + depth(&n.children))
        .max()
        .unwrap_or(0)
}

fn json(nodes: &[OutlineNode]) -> serde_json::Value {
    serde_json::to_value(nodes).unwrap()
}

fn hub_meta(w: &Workspace) -> PageMeta {
    list_pages(w).into_iter().find(|m| m.slug == "hub").unwrap()
}

#[test]
fn the_stored_subtree_equals_a_fresh_read_capped() {
    let dir = TempDir::new().unwrap();
    let f = fixture(&dir);
    let metas = list_pages(&f.w);
    let index = build_backlink_index_from_disk(&metas, &f.root);

    let note_meta = metas.iter().find(|m| m.slug == "note").unwrap();
    let outline = read_page_outline(&f.root, note_meta).unwrap();
    let source = find(&outline.nodes, &f.deep.to_string()).unwrap();

    let stored = index
        .subtree(&f.deep.to_string())
        .expect("a citing block with children has a stored subtree");

    assert_eq!(
        json(&stored),
        json(&clone_capped(&source.children, SUBTREE_MAX_DEPTH)),
        "ids, text and tokens must match a fresh read"
    );
    assert_eq!(
        depth(&stored),
        SUBTREE_MAX_DEPTH,
        "a six-level chain comes back capped, the rest is never stored"
    );
}

#[test]
fn a_leaf_has_no_stored_subtree() {
    let dir = TempDir::new().unwrap();
    let f = fixture(&dir);
    let index = build_backlink_index_from_disk(&list_pages(&f.w), &f.root);

    assert!(index.subtree(&f.leaf.to_string()).is_none());
    assert!(index.subtree("not-a-block-id").is_none());
}

#[test]
fn a_lookup_returns_shallow_leaves_and_subtree_is_not_a_copy() {
    let dir = TempDir::new().unwrap();
    let f = fixture(&dir);
    let index = build_backlink_index_from_disk(&list_pages(&f.w), &f.root);
    let hub = hub_meta(&f.w);

    let split = index.for_page_split(&f.w, &hub);
    assert!(
        !split.direct.is_empty(),
        "the fixture has two citing blocks"
    );
    for hit in split.direct.iter().chain(split.namespaced.iter()) {
        assert!(
            hit.source_block.children.is_empty(),
            "a lookup runs under the workspace lock on the GUI: it must clone a leaf, \
             never a subtree (hit {})",
            hit.block_id
        );
    }
    for hit in index.for_page(&f.w, &hub) {
        assert!(hit.source_block.children.is_empty());
    }

    let a = index.subtree(&f.deep.to_string()).unwrap();
    let b = index.subtree(&f.deep.to_string()).unwrap();
    assert!(
        Arc::ptr_eq(&a, &b),
        "subtree() must hand out the stored Arc, not a deep copy"
    );
}

#[test]
fn with_children_is_the_only_way_a_subtree_reaches_a_backlink() {
    let dir = TempDir::new().unwrap();
    let f = fixture(&dir);
    let index = build_backlink_index_from_disk(&list_pages(&f.w), &f.root);
    let hub = hub_meta(&f.w);

    let hit = index
        .for_page(&f.w, &hub)
        .into_iter()
        .find(|b| b.block_id == f.deep.to_string())
        .unwrap();
    let kids = index.subtree(&hit.block_id).unwrap();
    let attached = hit.with_children(&kids);

    assert_eq!(json(&attached.source_block.children), json(&kids));
    assert!(
        attached.into_shallow().source_block.children.is_empty(),
        "into_shallow stays the narrowing for rows that must not carry children"
    );
}

#[test]
fn the_workspace_builder_never_has_a_subtree() {
    // CLI / MCP read through the workspace, where filling children means
    // materializing every descendant under the lock (#179). The asymmetry
    // with the from-disk builder is deliberate, and pinned so that nobody
    // "fixes" one side to match the other.
    let dir = TempDir::new().unwrap();
    let f = fixture(&dir);
    let index = build_backlink_index(&f.w, &f.root);

    assert!(index.subtree(&f.deep.to_string()).is_none());
    assert!(index.subtree(&f.leaf.to_string()).is_none());
}

#[test]
fn reindexing_a_page_replaces_its_stored_subtree() {
    let dir = TempDir::new().unwrap();
    let mut f = fixture(&dir);
    let metas = list_pages(&f.w);
    let mut index = build_backlink_index_from_disk(&metas, &f.root);
    let note_meta = page_meta(&f.w, f.note).unwrap();

    let before = index.subtree(&f.deep.to_string()).unwrap();
    assert_eq!(before[0].text, "level 1");

    edit_text(&mut f.w, &f.hlc, f.first_child, "level 1 edited").unwrap();
    apply_page_md_with_sidecar(&f.w, &f.root, f.note).unwrap();
    index.reindex_page_from_disk(&note_meta, &f.root);

    let after = index.subtree(&f.deep.to_string()).unwrap();
    assert_eq!(
        after[0].text, "level 1 edited",
        "an edit to a descendant is a page-level reindex, no extra invalidation hook"
    );
    assert!(
        !Arc::ptr_eq(&before, &after),
        "the old Arc is dropped with the page's entries"
    );
}
