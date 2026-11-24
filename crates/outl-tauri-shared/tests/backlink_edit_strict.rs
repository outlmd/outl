//! Editing a block through a backlink row, where tolerance is the bug.
//!
//! A backlink row shows a block that lives on **another** page, and its
//! `block_id` is trusted only as far as the tree confirms it:
//! `build_backlink_index_from_disk` reads each page through
//! `read_page_outline`, and `outline_from_parsed` mints a transient
//! `NodeId::new()` per block the sidecar does not cover (absent, short,
//! misaligned) by documented design. That id names no node in the tree.
//!
//! [`edit_block`] answers `NotInTree` by re-targeting through
//! `create_after_or_append`, which appends the text to the **end of the
//! page**. On the page the user is looking at that is a kindness — the
//! text stays where they can still see it. On a backlink it moves their
//! text to a page they are not looking at, which is root `CLAUDE.md`
//! invariant 8's shape (a fix whose mirrored direction deletes) arriving
//! through a different door.
//!
//! So the cross-page path **refuses**, and both commands keep their own
//! recovery. The pin on `edit_block`'s is deliberate: a test that only
//! covered the strict command would pass if someone "unified" the two
//! and put the append back.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{
    append_block, apply_page_md_with_sidecar, build_backlink_index_from_disk, find_by_slug,
    list_pages, open_or_create_page, page_meta, BacklinkIndex, PageKind, SyncTransport,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::commands::backlink_edit::edit_backlink_block;
use outl_tauri_shared::commands::block::edit_block;
use outl_tauri_shared::host::AppHost;
use outl_tauri_shared::surface::SURFACE;
use outl_tauri_shared::ProjectionWriter;
use parking_lot::Mutex;
use tempfile::TempDir;

/// Minimal [`AppHost`]: a workspace, an HLC, a storage root, and the
/// cached backlink-index slot a GUI client wires. No projection writer,
/// so the `.md` is written inline — the synchronous path.
struct TestHost {
    workspace: Arc<Mutex<Option<Workspace>>>,
    hlc: HlcGenerator,
    root: PathBuf,
    registry: Arc<RuntimeRegistry>,
    backlinks: Arc<Mutex<Option<BacklinkIndex>>>,
}

impl AppHost for TestHost {
    fn workspace(&self) -> &Mutex<Option<Workspace>> {
        &self.workspace
    }
    fn workspace_arc(&self) -> Arc<Mutex<Option<Workspace>>> {
        self.workspace.clone()
    }
    fn hlc(&self) -> &HlcGenerator {
        &self.hlc
    }
    fn storage_root(&self) -> Result<PathBuf, String> {
        Ok(self.root.clone())
    }
    fn sync_transport(&self) -> Option<Arc<dyn SyncTransport>> {
        None
    }
    fn exec_registry(&self) -> Arc<RuntimeRegistry> {
        self.registry.clone()
    }
    fn backlink_index(&self) -> Option<Arc<Mutex<Option<BacklinkIndex>>>> {
        Some(self.backlinks.clone())
    }
}

fn host_for(workspace: Workspace, hlc: HlcGenerator, root: &TempDir) -> TestHost {
    TestHost {
        workspace: Arc::new(Mutex::new(Some(workspace))),
        hlc,
        root: root.path().to_path_buf(),
        registry: Arc::new(RuntimeRegistry::new()),
        backlinks: Arc::new(Mutex::new(None)),
    }
}

/// The same host **with a `ProjectionWriter` wired**, sharing the one
/// workspace `Arc` every command locks — the shape both `outl-desktop`
/// and `outl-mobile` build at boot, and the *only* one either ships.
///
/// [`TestHost`] leaves `projection_writer()` at its `None` default, so
/// every test on it takes the synchronous fallback — where `DropAll` and
/// `PatchPage` differ only in the invalidate hook, and the branch
/// `PatchPage` exists for never runs. Root `CLAUDE.md` invariant 15:
/// *"Shipping a fast path whose test only covers the early return."*
struct AsyncTestHost {
    workspace: Arc<Mutex<Option<Workspace>>>,
    hlc: HlcGenerator,
    root: PathBuf,
    registry: Arc<RuntimeRegistry>,
    backlinks: Arc<Mutex<Option<BacklinkIndex>>>,
    projection_writer: ProjectionWriter,
}

