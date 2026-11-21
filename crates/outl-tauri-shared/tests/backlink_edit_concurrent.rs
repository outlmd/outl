//! Editing through a backlink row while a peer's ops are arriving.
//!
//! `tests/backlink_edit_strict.rs` pins the two refusals against a page
//! that was already in the bad state when the command ran. What it cannot
//! say is that they hold when the state changes *between the row render
//! and the commit*, which is the only way a user reaches them: the row is
//! drawn from `build_backlink_index_from_disk`, the user types, and `Esc`
//! commits some seconds later with whatever a peer wrote in the meantime.
//!
//! The peer is not mocked. Sync ingest ends in ops in the log under a
//! different actor, so each test applies the peer's change through
//! `Workspace::apply` (via the shared action functions) with a second
//! `HlcGenerator`, after the "render" read and before the commit.
//!
//! Three questions, one per test group:
//!
//! - a stale id is **refused, never re-targeted**, and the source page
//!   gains nothing (`edit_block` would have appended);
//! - a page ahead of its log is refused **before** the mutation, which is
//!   observable as "no op in the log", since a post-commit refusal is an
//!   edit that is already durable;
//! - a peer editing a *different* block is not a conflict, and both edits
//!   land, which is what proves the refusals are not over-broad.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::refusal::{ahead_of_log_notice, stale_backlink_block_text};
use outl_actions::{
    append_block, apply_page_md_with_sidecar, build_backlink_index_from_disk, children_of,
    edit_text, find_by_slug, list_pages, open_or_create_page, page_meta, BacklinkIndex, PageKind,
    SyncTransport,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::commands::backlink_edit::edit_backlink_block;
use outl_tauri_shared::helpers::parse_node_id;
use outl_tauri_shared::host::AppHost;
use outl_tauri_shared::surface::SURFACE;
use outl_tauri_shared::ProjectionWriter;
use parking_lot::Mutex;
use tempfile::TempDir;

/// The shape both GUI clients ship: a cached index slot and a projection
/// writer (`Some` is the only `projection_writer()` either client wires).
struct TestHost {
    workspace: Arc<Mutex<Option<Workspace>>>,
    hlc: HlcGenerator,
    root: PathBuf,
    registry: Arc<RuntimeRegistry>,
    backlinks: Arc<Mutex<Option<BacklinkIndex>>>,
    writer: ProjectionWriter,
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
    fn projection_writer(&self) -> Option<&ProjectionWriter> {
        Some(&self.writer)
    }
}

/// A second device. Its own actor and its own clock, so its ops are
/// distinguishable from the local ones and carry an HLC of their own.
struct Peer {
    hlc: HlcGenerator,
}

impl Peer {
    fn new() -> Self {
        Self {
            hlc: HlcGenerator::new(ActorId::new()),
        }
    }

    /// Apply a change as that actor, the way ingested sync ops land.
    fn with<R>(&self, host: &TestHost, f: impl FnOnce(&mut Workspace, &HlcGenerator) -> R) -> R {
        let mut guard = host.workspace.lock();
        let ws = guard.as_mut().expect("the workspace is open");
        f(ws, &self.hlc)
    }
}

struct Fixture {
    _tmp: TempDir,
    host: TestHost,
    source: NodeId,
    /// The block the backlink row cites `[[target]]` from.
    citing: NodeId,
    /// A sibling on the same source page that cites nothing.
    other: NodeId,
}

/// `notes` holds two blocks, the first of which cites `[[target]]`, so it
/// is a backlink of `target` and lives on a page the reader is not on.
/// Both pages projected and in sync.
fn fixture() -> Fixture {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path().to_path_buf();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).expect("in-memory workspace");

    let target = open_or_create_page(&mut ws, &hlc, "target", "Target", PageKind::Page)
        .expect("the page being read");
    append_block(&mut ws, &hlc, Some(target), Some("the page being read")).expect("a block");
    let source =
        open_or_create_page(&mut ws, &hlc, "notes", "Notes", PageKind::Page).expect("source");
    let citing = append_block(&mut ws, &hlc, Some(source), Some("ask [[target]] about it"))
        .expect("the citing block");
    let other =
        append_block(&mut ws, &hlc, Some(source), Some("unrelated sibling")).expect("the sibling");
    apply_page_md_with_sidecar(&ws, &root, target).expect("target projected");
    apply_page_md_with_sidecar(&ws, &root, source).expect("source projected");

    let workspace = Arc::new(Mutex::new(Some(ws)));
    let writer = ProjectionWriter::spawn(workspace.clone(), root.clone(), |_| {});
    let host = TestHost {
        workspace,
        hlc,
        root,
        registry: Arc::new(RuntimeRegistry::new()),
        backlinks: Arc::new(Mutex::new(None)),
        writer,
    };
    Fixture {
        _tmp: tmp,
        host,
        source,
        citing,
        other,
    }
}

