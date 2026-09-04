//! `outl_tauri_shared::commands::history` — generic over [`AppHost`], so
//! any client whose `history()` returns `Some` gets vim-style undo/redo
//! (RFC 0254 phase 1: this used to be desktop-only code in
//! `outl-desktop/src-tauri/src/commands/history.rs`).
//!
//! `crates/outl-actions/src/history.rs` already pins `HistoryStacks` +
//! `restore_page_md` in isolation; these pin the **command wiring** a
//! real client depends on: recording through the same `finish_in_page`
//! path a mutation command uses, keying stacks per page, the exact
//! error strings a frontend matches on, and the "this host never wired
//! a history slot" branch every non-desktop `AppHost` used to hit
//! silently before this RFC.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{
    append_block, open_or_create_page as open_or_create, render_page_md, HistoryStacks, PageKind,
    SyncTransport,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::commands::block::edit_block;
use outl_tauri_shared::commands::history::{redo_page, undo_page};
use outl_tauri_shared::host::AppHost;
use outl_tauri_shared::ProjectionWriter;
use parking_lot::Mutex;
use tempfile::TempDir;

/// A host that supports undo — the shape every `AppHost::history()`
/// implementation is `Some`.
struct HistoryHost {
    workspace: Arc<Mutex<Option<Workspace>>>,
    hlc: HlcGenerator,
    root: PathBuf,
    registry: Arc<RuntimeRegistry>,
    history: Mutex<HashMap<NodeId, HistoryStacks<String>>>,
}

impl AppHost for HistoryHost {
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
    fn history(&self) -> Option<&Mutex<HashMap<NodeId, HistoryStacks<String>>>> {
        Some(&self.history)
    }
}

/// A host that never wired a history slot — the default every `AppHost`
/// gets until it overrides `history()`. Mobile was exactly this before
/// RFC 0254 phase 1.
struct NoHistoryHost {
    workspace: Arc<Mutex<Option<Workspace>>>,
    hlc: HlcGenerator,
    root: PathBuf,
    registry: Arc<RuntimeRegistry>,
}

impl AppHost for NoHistoryHost {
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
    // `history()` deliberately left at the trait default (`None`).
}

/// A host wired exactly like a real GUI client: a real background
/// `ProjectionWriter` sharing the same workspace `Arc` every command
/// locks (not a synchronous stand-in) — the same shape both
/// `outl-desktop` and `outl-mobile` build at boot. `HistoryHost` above
/// has no `projection_writer()` override, so it always takes
/// `finish_in_page_with`'s synchronous branch; that never exercises the
/// bug this host exists to pin.
struct AsyncHistoryHost {
    workspace: Arc<Mutex<Option<Workspace>>>,
    hlc: HlcGenerator,
    root: PathBuf,
    registry: Arc<RuntimeRegistry>,
    history: Mutex<HashMap<NodeId, HistoryStacks<String>>>,
    projection_writer: ProjectionWriter,
}

impl AppHost for AsyncHistoryHost {
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
    fn history(&self) -> Option<&Mutex<HashMap<NodeId, HistoryStacks<String>>>> {
        Some(&self.history)
    }
    fn projection_writer(&self) -> Option<&ProjectionWriter> {
        Some(&self.projection_writer)
    }
}

