//! A reload must never publish a tree that is missing an op the live
//! workspace already applied.
//!
//! The replay runs on a blocking pool thread, outside the workspace
//! mutex, so an ordinary keystroke can commit — appending to
//! `ops-<actor>.jsonl` — after `JsonlStorage::open` has scanned the log
//! and before the fresh workspace is swapped in. The op stays on disk and
//! vanishes from the materialized tree, and the next projection renders
//! that tree over the page's `.md`. Invariant 8's guard does not catch it:
//! the sidecar still lists the block, so the file's lines are all
//! "known to the log" as far as the guard can tell, and the line is
//! deleted from disk.
//!
//! Nothing here stages a probabilistic race: the replay closure lands the
//! edit itself, in exactly the window the real bug uses.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use outl_actions::{
    append_block, open_or_create_page, render_page_md, HistoryStacks, PageKind, SyncTransport,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::host::AppHost;
use outl_tauri_shared::workspace_open::{open_workspace_at, WorkspaceGuards};
use outl_tauri_shared::workspace_reload::{
    publish_replayed, reload_workspace_into, replay_from_disk, RELOAD_ATTEMPTS,
};
use parking_lot::Mutex;
use tempfile::TempDir;

/// A client wired the way both GUI clients are: one workspace slot behind
/// an `Arc`, undo stacks, a real on-disk root.
struct TestHost {
    workspace: Arc<Mutex<Option<Workspace>>>,
    hlc: HlcGenerator,
    root: PathBuf,
    registry: Arc<RuntimeRegistry>,
    history: Mutex<HashMap<NodeId, HistoryStacks<String>>>,
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
    fn history(&self) -> Option<&Mutex<HashMap<NodeId, HistoryStacks<String>>>> {
        Some(&self.history)
    }
}

/// The text a local edit lands mid-replay. Distinct enough that finding
/// it in the published page can only mean the op survived.
const MID_REPLAY: &str = "typed while the op log was replaying";

/// Open a real on-disk workspace with one page, and hand back the host
/// plus that page's id. The guard slot is kept alive by the caller for
/// the lifetime of the test (dropping it releases the flocks).
fn host_with_a_page(
    root: &std::path::Path,
    guards: &Mutex<Option<WorkspaceGuards>>,
) -> (TestHost, NodeId) {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = open_workspace_at(actor, &hlc, root, 0, guards).expect("open workspace");
    let page =
        open_or_create_page(&mut ws, &hlc, "alpha", "Alpha", PageKind::Page).expect("create page");
    append_block(&mut ws, &hlc, Some(page), Some("already in the log")).expect("seed block");
    let host = TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root: root.to_path_buf(),
        registry: Arc::new(RuntimeRegistry::new()),
        history: Mutex::new(HashMap::new()),
    };
    (host, page)
}

/// A replay that lands a local edit on the live workspace on each of its
/// first `edits` calls, *after* reading the log off disk — the window the
/// offloaded reload actually leaves open.
fn racing_replay(
    host: &TestHost,
    page: NodeId,
    edits: usize,
    calls: Arc<AtomicUsize>,
) -> impl Fn() -> Result<Workspace, String> + Clone + Send + 'static {
    let root = host.root.clone();
    let hlc = host.hlc.clone();
    let live = host.workspace.clone();
    move || {
        let fresh = replay_from_disk(root.clone(), hlc.clone())?;
        if calls.fetch_add(1, Ordering::SeqCst) < edits {
            let mut slot = live.lock();
            let ws = slot.as_mut().expect("live workspace");
            append_block(ws, &hlc, Some(page), Some(MID_REPLAY)).expect("the user's edit");
        }
        Ok(fresh)
    }
}

