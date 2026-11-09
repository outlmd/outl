//! `restore_trash_entry` resolves the landing page under a **read**
//! lock and mutates under a **write** lock, so a peer re-parenting the
//! origin block in that window commits the restore against a page that
//! is no longer the one it lands on.
//!
//! The command's own doc says the op log stays correct and the `.md`
//! lags. These pin both halves of that claim, because the second half
//! is only acceptable *because* of the first — a projection aimed at
//! the wrong page is a bookkeeping slip; an `Op::Move` aimed at the
//! wrong parent is content in the wrong place forever.
//!
//! The window is not schedulable from a test (both acquisitions are
//! inside one function), so these drive its effect: commit through
//! `finish_in_page_with` with the page a stale read would have
//! produced, and inspect the op log, the tree and the `.md`.
//!
//! What is deliberately **not** asserted is that the landing page's
//! `.md` is stale afterwards. That is today's behaviour and it is the
//! defect, not the contract; pinning it would fail the fix. What is
//! asserted is the property that has to hold either way: an ordinary
//! stale re-projection recovers it without an invariant-8 refusal.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::block::move_under;
use outl_actions::journal::{
    apply_page_md_with_sidecar, apply_page_md_with_sidecar_if_stale, page_md_path,
};
use outl_actions::{append_block, open_or_create_page, PageKind, SyncTransport};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::Op;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::helpers::finish_in_page_with;
use outl_tauri_shared::host::AppHost;
use parking_lot::Mutex;
use tempfile::TempDir;

/// Minimal [`AppHost`] — no projection writer and no undo stacks, so
/// every commit projects synchronously and the `.md` on disk is the
/// commit's own output rather than a queue's.
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
        let tmp = TempDir::new().expect("tmp");
        let actor = ActorId::new();
        let host = TestHost {
            workspace: Arc::new(Mutex::new(Some(
                Workspace::open_in_memory(actor).expect("workspace"),
            ))),
            hlc: HlcGenerator::new(actor),
            root: tmp.path().to_path_buf(),
            registry: Arc::new(RuntimeRegistry::new()),
        };
        (tmp, host)
    }

    fn with_ws<T>(&self, f: impl FnOnce(&Workspace) -> T) -> T {
        let guard = self.workspace.lock();
        f(guard.as_ref().expect("open"))
    }

    fn mutate<T>(&self, f: impl FnOnce(&mut Workspace, &HlcGenerator) -> T) -> T {
        let mut guard = self.workspace.lock();
        let hlc = self.hlc.clone();
        f(guard.as_mut().expect("open"), &hlc)
    }

    fn page(&self, slug: &str) -> NodeId {
        self.mutate(|ws, hlc| {
            open_or_create_page(ws, hlc, slug, slug, PageKind::Page).expect("page")
        })
    }

    fn md(&self, page: NodeId) -> String {
        let path = self.with_ws(|ws| {
            page_md_path(
                &self.root,
                &outl_actions::page_meta(ws, page).expect("meta"),
            )
        });
        std::fs::read_to_string(path).expect("read the projection")
    }

    /// What `restore_trash_entry` reads under its read lock.
    ///
    /// Calls the function the command calls, rather than re-typing it.
    /// This used to be a copy of the old inline resolution, and it went
    /// stale the moment that moved into `trash::landing_page` and gained
    /// a `page_meta` check — leaving a helper whose doc said "what the
    /// command reads" while reading something else.
    fn resolve_landing_page(&self, node: NodeId) -> Option<NodeId> {
        self.with_ws(|ws| outl_actions::trash::landing_page(ws, node))
    }
}

/// The newest `Move` naming `node` — the op log's own answer to "where
/// did the restore put it", independent of the tree.
fn last_move_target(ws: &Workspace, node: NodeId) -> Option<NodeId> {
    ws.log()
        .iter()
        .filter_map(|logged| match &logged.op {
            Op::Move {
                node: moved,
                new_parent,
                ..
            } if *moved == node => Some(*new_parent),
            _ => None,
        })
        .last()
}