impl AppHost for AsyncTestHost {
    fn workspace(&self) -> &Mutex<Option<Workspace>> {
        &self.workspace
    }
    fn workspace_arc(&self) -> Arc<Mutex<Option<Workspace>>> {
        self.workspace.clone()
    }
    fn hlc(&self) -> &HlcGenerator {
        &self.hlc
    }
    fn storage_root(&self) -> Result<PathBuf, String> {
        Ok(self.root.clone())
    }
    fn sync_transport(&self) -> Option<Arc<dyn SyncTransport>> {
        None
    }
    fn exec_registry(&self) -> Arc<RuntimeRegistry> {
        self.registry.clone()
    }
    fn backlink_index(&self) -> Option<Arc<Mutex<Option<BacklinkIndex>>>> {
        Some(self.backlinks.clone())
    }
    fn projection_writer(&self) -> Option<&ProjectionWriter> {
        Some(&self.projection_writer)
    }
}

/// A projected page holding one live block, plus the id of a block that
/// is **in the trash** — the state a transient id or a peer's delete
/// leaves behind while a backlink row still carries the id it rendered.
fn host_with_a_trashed_block() -> (TempDir, TestHost, NodeId, NodeId) {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    let page = open_or_create_page(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("kept")).unwrap();
    let doomed = append_block(&mut ws, &hlc, Some(page), Some("cited from somewhere")).unwrap();
    outl_actions::delete(&mut ws, &hlc, doomed).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();

    let host = host_for(ws, hlc, &tmp);
    (tmp, host, page, doomed)
}

/// Two pages: `notes` cites `[[target]]`, so the citing block is a
/// backlink of `target` and lives on a page the reader is not on.
fn host_with_a_backlink() -> (TempDir, TestHost, NodeId, NodeId) {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    let target = open_or_create_page(&mut ws, &hlc, "target", "Target", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(target), Some("the page being read")).unwrap();
    let source = open_or_create_page(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    let citing =
        append_block(&mut ws, &hlc, Some(source), Some("ask [[target]] about it")).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), target).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), source).unwrap();

    let host = host_for(ws, hlc, &tmp);
    (tmp, host, source, citing)
}

/// [`host_with_a_backlink`]'s writer-wired twin.
fn async_host_with_a_backlink() -> (TempDir, AsyncTestHost, NodeId, NodeId) {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    let target = open_or_create_page(&mut ws, &hlc, "target", "Target", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(target), Some("the page being read")).unwrap();
    let source = open_or_create_page(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    let citing =
        append_block(&mut ws, &hlc, Some(source), Some("ask [[target]] about it")).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), target).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), source).unwrap();

    let workspace = Arc::new(Mutex::new(Some(ws)));
    let projection_writer =
        ProjectionWriter::spawn(workspace.clone(), tmp.path().to_path_buf(), |_| {});
    let host = AsyncTestHost {
        workspace,
        hlc,
        root: tmp.path().to_path_buf(),
        registry: Arc::new(RuntimeRegistry::new()),
        backlinks: Arc::new(Mutex::new(None)),
        projection_writer,
    };
    (tmp, host, source, citing)
}

/// Fill the host's cached index the way `page_backlinks` does: once,
/// from disk, over every page.
///
/// Generic over [`AppHost`] so the sync and writer-wired hosts share one
/// set of probes — a second copy is how one of them would stop asserting
/// what the other does.
fn warm_the_index<S: AppHost>(state: &S) {
    let slot = state.backlink_index().expect("this host caches an index");
    let root = state.storage_root().unwrap();
    let metas = {
        let workspace = state.workspace_arc();
        let guard = workspace.lock();
        list_pages(guard.as_ref().unwrap())
    };
    *slot.lock() = Some(build_backlink_index_from_disk(&metas, &root));
}

