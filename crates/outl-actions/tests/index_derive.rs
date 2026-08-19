//! The tree-derived workspace index (`outl_actions::index::derive`).
//!
//! These tests pin the two claims that justify the derivation existing
//! at all. Both are stated against the **disk** build
//! (`WorkspaceIndex::build`) so they fail if someone quietly makes the
//! two paths equivalent in the wrong direction:
//!
//! 1. **Parity** — for a workspace whose `.md` files are a faithful
//!    projection of the tree, the two builds agree on pages, blocks,
//!    handles and reverse refs. Without this the derivation is not a
//!    replacement, it is a second opinion, and the whole point is to
//!    stop having two.
//! 2. **No positional skip** — the disk path pairs the parsed AST with
//!    `sidecar_blocks[cursor]` and drops any block whose
//!    `content_hash` disagrees, so a `.md` edited before reconcile ran
//!    loses the rest of the page from the index. The tree path cannot:
//!    the id arrives on the node.
//!
//! (2) is the user-visible one: it is why a block typed in vim stops
//! being findable by `outl search` and why its `((blk-…))` stops
//! resolving, until a reconcile happens to run.

use std::fs;
use std::path::Path;

use outl_actions::page::{open_or_create, set_property, PageKind};
use outl_actions::{append_block, apply_page_md_with_sidecar};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;
use outl_md::index::WorkspaceIndex;
use tempfile::TempDir;

/// An in-memory workspace plus the temp dir its `.md` projections land
/// in.
struct Fixture {
    workspace: Workspace,
    hlc: HlcGenerator,
    dir: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let actor = ActorId::new();
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("pages")).unwrap();
        fs::create_dir_all(dir.path().join("journals")).unwrap();
        Self {
            workspace: Workspace::open_in_memory(actor).unwrap(),
            hlc: HlcGenerator::new(actor),
            dir,
        }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn page(&mut self, slug: &str, title: &str) -> NodeId {
        open_or_create(&mut self.workspace, &self.hlc, slug, title, PageKind::Page).unwrap()
    }

    fn block(&mut self, parent: NodeId, text: &str) -> NodeId {
        append_block(&mut self.workspace, &self.hlc, Some(parent), Some(text)).unwrap()
    }

    /// Write the page's `.md` + sidecar, so the disk build has
    /// something faithful to read.
    fn project(&self, page_root: NodeId) {
        apply_page_md_with_sidecar(&self.workspace, self.root(), page_root).unwrap();
    }

    fn derived(&self) -> WorkspaceIndex {
        outl_actions::index::derive(&self.workspace, self.root())
    }

    fn built_from_disk(&self) -> WorkspaceIndex {
        WorkspaceIndex::build(self.root())
    }
}

/// Slug + title + block text of every indexed block, sorted — the
/// comparable shape of an index.
/// `(slug, title)` per page and `(slug, handle, text)` per block.
type IndexSummary = (Vec<(String, String)>, Vec<(String, String, String)>);

fn summary(idx: &WorkspaceIndex) -> IndexSummary {
    let mut pages: Vec<(String, String)> = idx
        .pages()
        .map(|p| (p.slug.clone(), p.title.clone()))
        .collect();
    pages.sort();
    let mut blocks: Vec<(String, String, String)> = idx
        .iter_blocks()
        .map(|b| (b.source_slug.clone(), b.ref_handle.clone(), b.text.clone()))
        .collect();
    blocks.sort();
    (pages, blocks)
}

#[test]
fn derive_agrees_with_the_disk_build_on_a_faithful_workspace() {
    let mut f = Fixture::new();

    let notes = f.page("notes", "Notes");
    let a = f.block(notes, "first block");
    f.block(a, "a child of the first");
    f.block(notes, "second block");

    let other = f.page("other", "Other Page");
    f.block(other, "somewhere else entirely");

    f.project(notes);
    f.project(other);

    let derived = f.derived();
    let disk = f.built_from_disk();

    assert_eq!(
        summary(&derived),
        summary(&disk),
        "tree-derived and disk-built indexes must describe the same workspace"
    );
    assert_eq!(derived.page_count(), 2);
    assert_eq!(derived.block_count(), 4);
}

#[test]
fn derive_agrees_with_the_disk_build_on_reverse_refs() {
    let mut f = Fixture::new();

    let target_page = f.page("target", "Target");
    let cited = f.block(target_page, "the block everyone cites");

    // Resolve the handle the way a user would: through the index.
    f.project(target_page);
    let handle = f
        .derived()
        .block_by_id(cited)
        .expect("the cited block is indexed")
        .ref_handle
        .clone();

    let citing_page = f.page("citing", "Citing");
    f.block(citing_page, &format!("see ((({handle})))")[1..]);
    f.project(citing_page);

    let derived = f.derived();
    let disk = f.built_from_disk();

    assert_eq!(
        derived.block_refs_to(cited).len(),
        1,
        "the citing block must produce one reverse edge"
    );
    assert_eq!(
        derived.block_refs_to(cited),
        disk.block_refs_to(cited),
        "reverse edges must match the disk build"
    );
}

