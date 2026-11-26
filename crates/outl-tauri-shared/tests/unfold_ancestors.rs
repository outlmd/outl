//! `unfold_ancestors`: the reveal half of "open this page at that block".
//!
//! Three things have to hold, each a different way to land on a block
//! that is still hidden or to write for no reason:
//!
//! - the reply is the page's outline with the folded ancestors **open**,
//!   and the op log holds one `SetCollapsed(false)` per folded block;
//! - a block that is not live on the page (another page's, trashed) is
//!   refused with the log untouched;
//! - nothing folded writes nothing.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{
    append_block, apply_page_md_with_sidecar, open_or_create_page, set_block_collapsed,
    BacklinkIndex, PageKind, SyncTransport,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::commands::block::unfold_ancestors;
use outl_tauri_shared::host::AppHost;
use parking_lot::Mutex;
use tempfile::TempDir;

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

fn fixture() -> (TempDir, TestHost, NodeId, NodeId, NodeId, NodeId, NodeId) {
    let tmp = TempDir::new().expect("tmp");
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).expect("workspace");
    let page = open_or_create_page(&mut ws, &hlc, "notes", "notes", PageKind::Page).unwrap();
    let other = open_or_create_page(&mut ws, &hlc, "other", "other", PageKind::Page).unwrap();
    let a = append_block(&mut ws, &hlc, Some(page), Some("a")).unwrap();
    let b = append_block(&mut ws, &hlc, Some(a), Some("b")).unwrap();
    let leaf = append_block(&mut ws, &hlc, Some(b), Some("leaf")).unwrap();
    let foreign = append_block(&mut ws, &hlc, Some(other), Some("foreign")).unwrap();
    // The view reads the outline from the page's `.md`, so project it.
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
    let host = TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root: tmp.path().to_path_buf(),
        registry: Arc::new(RuntimeRegistry::new()),
        backlinks: Arc::new(Mutex::new(None)),
    };
    (tmp, host, page, a, b, leaf, foreign)
}

fn fold(host: &TestHost, node: NodeId) {
    let mut guard = host.workspace.lock();
    let ws = guard.as_mut().unwrap();
    set_block_collapsed(ws, &host.hlc, node, true).unwrap();
}

fn log_len(host: &TestHost) -> usize {
    host.workspace.lock().as_ref().unwrap().log().len()
}

fn is_collapsed(host: &TestHost, node: NodeId) -> bool {
    host.workspace
        .lock()
        .as_ref()
        .unwrap()
        .tree()
        .is_collapsed(node)
}

#[test]
fn unfolds_the_folded_ancestors_and_returns_the_page_view() {
    let (_tmp, host, page, a, b, leaf, _) = fixture();
    fold(&host, a);
    fold(&host, b);
    let before = log_len(&host);

    let view = unfold_ancestors(&host, page.to_string(), leaf.to_string()).expect("unfolded");

    assert!(!is_collapsed(&host, a));
    assert!(!is_collapsed(&host, b));
    assert_eq!(log_len(&host), before + 2, "one op per folded ancestor");
    let top = &view.outline[0];
    assert!(!top.collapsed, "the reply carries the opened fold");
    assert!(!top.children[0].collapsed);
}

#[test]
fn a_block_of_another_page_is_refused_and_writes_nothing() {
    let (_tmp, host, page, a, _, _, foreign) = fixture();
    fold(&host, a);
    let before = log_len(&host);

    let err = unfold_ancestors(&host, page.to_string(), foreign.to_string())
        .expect_err("not on this page");

    assert!(err.contains("not in the tree"), "{err}");
    assert_eq!(log_len(&host), before);
    assert!(is_collapsed(&host, a));
}

#[test]
fn nothing_folded_writes_nothing() {
    let (_tmp, host, page, _, _, leaf, _) = fixture();
    let before = log_len(&host);

    unfold_ancestors(&host, page.to_string(), leaf.to_string()).expect("a no-op is fine");

    assert_eq!(log_len(&host), before);
}