/// What the cached index says the backlinks of `slug` read right now.
fn cached_backlink_texts<S: AppHost>(state: &S, slug: &str) -> Vec<String> {
    let slot = state.backlink_index().expect("this host caches an index");
    let workspace = state.workspace_arc();
    let guard = workspace.lock();
    let ws = guard.as_ref().unwrap();
    let id = find_by_slug(ws, slug).expect("the page exists");
    let meta = page_meta(ws, id).expect("the page has meta");
    let index = slot.lock();
    index
        .as_ref()
        .expect("the slot still holds an index")
        .for_page(ws, &meta)
        .into_iter()
        .map(|link| link.source_block.text)
        .collect()
}

/// Every block's text on `page`, in outline order, straight from the tree.
fn page_texts<S: AppHost>(state: &S, page: NodeId) -> Vec<String> {
    let workspace = state.workspace_arc();
    let guard = workspace.lock();
    let ws = guard.as_ref().unwrap();
    outl_actions::children_of(ws, page)
        .into_iter()
        .filter_map(|(id, _)| ws.block_text(id))
        .collect()
}

fn op_count<S: AppHost>(state: &S) -> usize {
    let workspace = state.workspace_arc();
    let ops = workspace.lock().as_ref().unwrap().log().len();
    ops
}

/// Re-stamp the sidecar so its `last_synced_hash` agrees with whatever
/// is on disk — the state invariant 8 exists for: outl wrote these bytes
/// last, and the op log still does not hold all of them.
fn restamp_sidecar_as_faithful(md_path: &PathBuf) {
    let sidecar_path = outl_md::sidecar::sidecar_path_for(md_path);
    let mut sc = outl_md::sidecar::read(&sidecar_path).unwrap();
    let disk = std::fs::read_to_string(md_path).unwrap();
    sc.last_synced_hash = outl_md::sidecar::file_hash(&disk);
    outl_md::sidecar::write(&sidecar_path, &sc).unwrap();
}

/// Review focus 1, and both halves are the guarantee. An error alone
/// would be satisfied by a command that also appended; a page that
/// gained no block would be satisfied by a command that silently did
/// nothing. `edit_block` returns `Ok` here **and** appends, which is why
/// the second assertion is the one that cannot be dropped.
#[test]
fn a_stale_block_id_is_refused_rather_than_appended() {
    let (_tmp, host, page, doomed) = host_with_a_trashed_block();
    let before_texts = page_texts(&host, page);
    let before_ops = op_count(&host);

    let result = edit_backlink_block(
        &host,
        page.to_string(),
        doomed.to_string(),
        "what the user typed into the backlink row".to_string(),
    );

    let err = result.expect_err(
        "a transient backlink id must not be re-targeted — the append lands on a page \
         the user is not looking at",
    );
    assert_eq!(
        err, "That block was deleted, so your text could not be saved.",
        "a block a peer deleted is said to be deleted"
    );
    assert_eq!(
        page_texts(&host, page),
        before_texts,
        "the source page must gain no block: the whole point of refusing"
    );
    assert_eq!(
        op_count(&host),
        before_ops,
        "and no op may be written for a refused cross-page edit"
    );
}

/// The other reason a backlink's block is gone, and the one "deleted"
/// would misdescribe: the id was transient and never named a node
/// (sidecar absent, short or misaligned). Nothing was deleted, so the
/// sentence must not say so, and it must not show the user a ULID.
#[test]
fn an_id_that_never_named_a_block_is_not_called_deleted() {
    let (_tmp, host, page, _doomed) = host_with_a_trashed_block();
    let before_ops = op_count(&host);
    let transient = outl_core::id::NodeId::new();

    let err = edit_backlink_block(
        &host,
        page.to_string(),
        transient.to_string(),
        "typed into a row whose id was minted on read".to_string(),
    )
    .expect_err("a transient id names no block to edit");

    assert_eq!(
        err,
        "That block could not be found, so your text could not be saved."
    );
    assert!(
        !err.contains("deleted"),
        "a confidently wrong cause is worse than a vaguer true one: {err:?}"
    );
    assert!(
        !err.contains(&transient.to_string()),
        "no raw id reaches the user: {err:?}"
    );
    assert_eq!(op_count(&host), before_ops, "and nothing is written");
}