#[test]
fn derive_still_sees_a_page_whose_md_was_edited_before_reconcile_ran() {
    let mut f = Fixture::new();

    let notes = f.page("notes", "Notes");
    f.block(notes, "block one");
    f.block(notes, "block two");
    f.block(notes, "block three");
    f.project(notes);

    // An external editor inserts a bullet at the top and saves. The
    // sidecar still describes the previous revision — exactly the
    // window between an external write and the next reconcile.
    let md = f.root().join("pages/notes.md");
    let edited = format!("- typed in vim\n{}", fs::read_to_string(&md).unwrap());
    fs::write(&md, edited).unwrap();

    let disk = f.built_from_disk();
    let derived = f.derived();

    // The disk build desynchronises its cursor on the inserted line and
    // drops what follows. This assertion documents the defect; if a
    // future change fixes the disk path, tighten it rather than
    // deleting it.
    assert!(
        disk.block_count() < 3,
        "the disk build is expected to lose blocks here (got {}), \
         which is the failure this derivation removes",
        disk.block_count()
    );

    // The tree is untouched by the external edit, so every block the op
    // log holds is still indexed and still searchable.
    assert_eq!(
        derived.block_count(),
        3,
        "the tree-derived index must not lose a block to a stale sidecar"
    );
    assert_eq!(
        derived.search_block_text("block three", 10).len(),
        1,
        "a block the op log holds must stay findable regardless of sidecar state"
    );
}

#[test]
fn derive_reads_page_metadata_off_the_tree() {
    let mut f = Fixture::new();
    let page = f.page("avelino", "Avelino");
    f.block(page, "a block");
    f.project(page);

    let idx = f.derived();
    let entry = idx.by_slug("avelino").expect("page is indexed");

    // Every one of these comes off the page root's properties, not off
    // a parsed `.md` header.
    assert_eq!(entry.title, "Avelino");
    assert!(!entry.is_journal);
    assert_eq!(
        idx.by_title("Avelino").map(|p| p.slug.as_str()),
        Some("avelino"),
        "the title alias must be registered"
    );
    assert!(
        entry.path.ends_with("pages/avelino.md"),
        "PageEntry::path must point at the page's `.md`, got {}",
        entry.path.display()
    );
}

#[test]
fn derive_skips_root_children_that_are_not_pages() {
    let mut f = Fixture::new();
    let page = f.page("notes", "Notes");
    f.block(page, "a block");
    // A bare block parented to the root carries no `page-slug`, so it
    // is not a page and must not become one.
    f.block(NodeId::root(), "orphan block at the root");

    let idx = f.derived();
    assert_eq!(idx.page_count(), 1);
    assert!(idx.by_slug("notes").is_some());
}

#[test]
fn derive_on_an_empty_workspace_is_empty_not_a_panic() {
    let f = Fixture::new();
    let idx = f.derived();
    assert_eq!(idx.page_count(), 0);
    assert_eq!(idx.block_count(), 0);
}

#[test]
fn a_block_property_named_like_page_book_keeping_survives_the_round_trip() {
    // `page-slug` / `page-kind` are book-keeping on a *page root*. On an
    // ordinary block they are whatever the user typed, and the dialect
    // has no allow-list of keys, so the parser accepts them and the diff
    // emits a `SetProp` like any other property.
    //
    // Filtering them out of a block's projection puts the value in the
    // tree and nowhere on disk, and the next external-edit reconcile
    // emits the removal. That is convergent data loss, and it is the
    // exact failure `block_properties` was written to stop.
    let mut f = Fixture::new();
    let notes = f.page("notes", "Notes");
    let block = f.block(notes, "a block that mentions a page");
    set_property(
        &mut f.workspace,
        &f.hlc,
        block,
        "page-kind",
        Some(PropValue::Text("not-book-keeping".to_string())),
    )
    .unwrap();
    f.project(notes);

    let md = fs::read_to_string(f.root().join("pages/notes.md")).unwrap();
    assert!(
        md.contains("page-kind:: not-book-keeping"),
        "the block property must reach the `.md`, got:\n{md}"
    );

    // And the index must agree with the renderer about what the block
    // carries, or a query filtering on the property misses it.
    let idx = f.derived();
    let entry = idx.block_by_id(block).expect("the block is indexed");
    assert_eq!(entry.source_slug, "notes");
}

#[test]
fn duplicate_slug_roots_index_as_one_page() {
    // Split-brain: two live roots carrying one slug, until
    // `merge_duplicate_slug_roots` repairs it. The index is keyed by
    // slug in three places (page entry, per-page block list, and
    // `(slug, dfs_path) -> NodeId`), so indexing both roots lets the
    // last one win the entry while their blocks merge under one slug
    // and their DFS paths collide.
    let mut f = Fixture::new();
    let first = f.page("notes", "Notes");
    f.block(first, "block from the first root");

    // A second root with the same slug, as a peer's concurrent create
    // would leave it.
    let second = append_block(&mut f.workspace, &f.hlc, Some(NodeId::root()), None).unwrap();
    set_property(
        &mut f.workspace,
        &f.hlc,
        second,
        "page-slug",
        Some(PropValue::Text("notes".to_string())),
    )
    .unwrap();
    f.block(second, "block from the duplicate root");

    let idx = f.derived();

    assert_eq!(idx.page_count(), 1, "one slug must yield one page entry");
    // Whichever root wins, `(slug, path)` must resolve to a block that
    // actually belongs to the page the entry describes.
    let located = idx
        .block_at_location("notes", &[0])
        .expect("the first block of the winning root resolves");
    assert_eq!(located.source_slug, "notes");
    assert_eq!(
        idx.block_count(),
        1,
        "only the winning root's blocks are indexed; the loser's stay in \
         the op log until the merge repair re-parents them"
    );
}