/// The headline case: the edit is still there after the reload publishes.
#[test]
fn an_edit_that_lands_mid_replay_survives_the_publish() {
    let dir = TempDir::new().expect("tempdir");
    let guards = Mutex::new(None);
    let (host, page) = host_with_a_page(dir.path(), &guards);

    let calls = Arc::new(AtomicUsize::new(0));
    let replay = racing_replay(&host, page, 1, calls.clone());

    tauri::async_runtime::block_on(publish_replayed(&host, RELOAD_ATTEMPTS, replay))
        .expect("the reload publishes once it stops losing the race");

    let slot = host.workspace.lock();
    let published = slot.as_ref().expect("a workspace was published");
    assert!(
        render_page_md(published, page).contains(MID_REPLAY),
        "the published tree dropped an op the live workspace had already applied"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "the losing attempt must be discarded and replayed, not published"
    );
}

/// The give-up path publishes nothing. A tree that is missing a local op
/// must never reach the slot, however many times the replay loses — a
/// stale workspace costs a round of peer ops, a lossy one costs the
/// user's text.
#[test]
fn a_reload_that_never_wins_the_race_leaves_the_workspace_alone() {
    let dir = TempDir::new().expect("tempdir");
    let guards = Mutex::new(None);
    let (host, page) = host_with_a_page(dir.path(), &guards);

    let calls = Arc::new(AtomicUsize::new(0));
    let replay = racing_replay(&host, page, usize::MAX, calls.clone());

    let err = tauri::async_runtime::block_on(publish_replayed(&host, RELOAD_ATTEMPTS, replay))
        .expect_err("a reload that keeps losing must refuse, not publish a lossy tree");
    assert!(
        err.contains("lost") && err.contains("races"),
        "the refusal has to say what happened, got {err}"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        RELOAD_ATTEMPTS,
        "the retry is bounded"
    );

    let slot = host.workspace.lock();
    let live = slot.as_ref().expect("the live workspace is still there");
    let md = render_page_md(live, page);
    assert_eq!(
        md.matches(MID_REPLAY).count(),
        RELOAD_ATTEMPTS,
        "every edit made during the attempts must still be in the published tree"
    );
}

/// And the mechanism still does its job: an uncontended reload picks up
/// what another actor wrote into the same workspace. Without this the
/// checks above could pass by never publishing anything.
#[test]
fn an_uncontended_reload_still_picks_up_a_peer() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    let guards = Mutex::new(None);
    let (host, page) = host_with_a_page(root, &guards);

    // A second actor writes its own `ops-<actor>.jsonl` into the same
    // workspace — what a peer's sync ingest leaves behind.
    {
        let peer_actor = ActorId::new();
        let peer_hlc = HlcGenerator::new(peer_actor);
        let peer_guards = Mutex::new(None);
        let mut peer =
            open_workspace_at(peer_actor, &peer_hlc, root, 0, &peer_guards).expect("peer open");
        append_block(&mut peer, &peer_hlc, Some(page), Some("written by a peer")).expect("peer op");
    }

    tauri::async_runtime::block_on(reload_workspace_into(&host)).expect("reload");

    let slot = host.workspace.lock();
    let published = slot.as_ref().expect("published");
    assert!(
        render_page_md(published, page).contains("written by a peer"),
        "the reload published a tree without the peer's op"
    );
}

/// Both clients run *this* reload, not one each.
///
/// A half-fix across two clients is the partial-coverage shape root
/// `CLAUDE.md` invariant 12 warns about, and the client that silently
/// kept the bug would be the one nobody looked at again. The guard is a
/// source check because there is nothing else that can fail: a client
/// calling `SyncEngine::reload_workspace` directly compiles perfectly
/// well and just quietly reopens the window.
#[test]
fn neither_client_replays_the_op_log_on_its_own() {
    const CLIENTS: &[&str] = &[
        "../outl-desktop/src-tauri/src/commands/workspace.rs",
        "../outl-mobile/src-tauri/src/commands/workspace.rs",
    ];
    for relative in CLIENTS {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        assert!(
            src.contains("reload_workspace_into("),
            "{relative} must reload through the shared publish-when-safe path"
        );
        assert!(
            !src.contains(".reload_workspace("),
            "{relative} replays the op log itself; the swap there is unguarded"
        );
    }
}