/// A peer moves the origin block to another page inside the window.
#[test]
fn a_landing_that_moved_pages_mid_restore_still_lands_in_the_right_parent() {
    let (_tmp, host) = TestHost::new();
    let root = host.root.clone();
    let page_one = host.page("um");
    let page_two = host.page("dois");
    let origin = host
        .mutate(|ws, hlc| append_block(ws, hlc, Some(page_one), Some("origem")).expect("origin"));
    let block =
        host.mutate(|ws, hlc| append_block(ws, hlc, Some(origin), Some("o bloco")).expect("block"));

    host.mutate(|ws, hlc| outl_actions::delete(ws, hlc, block).expect("delete"));
    host.with_ws(|ws| {
        apply_page_md_with_sidecar(ws, &root, page_one).expect("project um");
        apply_page_md_with_sidecar(ws, &root, page_two).expect("project dois");
    });
    assert_eq!(host.resolve_landing_page(block), Some(page_one));

    // ---- the window.
    host.mutate(|ws, hlc| move_under(ws, hlc, origin, page_two).expect("peer re-parent"));

    // ---- the commit, against the page the stale read produced.
    let (_entry, view) = finish_in_page_with(&host, page_one, |ws| {
        outl_actions::trash::restore(ws, &host.hlc.clone(), block)
    })
    .expect("the restore itself still lands");
    assert_eq!(
        view.page.id,
        page_one.to_string(),
        "the stale page committed"
    );

    // The op log is right, which is the half that matters.
    host.with_ws(|ws| {
        assert_eq!(
            last_move_target(ws, block),
            Some(origin),
            "the `Move` names the origin the write lock re-resolved"
        );
        assert_eq!(ws.tree().parent(block), Some(origin));
        assert_eq!(
            outl_actions::enclosing_page_id(ws, block),
            Some(page_two),
            "so the block really is on the other page now"
        );
    });

    // And the `.md` is recoverable by the ordinary stale pass, with no
    // invariant-8 refusal in the way.
    eprintln!(
        "pages/dois.md right after the commit:\n{}",
        host.md(page_two)
    );
    let written = host.with_ws(|ws| {
        apply_page_md_with_sidecar_if_stale(ws, &root, page_two).expect("no invariant-8 refusal")
    });
    assert!(written.is_some(), "the stale pass had work to do");
    assert!(
        host.md(page_two).contains("o bloco"),
        "and after it the `.md` agrees with the op log:\n{}",
        host.md(page_two)
    );
}

/// The other half of the same window: a peer *trashes* the origin
/// between the read and the write. The mutation re-asks the verdict
/// under its own lock, so it refuses — having written nothing.
#[test]
fn a_landing_trashed_inside_the_window_refuses_and_writes_nothing() {
    let (_tmp, host) = TestHost::new();
    let root = host.root.clone();
    let page = host.page("um");
    let origin =
        host.mutate(|ws, hlc| append_block(ws, hlc, Some(page), Some("origem")).expect("origin"));
    let block =
        host.mutate(|ws, hlc| append_block(ws, hlc, Some(origin), Some("bloco")).expect("block"));
    host.mutate(|ws, hlc| outl_actions::delete(ws, hlc, block).expect("delete"));
    host.with_ws(|ws| {
        apply_page_md_with_sidecar(ws, &root, page).expect("project");
    });
    assert_eq!(host.resolve_landing_page(block), Some(page));

    // ---- the window.
    host.mutate(|ws, hlc| outl_actions::delete(ws, hlc, origin).expect("peer delete"));

    let before_log = host.with_ws(|ws| ws.log().len());
    let err = finish_in_page_with(&host, page, |ws| {
        outl_actions::trash::restore(ws, &host.hlc.clone(), block)
    })
    .expect_err("the origin is in the trash now");

    assert!(
        err.contains(&origin.to_string()),
        "the refusal names where the content still is: {err}"
    );
    host.with_ws(|ws| {
        assert_eq!(ws.log().len(), before_log, "no op was appended");
        assert_eq!(ws.tree().parent(block), Some(NodeId::trash()));
    });
}
