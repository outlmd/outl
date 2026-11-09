//! Reading a deletion back through the IPC surface (issue #287).
//!
//! The policy is `outl_actions::trash`'s and is tested there. What these
//! cover is the part only this layer can get wrong: that the listing
//! carries the mutation's own verdict rather than a second opinion, and
//! that a restore whose landing place is gone refuses instead of
//! dropping the block somewhere plausible.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{append_block, open_or_create_page, PageKind, SyncTransport};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::commands::trash::{list_trash, restore_trash_entry};
use outl_tauri_shared::host::AppHost;
use parking_lot::Mutex;
// `list_trash` is `async` because it folds every deletion's parent trail
// out of the op log and must not do that on the IPC thread
// (`outl_tauri_shared::offload`). A plain `#[test]` has no executor, so
// drive it on Tauri's own runtime — same as `block_clipboard_command`.
use tauri::async_runtime::block_on;
use tempfile::TempDir;

/// Minimal [`AppHost`] — no projection writer, no undo stacks, so every
/// mutation projects synchronously and reads back off the `.md`.
struct TestHost {
    workspace: Arc<Mutex<Option<Workspace>>>,
    hlc: HlcGenerator,
    root: PathBuf,
    registry: Arc<RuntimeRegistry>,
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
}

impl TestHost {
    fn new() -> (TempDir, Self) {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let ws = Workspace::open_in_memory(actor).unwrap();
        let host = TestHost {
            workspace: Arc::new(Mutex::new(Some(ws))),
            hlc,
            root: tmp.path().to_path_buf(),
            registry: Arc::new(RuntimeRegistry::new()),
        };
        (tmp, host)
    }

    fn with_ws<T>(&self, f: impl FnOnce(&Workspace) -> T) -> T {
        let guard = self.workspace.lock();
        f(guard.as_ref().unwrap())
    }

    fn mutate<T>(&self, f: impl FnOnce(&mut Workspace, &HlcGenerator) -> T) -> T {
        let mut guard = self.workspace.lock();
        let hlc = self.hlc.clone();
        f(guard.as_mut().unwrap(), &hlc)
    }

    fn page(&self, slug: &str) -> NodeId {
        self.mutate(|ws, hlc| open_or_create_page(ws, hlc, slug, slug, PageKind::Page).unwrap())
    }

    fn block(&self, parent: NodeId, text: &str) -> NodeId {
        self.mutate(|ws, hlc| append_block(ws, hlc, Some(parent), Some(text)).unwrap())
    }

    fn trash(&self, node: NodeId) {
        self.mutate(|ws, hlc| outl_actions::delete(ws, hlc, node).unwrap());
    }
}

/// Every node the tree still holds, so a refused mutation can be shown
/// to have written nothing rather than merely to have returned an error.
fn live_nodes(ws: &Workspace) -> Vec<NodeId> {
    let mut ids: Vec<NodeId> = ws.tree().iter_nodes().map(|(node, _, _)| node).collect();
    ids.sort();
    ids
}

/// Invariant 8's shape, one layer out: the row the user clicks and the
/// mutation that runs have to answer "can this be restored" from the
/// same owner.
///
/// Asserted against `trash::refusal_for` per row rather than against a
/// literal, because a literal passes just as happily once the DTO grows
/// its own copy of the rule — which is the drift being prevented.
#[test]
fn a_listed_entry_carries_the_refusal_a_restore_would_return() {
    let (_tmp, host) = TestHost::new();
    let page = host.page("notas");

    // Two shapes in one listing: a loose block whose parent is still
    // live (restorable) and a block whose parent went to the trash after
    // it did (refused — the parent is not there to land under).
    let loose = host.block(page, "solto");
    let section = host.block(page, "seção");
    let child = host.block(section, "filho");
    host.trash(child);
    host.trash(loose);
    host.trash(section);

    let rows = block_on(list_trash(&host)).expect("listing");
    assert!(!rows.is_empty(), "three deletions happened");

    host.with_ws(|ws| {
        for row in &rows {
            let node = NodeId(row.id.parse().expect("a listed id parses"));
            let owner = outl_actions::trash::refusal_for(ws, node).map(|e| e.to_string());
            assert_eq!(
                row.refusal, owner,
                "row {} carries its own verdict instead of trash::refusal_for's",
                row.id
            );
        }
    });

    // And the rows really do disagree about something, or the loop above
    // would be comparing `None` with `None` and proving nothing.
    assert!(
        rows.iter().any(|r| r.refusal.is_some()),
        "the child's parent is trashed, so at least one row must refuse"
    );
    assert!(
        rows.iter().any(|r| r.refusal.is_none()),
        "the loose block is restorable, so at least one row must not refuse"
    );
}