/// The sibling behaviour stays. `edit_block` keeps the user's text where
/// they can still see it, because on the open page the end of the page
/// *is* visible and mobile pins a Retry to a failure. A test that only
/// pinned the strict command would pass if someone unified the two and
/// put the append back on the cross-page path.
///
/// `tests/edit_block_stale_id.rs` owns the full behaviour (the
/// `DroppedEdit` notice, the trashed-page case); this asserts only the
/// half the strict command deliberately does **not** share.
#[test]
fn edit_block_still_re_targets_so_the_open_page_keeps_its_text() {
    let (_tmp, host, page, doomed) = host_with_a_trashed_block();

    let view = edit_block(
        &host,
        page.to_string(),
        doomed.to_string(),
        "what the user typed into the open page".to_string(),
    )
    .expect("the same-page path must not fail the commit");

    assert!(
        view.outline
            .iter()
            .any(|node| node.text == "what the user typed into the open page"),
        "the open page keeps the text: {:?}",
        view.outline.iter().map(|n| &n.text).collect::<Vec<_>>()
    );
}

/// Review focus 2. A page whose `.md` holds lines that exist in no op is
/// frozen in both directions (invariant 8): the op would apply to the
/// *logged* text, which is not the text the row rendered, and the
/// projection would then be refused — an edit that silently did not
/// happen. So it is refused before anything lands.
///
/// Every word of the refusal is `outl_actions::refusal::ahead_of_log_notice`'s.
/// A client decides where a notice goes, never what it says.
#[test]
fn a_page_ahead_of_its_log_is_refused_with_the_shared_wording() {
    let (tmp, host, page, _doomed) = host_with_a_trashed_block();
    let md_path = tmp.path().join("pages").join("notes.md");
    std::fs::write(&md_path, "- kept\n- only ever on disk\n").unwrap();
    restamp_sidecar_as_faithful(&md_path);
    let live = {
        let guard = host.workspace.lock();
        let ws = guard.as_ref().unwrap();
        outl_actions::children_of(ws, page)
            .into_iter()
            .map(|(id, _)| id)
            .next()
            .unwrap()
    };
    let before_ops = op_count(&host);

    let err = edit_backlink_block(
        &host,
        page.to_string(),
        live.to_string(),
        "edited through a backlink row".to_string(),
    )
    .expect_err("a page that stopped syncing cannot take a cross-page edit");

    let expected = outl_actions::refusal::ahead_of_log_notice(SURFACE, 1, "- only ever on disk");
    assert!(
        err.contains(expected.title),
        "the refusal must carry the shared headline, got {err:?}"
    );
    assert!(
        err.contains(expected.command),
        "and the recovery command the shared notice names, got {err:?}"
    );
    assert_eq!(
        op_count(&host),
        before_ops,
        "refused before the mutation, not after it"
    );
    assert_eq!(
        std::fs::read_to_string(&md_path).unwrap(),
        "- kept\n- only ever on disk\n",
        "and the bytes the guard protects are untouched"
    );
}

/// Invariant 15. Every commit drops the whole cached index, so the next
/// `page_backlinks` re-reads and re-parses every `.md` in the workspace.
/// That is fine for an occasional TODO toggle and not fine for text
/// editing, which is the daily flow this command exists for.
///
/// Two assertions, because either alone passes a broken fix: the slot
/// still holds an index (so the next read is an `O(refs)` lookup), and
/// that index reads the text the user just typed (a patch from a stale
/// `.md` is as wrong as a drop, and silently so).
#[test]
fn reindexing_one_page_leaves_the_cached_index_in_place() {
    let (_tmp, host, source, citing) = host_with_a_backlink();
    warm_the_index(&host);
    assert_eq!(
        cached_backlink_texts(&host, "target"),
        vec!["ask [[target]] about it".to_string()],
        "precondition: the warm index already names the citing block"
    );

    edit_backlink_block(
        &host,
        source.to_string(),
        citing.to_string(),
        "ask [[target]] about the invoice".to_string(),
    )
    .expect("a live cross-page edit must land");

    assert!(
        host.backlinks.lock().is_some(),
        "the index must survive the commit — dropping it makes the next read a \
         workspace rescan"
    );
    assert_eq!(
        cached_backlink_texts(&host, "target"),
        vec!["ask [[target]] about the invoice".to_string()],
        "and the patched entry must read the text the user just typed"
    );
}