/// What the backlinks section does when it draws a row: read the index
/// from disk, ask it for `target`'s backlinks, and keep the block id the
/// row carries. This is the id the commit is later called with.
fn render_the_row(host: &TestHost) -> NodeId {
    let metas = {
        let guard = host.workspace.lock();
        list_pages(guard.as_ref().expect("the workspace is open"))
    };
    let index = build_backlink_index_from_disk(&metas, &host.root);
    let guard = host.workspace.lock();
    let ws = guard.as_ref().expect("the workspace is open");
    let target = find_by_slug(ws, "target").expect("target exists");
    let meta = page_meta(ws, target).expect("target has meta");
    let links = index.for_page(ws, &meta);
    assert_eq!(links.len(), 1, "exactly one row cites target");
    parse_node_id(&links[0].block_id).expect("the row carries a node id")
}

fn texts(host: &TestHost, page: NodeId) -> Vec<String> {
    let guard = host.workspace.lock();
    let ws = guard.as_ref().expect("the workspace is open");
    children_of(ws, page)
        .into_iter()
        .filter_map(|(id, _)| ws.block_text(id))
        .collect()
}

fn text_of(host: &TestHost, block: NodeId) -> Option<String> {
    let guard = host.workspace.lock();
    guard
        .as_ref()
        .expect("the workspace is open")
        .block_text(block)
}

fn op_count(host: &TestHost) -> usize {
    host.workspace
        .lock()
        .as_ref()
        .expect("the workspace is open")
        .log()
        .len()
}

// ---------------------------------------------------------------------
// 1. A stale id is refused, never re-targeted.
// ---------------------------------------------------------------------

/// The peer deletes the cited block after the row was drawn. The row's
/// id now names a block under the trash, and `edit_block` would recover by
/// appending the text at the end of `notes`. Here the page must come out
/// exactly as the peer left it.
#[test]
fn a_block_a_peer_deleted_after_the_row_rendered_is_refused_and_nothing_is_appended() {
    let f = fixture();
    let row_id = render_the_row(&f.host);
    assert_eq!(row_id, f.citing, "the row was drawn from the live block");

    Peer::new().with(&f.host, |ws, hlc| {
        outl_actions::delete(ws, hlc, f.citing).expect("the peer deletes the block");
    });
    let before_texts = texts(&f.host, f.source);
    let before_ops = op_count(&f.host);
    assert_eq!(
        before_texts,
        vec!["unrelated sibling".to_string()],
        "the peer's delete landed"
    );

    let err = edit_backlink_block(
        &f.host,
        f.source.to_string(),
        row_id.to_string(),
        "typed into a row whose block is gone".into(),
    )
    .expect_err("an edit to a block a peer deleted must be refused");

    assert_eq!(err, stale_backlink_block_text(true));
    assert_eq!(
        err, "That block was deleted, so your text could not be saved.",
        "the wording itself is pinned, not only its source"
    );
    let after = texts(&f.host, f.source);
    assert_eq!(
        after.len(),
        before_texts.len(),
        "the source page gained a block: the text was re-targeted, not refused \
         ({after:?})"
    );
    assert_eq!(
        after.last(),
        before_texts.last(),
        "the end of the source page changed: {after:?}"
    );
    assert!(
        !after.iter().any(|t| t.contains("typed into a row")),
        "the user's text landed on a page they are not looking at: {after:?}"
    );
    assert_eq!(op_count(&f.host), before_ops, "a refusal writes no op");
}