/// An `AsyncHistoryHost` with one page (`ideas`) holding a single block
/// (`"one"`), plus that block's id. Mirrors `host_with_page` above.
fn async_host_with_page() -> (TempDir, AsyncHistoryHost, NodeId, String) {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let page = open_or_create(&mut ws, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
    let block = append_block(&mut ws, &hlc, Some(page), Some("one")).unwrap();

    let workspace = Arc::new(Mutex::new(Some(ws)));
    let projection_writer =
        ProjectionWriter::spawn(workspace.clone(), tmp.path().to_path_buf(), |_| {});
    let host = AsyncHistoryHost {
        workspace,
        hlc,
        root: tmp.path().to_path_buf(),
        registry: Arc::new(RuntimeRegistry::new()),
        history: Mutex::new(HashMap::new()),
        projection_writer,
    };
    (tmp, host, page, block.to_string())
}

/// The bug this phase's coordinator asked to have fixed at the source
/// rather than papered over in tests: `edit_block` only *queues* its
/// `.md` + sidecar write when a real `ProjectionWriter` is wired (the
/// async-writes default both GUI clients use), and `undo_page`
/// reconciles its restore against whatever sidecar is *currently on
/// disk*. With no flush between the two, `undo_page` can win the race
/// against the background writer, find no sidecar (or a stale one) to
/// match "two" against, and create a **second** block for "one" instead
/// of replacing it — a real duplicated-content bug a user hits by
/// editing a block and hitting undo quickly, not a test artifact.
///
/// No explicit flush anywhere in this test: `step_history` itself must
/// call `ProjectionWriter::flush()` before reconciling. Delete that call
/// and this test starts failing — confirmed by reverting it locally and
/// running in a loop: most runs still passed (the background write
/// often wins the race on this machine), but a real minority reproduced
/// the exact duplicate this asserts against, `"- two\n- one\n"` instead
/// of `"- one\n"`, plus one run that hit a second, unrelated symptom of
/// the same missing synchronization (`reconcile_md` erroring on a
/// missing `orphans.log` path). Scheduling-dependent, not deterministic
/// — which is the whole reason a flush belongs in the source, not a
/// hope that CI happens to schedule kindly.
#[test]
fn undo_immediately_after_an_edit_does_not_duplicate_the_block() {
    let (_tmp, host, page, block_id) = async_host_with_page();
    let page_id = page.to_string();
    assert_eq!(page_text(&host, page), "title:: Ideas\n\n- one\n");

    edit_block(&host, page_id.clone(), block_id, "two".into()).expect("edit_block");
    // Deliberately no `flush()` call here — see the doc comment above.
    let view = undo_page(&host, page_id).expect("undo_page");

    assert_eq!(
        page_text(&host, page),
        "title:: Ideas\n\n- one\n",
        "reconcile must replace the edited block, not add a second one"
    );
    assert_eq!(
        view.outline.len(),
        1,
        "the page must still have exactly one block"
    );
    assert_eq!(view.outline[0].text, "one");
}

/// Same shape, the other direction: redo immediately after undo, with
/// no flush, must replay "two" without leaving "one" behind as an extra
/// block.
#[test]
fn redo_immediately_after_undo_does_not_duplicate_the_block() {
    let (_tmp, host, page, block_id) = async_host_with_page();
    let page_id = page.to_string();
    edit_block(&host, page_id.clone(), block_id, "two".into()).expect("edit_block");

    undo_page(&host, page_id.clone()).expect("undo_page");
    let view = redo_page(&host, page_id).expect("redo_page");

    assert_eq!(page_text(&host, page), "title:: Ideas\n\n- two\n");
    assert_eq!(
        view.outline.len(),
        1,
        "the page must still have exactly one block"
    );
    assert_eq!(view.outline[0].text, "two");
}

/// A host with one page (`ideas`) holding a single block (`"one"`), plus
/// that block's id.
fn host_with_page() -> (TempDir, HistoryHost, NodeId, String) {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let page = open_or_create(&mut ws, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
    let block = append_block(&mut ws, &hlc, Some(page), Some("one")).unwrap();

    let host = HistoryHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root: tmp.path().to_path_buf(),
        registry: Arc::new(RuntimeRegistry::new()),
        history: Mutex::new(HashMap::new()),
    };
    (tmp, host, page, block.to_string())
}

fn page_text<H: AppHost>(host: &H, page: NodeId) -> String {
    let ws = host.workspace().lock();
    render_page_md(ws.as_ref().unwrap(), page)
}

#[test]
fn undo_restores_the_pre_edit_snapshot() {
    let (_tmp, host, page, block_id) = host_with_page();
    let page_id = page.to_string();

    edit_block(&host, page_id.clone(), block_id, "two".into()).expect("edit_block");
    assert_eq!(page_text(&host, page), "title:: Ideas\n\n- two\n");

    let view = undo_page(&host, page_id).expect("undo_page");
    assert_eq!(page_text(&host, page), "title:: Ideas\n\n- one\n");
    assert_eq!(view.outline.len(), 1);
    assert_eq!(view.outline[0].text, "one");
}

#[test]
fn redo_replays_the_edit_undo_reverted() {
    let (_tmp, host, page, block_id) = host_with_page();
    let page_id = page.to_string();
    edit_block(&host, page_id.clone(), block_id, "two".into()).expect("edit_block");

    undo_page(&host, page_id.clone()).expect("undo_page");
    redo_page(&host, page_id).expect("redo_page");
    assert_eq!(page_text(&host, page), "title:: Ideas\n\n- two\n");
}

#[test]
fn failed_restore_does_not_consume_or_corrupt_undo_redo() {
    let (tmp, mut host, page, block_id) = host_with_page();
    let page_id = page.to_string();
    edit_block(&host, page_id.clone(), block_id, "two".into()).expect("edit_block");

    host.root = tmp.path().join("missing").join("workspace");
    assert!(undo_page(&host, page_id.clone()).is_err());
    assert_eq!(page_text(&host, page), "title:: Ideas\n\n- two\n");

    host.root = tmp.path().to_path_buf();
    undo_page(&host, page_id.clone()).expect("retry undo");
    assert_eq!(page_text(&host, page), "title:: Ideas\n\n- one\n");

    host.root = tmp.path().join("missing").join("workspace");
    assert!(redo_page(&host, page_id.clone()).is_err());
    assert_eq!(page_text(&host, page), "title:: Ideas\n\n- one\n");

    host.root = tmp.path().to_path_buf();
    redo_page(&host, page_id).expect("retry redo");
    assert_eq!(page_text(&host, page), "title:: Ideas\n\n- two\n");
}

#[test]
fn undo_with_an_empty_stack_names_itself_in_the_error() {
    let (_tmp, host, page, _block_id) = host_with_page();
    let err = undo_page(&host, page.to_string()).unwrap_err();
    assert_eq!(err, "nothing to undo");
}

#[test]
fn redo_with_an_empty_stack_names_itself_in_the_error() {
    let (_tmp, host, page, _block_id) = host_with_page();
    let err = redo_page(&host, page.to_string()).unwrap_err();
    assert_eq!(err, "nothing to redo");
}

#[test]
fn a_new_edit_after_undo_branches_history_and_clears_redo() {
    let (_tmp, host, page, block_id) = host_with_page();
    let page_id = page.to_string();
    edit_block(&host, page_id.clone(), block_id.clone(), "two".into()).unwrap();
    undo_page(&host, page_id.clone()).unwrap();

    edit_block(&host, page_id.clone(), block_id, "three".into()).unwrap();
    let err = redo_page(&host, page_id).unwrap_err();
    assert_eq!(err, "nothing to redo");
}

/// The state a mobile `AppHost` was in before RFC 0254 phase 1 — and
/// what any future `AppHost` without a wired history slot still gets:
/// a clear error instead of a panic on the `Option::unwrap` a naive
/// implementation would reach for.
#[test]
fn a_host_with_no_history_slot_refuses_instead_of_panicking() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let page = open_or_create(&mut ws, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("one")).unwrap();

    let host = NoHistoryHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root: tmp.path().to_path_buf(),
        registry: Arc::new(RuntimeRegistry::new()),
    };

    let err = undo_page(&host, page.to_string()).unwrap_err();
    assert_eq!(err, "undo is not supported on this client");
    let err = redo_page(&host, page.to_string()).unwrap_err();
    assert_eq!(err, "undo is not supported on this client");
}
