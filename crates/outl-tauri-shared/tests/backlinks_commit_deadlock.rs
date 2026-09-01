//! A local mutation and a backlinks refresh must never deadlock.
//!
//! Every GUI commit funnels through `finish_in_page`, which holds the
//! **workspace** lock and then drops the cached backlinks index
//! (`invalidate_backlink_index` → **index** lock). `page_backlinks` runs
//! off-thread and used to do the opposite: take the **index** lock and
//! then, inside `finish_lookup`, the **workspace** lock. Two locks taken
//! in opposite orders is an ABBA deadlock, and `parking_lot::Mutex` has
//! no timeout — the app freezes for good and the user force-quits it.
//!
//! The frontend fires both together on every paste (`applyView` refreshes
//! backlinks after each commit, and a paste commits twice: the draft flush
//! plus the paste itself), which is why pasting was the reliable way in.
//!
//! The guard is a stress loop with a watchdog: on the buggy ordering the
//! threads park forever and the watchdog fails the test instead of hanging
//! the suite.

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use outl_actions::{
    append_block, apply_page_md_with_sidecar, edit_text, open_or_create_page, PageKind,
    SyncTransport,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::commands::page::page_backlinks;
use outl_tauri_shared::helpers::finish_in_page;
use outl_tauri_shared::host::AppHost;
use parking_lot::Mutex;
use tempfile::TempDir;

struct TestHost {
    workspace: Arc<Mutex<Option<Workspace>>>,
    hlc: HlcGenerator,
    root: PathBuf,
    registry: Arc<RuntimeRegistry>,
    backlinks: Arc<Mutex<Option<outl_actions::BacklinkIndex>>>,
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
    fn backlink_index(&self) -> Option<Arc<Mutex<Option<outl_actions::BacklinkIndex>>>> {
        Some(self.backlinks.clone())
    }
}

#[test]
fn a_commit_and_a_backlinks_refresh_never_deadlock() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);

    let storage = JsonlStorage::open(root.join("ops"), actor).unwrap();
    let mut ws =
        Workspace::open_with_storage(actor, Box::new(storage), Some(root.clone())).unwrap();

    // A page that is referenced from elsewhere, so the backlinks lookup
    // has real work to do (and the index is worth caching).
    let target = open_or_create_page(&mut ws, &hlc, "target", "Target", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(target), Some("target block")).unwrap();
    let src = open_or_create_page(&mut ws, &hlc, "src", "Src", PageKind::Page).unwrap();
    let block = append_block(&mut ws, &hlc, Some(src), Some("see [[target]]")).unwrap();
    apply_page_md_with_sidecar(&ws, &root, target).unwrap();
    apply_page_md_with_sidecar(&ws, &root, src).unwrap();

    let host = Arc::new(TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root,
        registry: Arc::new(RuntimeRegistry::with_builtins()),
        backlinks: Arc::new(Mutex::new(None)),
    });

    let (done_tx, done_rx) = mpsc::channel::<&'static str>();

    // Thread A: the commit path — the workspace lock, then the index.
    let commits = {
        let host = host.clone();
        let done = done_tx.clone();
        std::thread::spawn(move || {
            for i in 0..200 {
                let text = format!("see [[target]] {i}");
                let _ = finish_in_page(host.as_ref(), src, |ws| {
                    edit_text(ws, host.hlc(), block, &text).map(|_| ())
                });
            }
            let _ = done.send("commits");
        })
    };

    // Thread B: the backlinks path — the index lock, then the workspace.
    let lookups = {
        let host = host.clone();
        let done = done_tx.clone();
        std::thread::spawn(move || {
            for _ in 0..200 {
                let _ = tauri::async_runtime::block_on(page_backlinks(
                    host.as_ref(),
                    "target".to_string(),
                ));
            }
            let _ = done.send("backlinks");
        })
    };

    // Watchdog: a deadlock parks both threads forever, so turn "never
    // finished" into a failing assertion instead of a hung test binary.
    let mut finished = 0;
    while finished < 2 {
        match done_rx.recv_timeout(Duration::from_secs(60)) {
            Ok(_) => finished += 1,
            Err(_) => panic!(
                "deadlock: a commit and a backlinks refresh blocked each other \
                 (workspace/backlink-index lock order inversion)"
            ),
        }
    }

    commits.join().unwrap();
    lookups.join().unwrap();
    let _: Option<NodeId> = None;
}