/// A restore whose landing place is itself deleted has to say so, not
/// drop the block somewhere plausible.
///
/// Deleting the page the block came from is the realistic way in — the
/// user tidies up, then changes their mind about one block.
#[test]
fn restoring_into_a_deleted_page_is_refused_not_orphaned() {
    let (_tmp, host) = TestHost::new();
    let page = host.page("arquivo");
    let block = host.block(page, "o bloco");

    host.trash(block);
    host.mutate(|ws, hlc| outl_actions::delete(ws, hlc, page).unwrap());

    let before = host.with_ws(live_nodes);

    let err = restore_trash_entry(&host, block.to_string())
        .expect_err("the page it came from is in the trash");
    assert!(
        err.contains(&page.to_string()),
        "the refusal has to name where the content still is, got: {err}"
    );

    host.with_ws(|ws| {
        assert!(
            outl_actions::is_trashed(ws, block),
            "a refused restore leaves the block where it was"
        );
        assert!(
            !outl_actions::children_of(ws, NodeId::root())
                .iter()
                .any(|(node, _)| *node == block),
            "nothing landed under root() — that is the orphan this refuses"
        );
        assert_eq!(
            live_nodes(ws),
            before,
            "a refused restore writes nothing at all"
        );
    });
}

/// The ordinary path, which is what makes the two refusal assertions
/// above mean something: a restorable row restores, and the page it
/// lands on comes back projected so the client can show the user where
/// their block went.
#[test]
fn a_restored_block_comes_back_with_its_page() {
    let (_tmp, host) = TestHost::new();
    let page = host.page("diario");
    let block = host.block(page, "voltei");
    host.trash(block);

    let reply = restore_trash_entry(&host, block.to_string()).expect("restorable");

    assert_eq!(reply.id, block.to_string());
    assert_eq!(reply.blocks, 1);
    let view = reply
        .view
        .expect("it landed on a page, so the page comes back refreshed");
    assert_eq!(view.page.id, page.to_string());

    host.with_ws(|ws| {
        assert!(
            !outl_actions::is_trashed(ws, block),
            "the block is out of the trash"
        );
        assert!(
            outl_actions::children_of(ws, page)
                .iter()
                .any(|(node, _)| *node == block),
            "and back under the page it was deleted from"
        );
    });

    assert!(
        block_on(list_trash(&host)).expect("listing").is_empty(),
        "a restored block is no longer a deletion that can be restored"
    );
}

/// The listing reports when each deletion happened, so a panel can tell
/// a block deleted a minute ago from one deleted last year.
///
/// Pinned here as well as in `outl-actions` because the DTO is where the
/// field could silently stop being carried.
#[test]
fn the_listing_reports_when_each_block_was_deleted() {
    let (_tmp, host) = TestHost::new();
    let page = host.page("quando");
    let block = host.block(page, "apagado");

    let before = host.hlc.next().physical_ms;
    host.trash(block);
    let after = host.hlc.next().physical_ms;

    let rows = block_on(list_trash(&host)).expect("listing");
    let row = rows.iter().find(|r| r.id == block.to_string()).unwrap();
    let at = row
        .deleted_at_ms
        .expect("the op log holds the move that trashed it");
    assert!(
        (before..=after).contains(&at),
        "the instant has to be the trashing move's own HLC: {at} outside {before}..={after}"
    );
}