/// The other cause. A page whose sidecar is gone is read through
/// `outline_from_parsed`, which mints a fresh `NodeId` per block, so the
/// row's id was never in the tree at all. Nothing was deleted, so the
/// sentence must not say so.
#[test]
fn an_id_minted_on_read_for_a_page_with_no_sidecar_is_refused_as_not_found() {
    let f = fixture();
    let md_path = f.host.root.join("pages").join("notes.md");
    std::fs::remove_file(outl_md::sidecar::sidecar_path_for(&md_path))
        .expect("the sidecar exists to be removed");

    let transient = render_the_row(&f.host);
    assert_ne!(
        transient, f.citing,
        "with no sidecar the row's id is minted, not the live block's"
    );
    let before_texts = texts(&f.host, f.source);
    let before_ops = op_count(&f.host);

    let err = edit_backlink_block(
        &f.host,
        f.source.to_string(),
        transient.to_string(),
        "typed into a row with a transient id".into(),
    )
    .expect_err("an id that names no block must be refused");

    assert_eq!(err, stale_backlink_block_text(false));
    assert_eq!(
        err,
        "That block could not be found, so your text could not be saved."
    );
    let after = texts(&f.host, f.source);
    assert_eq!(after.len(), before_texts.len(), "a block was appended");
    assert_eq!(after.last(), before_texts.last());
    assert_eq!(op_count(&f.host), before_ops, "a refusal writes no op");
}

// ---------------------------------------------------------------------
// 2. A page ahead of its log is refused before the mutation.
// ---------------------------------------------------------------------

/// Lines on disk that no op holds: the edit would apply to the *logged*
/// text, not the text the row showed. The refusal has to come first, and
/// the proof is the op log, because a refusal after `edit_text` would
/// leave the op durable and the user told it was not saved.
///
/// The peer's op arrives first, so the pre-check has real work to do on a
/// tree that is ahead of the `.md` in the other direction too.
#[test]
fn a_page_ahead_of_its_log_is_refused_before_any_op_is_written() {
    let f = fixture();
    let row_id = render_the_row(&f.host);

    Peer::new().with(&f.host, |ws, hlc| {
        append_block(ws, hlc, Some(f.source), Some("written by a peer")).expect("peer append");
    });

    let md_path = f.host.root.join("pages").join("notes.md");
    let on_disk = "- ask [[target]] about it\n- unrelated sibling\n- only ever on disk\n";
    std::fs::write(&md_path, on_disk).expect("hand-edit the md");
    let sidecar_path = outl_md::sidecar::sidecar_path_for(&md_path);
    let mut sc = outl_md::sidecar::read(&sidecar_path).expect("sidecar");
    sc.last_synced_hash = outl_md::sidecar::file_hash(on_disk);
    outl_md::sidecar::write(&sidecar_path, &sc).expect("restamp");

    let text_before = text_of(&f.host, row_id);
    let texts_before = texts(&f.host, f.source);
    let ops_before = op_count(&f.host);

    let err = edit_backlink_block(
        &f.host,
        f.source.to_string(),
        row_id.to_string(),
        "edited through a backlink row".into(),
    )
    .expect_err("a page ahead of its log cannot take a cross-page edit");

    let expected = ahead_of_log_notice(SURFACE, 1, "- only ever on disk");
    assert!(
        err.contains(expected.title),
        "the shared headline is missing: {err:?}"
    );
    assert!(
        err.contains(expected.command),
        "the shared recovery command is missing: {err:?}"
    );
    assert_eq!(
        op_count(&f.host),
        ops_before,
        "the refusal came after the mutation: the op is already durable"
    );
    assert_eq!(
        text_of(&f.host, row_id),
        text_before,
        "the block's text changed despite the refusal"
    );
    assert_eq!(texts(&f.host, f.source), texts_before);
    assert_eq!(
        std::fs::read_to_string(&md_path).expect("md"),
        on_disk,
        "the bytes the guard protects were overwritten"
    );
}