/// The same guarantee on the **only** host configuration either client
/// ships: `projection_writer()` returning `Some`.
///
/// The test above runs on the synchronous fallback, where `DropAll` and
/// `PatchPage` differ only in the invalidate hook — so it never reaches
/// the branch `PatchPage` exists for. Here the writer is wired, and
/// `PatchPage` must still take its projection inline: relax that and the
/// write is queued, `reindex_page_from_disk` reads the **pre-edit** `.md`,
/// and the cached index serves the previous text with a green suite.
///
/// Deliberately no `flush()` and no sleep before the assertion. The
/// guarantee is that the write already landed inside the commit's own
/// lock; waiting for the worker here would pass either way and test
/// nothing.
#[test]
fn a_writer_wired_host_patches_the_index_from_the_md_it_just_wrote() {
    let (_tmp, host, source, citing) = async_host_with_a_backlink();
    warm_the_index(&host);
    assert_eq!(
        cached_backlink_texts(&host, "target"),
        vec!["ask [[target]] about it".to_string()],
        "precondition: the warm index already names the citing block"
    );

    edit_backlink_block(
        &host,
        source.to_string(),
        citing.to_string(),
        "ask [[target]] about the invoice".to_string(),
    )
    .expect("a live cross-page edit must land on a writer-wired host too");

    assert!(
        host.backlinks.lock().is_some(),
        "the index must survive the commit on this host as well"
    );
    assert_eq!(
        cached_backlink_texts(&host, "target"),
        vec!["ask [[target]] about the invoice".to_string()],
        "a queued projection would have left this reading the pre-edit text"
    );
}

/// A row built while a child lived on `notes` keeps `(notes, child)`
/// after the child moves to another page. The edit would commit on the
/// wrong page (undo snapshot and projection are scoped to `page_id`), so
/// it is refused in the stale wording, with nothing written.
#[test]
fn a_block_moved_to_another_page_is_refused_and_a_deep_child_commits() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let notes = open_or_create_page(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    let elsewhere =
        open_or_create_page(&mut ws, &hlc, "elsewhere", "Elsewhere", PageKind::Page).unwrap();
    let citing = append_block(&mut ws, &hlc, Some(notes), Some("cites [[target]]")).unwrap();
    let child = append_block(&mut ws, &hlc, Some(citing), Some("child")).unwrap();
    let grand = append_block(&mut ws, &hlc, Some(child), Some("grandchild")).unwrap();
    let deep = append_block(&mut ws, &hlc, Some(grand), Some("deep")).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), notes).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), elsewhere).unwrap();
    let host = host_for(ws, hlc, &tmp);

    edit_backlink_block(
        &host,
        notes.to_string(),
        deep.to_string(),
        "edited deep".into(),
    )
    .expect("a three-level-deep child of the page commits");
    let ops_after_deep = op_count(&host);

    {
        let workspace = host.workspace_arc();
        let mut guard = workspace.lock();
        let ws = guard.as_mut().unwrap();
        outl_actions::move_under(ws, &host.hlc, child, elsewhere).unwrap();
    }
    let ops = op_count(&host);
    assert!(ops > ops_after_deep, "the move itself is an op");
    let read = |slug: &str| {
        std::fs::read_to_string(tmp.path().join("pages").join(format!("{slug}.md"))).unwrap()
    };
    let (notes_md, elsewhere_md) = (read("notes"), read("elsewhere"));

    let err = edit_backlink_block(&host, notes.to_string(), child.to_string(), "x".into())
        .expect_err("a block that left the page is not this row's block");
    assert_eq!(
        err,
        "That block could not be found, so your text could not be saved."
    );
    assert_eq!(op_count(&host), ops, "nothing may reach the op log");
    assert_eq!(read("notes"), notes_md);
    assert_eq!(read("elsewhere"), elsewhere_md);
}