// ---------------------------------------------------------------------
// 3. A concurrent edit that is not a conflict still lands.
// ---------------------------------------------------------------------

/// A peer edits the sibling and appends a block while the user types in
/// the backlink row. Different blocks, so no conflict: neither refusal may
/// fire, and both sets of edits are in the tree, in the `.md`, and in the
/// reply.
#[test]
fn a_peer_editing_a_different_block_does_not_block_the_backlink_edit() {
    let f = fixture();
    let row_id = render_the_row(&f.host);

    Peer::new().with(&f.host, |ws, hlc| {
        edit_text(ws, hlc, f.other, "unrelated sibling, edited by a peer").expect("peer edit");
        append_block(ws, hlc, Some(f.source), Some("appended by a peer")).expect("peer append");
    });
    let ops_before = op_count(&f.host);

    let view = edit_backlink_block(
        &f.host,
        f.source.to_string(),
        row_id.to_string(),
        "ask [[target]] about it, done".into(),
    )
    .expect("an edit to a different block than the peer's is not a conflict");

    assert!(op_count(&f.host) > ops_before, "the edit wrote an op");
    assert_eq!(
        text_of(&f.host, f.citing).as_deref(),
        Some("ask [[target]] about it, done"),
        "the local edit did not land"
    );
    assert_eq!(
        text_of(&f.host, f.other).as_deref(),
        Some("unrelated sibling, edited by a peer"),
        "the local commit clobbered the peer's edit"
    );
    let tree = texts(&f.host, f.source);
    assert_eq!(
        tree,
        vec![
            "ask [[target]] about it, done".to_string(),
            "unrelated sibling, edited by a peer".to_string(),
            "appended by a peer".to_string(),
        ]
    );
    let reply: Vec<String> = view.outline.iter().map(|n| n.text.clone()).collect();
    assert_eq!(reply, tree, "the reply does not say what the tree says");

    // The projection is what the next open reads; it must carry both
    // sides, and flushing makes the writer's queue observable.
    f.host
        .writer
        .flush()
        .expect("every queued projection landed");
    let md = std::fs::read_to_string(f.host.root.join("pages").join("notes.md")).expect("md");
    assert!(md.contains("ask [[target]] about it, done"), "{md}");
    assert!(md.contains("edited by a peer"), "{md}");
    assert!(md.contains("appended by a peer"), "{md}");
}

/// The peer edits the **same** block. Text is a CRDT, so this is not a
/// refusal case either: the command must succeed and the block must hold
/// a converged string, not the pre-peer one and not an error.
#[test]
fn a_peer_editing_the_same_block_is_merged_not_refused() {
    let f = fixture();
    let row_id = render_the_row(&f.host);

    Peer::new().with(&f.host, |ws, hlc| {
        edit_text(ws, hlc, f.citing, "ask [[target]] about it today").expect("peer edit");
    });

    edit_backlink_block(
        &f.host,
        f.source.to_string(),
        row_id.to_string(),
        "ask [[target]] about it tomorrow".into(),
    )
    .expect("a concurrent edit to the same block is merged by the CRDT");

    let merged = text_of(&f.host, f.citing).expect("the block is live");
    assert!(
        merged.contains("ask [[target]] about it"),
        "the block lost its content: {merged:?}"
    );
    assert!(
        merged.contains("tomorrow"),
        "the local edit did not land: {merged:?}"
    );
}
