//! `paste_block_after`: where a view-mode paste lands, and what it
//! reports as having landed.
//!
//! A client captures `after_id` / `parent_id` before awaiting the OS
//! clipboard, so by the time the command runs either can name a block a
//! peer trashed or moved to another page. The command resolves the
//! anchor once, inside the commit and before any op: a live parent,
//! else a live `after`, else the page itself, else a refusal that writes
//! nothing.
//!
//! `new_id` is the last **root** pasted. Pasting a nested payload twice,
//! the second time after `new_id`, must put the second copy beside the
//! first rather than inside it — which is what anchoring on the last id
//! minted (a DFS tail) used to do.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{
    append_block, apply_page_md_with_sidecar, delete, move_under, open_or_create_page, PageKind,
    SyncTransport,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::commands::paste::paste_block_after;
use outl_tauri_shared::helpers::parse_node_id;
use outl_tauri_shared::host::AppHost;
use outl_tauri_shared::state::PasteReply;
use parking_lot::Mutex;
use tempfile::TempDir;

/// Minimal [`AppHost`]: no projection writer, so every reply is read
/// back off a synchronous `.md` render.
struct TestHost {
    workspace: Arc<Mutex<Option<Workspace>>>,
    hlc: HlcGenerator,
    root: PathBuf,
    registry: Arc<RuntimeRegistry>,
    _tmp: TempDir,
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
    fn new() -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let actor = ActorId::new();
        Self {
            workspace: Arc::new(Mutex::new(Some(
                Workspace::open_in_memory(actor).expect("workspace"),
            ))),
            hlc: HlcGenerator::new(actor),
            root: tmp.path().to_path_buf(),
            registry: Arc::new(RuntimeRegistry::new()),
            _tmp: tmp,
        }
    }

    fn mutate<T>(&self, f: impl FnOnce(&mut Workspace, &HlcGenerator) -> T) -> T {
        let mut guard = self.workspace.lock();
        f(guard.as_mut().expect("workspace is open"), &self.hlc)
    }

    /// A page, projected so the sidecar exists.
    fn page(&self, slug: &str) -> NodeId {
        let root = self.root.clone();
        self.mutate(|ws, hlc| {
            let page = open_or_create_page(ws, hlc, slug, slug, PageKind::Page).expect("page");
            apply_page_md_with_sidecar(ws, &root, page).expect("projection");
            page
        })
    }

    fn block(&self, parent: NodeId, text: &str) -> NodeId {
        self.mutate(|ws, hlc| append_block(ws, hlc, Some(parent), Some(text)).expect("block"))
    }

    fn text(&self, node: NodeId) -> Option<String> {
        self.mutate(|ws, _| ws.block_text(node))
    }

    fn parent(&self, node: NodeId) -> Option<NodeId> {
        self.mutate(|ws, _| ws.tree().parent(node))
    }

    fn log_len(&self) -> usize {
        self.mutate(|ws, _| ws.log().len())
    }

    fn paste(
        &self,
        page: NodeId,
        after: Option<NodeId>,
        parent: Option<NodeId>,
        text: &str,
    ) -> Result<PasteReply, String> {
        paste_block_after(
            self,
            page.to_string(),
            after.map(|id| id.to_string()),
            parent.map(|id| id.to_string()),
            text.to_string(),
        )
    }
}

fn top_level(reply: &PasteReply) -> Vec<&str> {
    reply.view.outline.iter().map(|n| n.text.as_str()).collect()
}

fn new_id(reply: &PasteReply) -> NodeId {
    let id = reply
        .new_id
        .as_deref()
        .expect("a paste that created blocks reports one");
    parse_node_id(id).expect("new_id is a NodeId")
}

const NESTED: &str = "- one\n- two\n  - two.a\n";

#[test]
fn new_id_is_the_last_root_so_a_second_paste_lands_beside_the_first() {
    let host = TestHost::new();
    let page = host.page("p");
    let first = host.block(page, "first");

    let reply = host.paste(page, Some(first), None, NESTED).expect("paste");
    let landed = new_id(&reply);
    assert_eq!(host.text(landed).as_deref(), Some("two"));

    let reply = host.paste(page, Some(landed), None, NESTED).expect("again");
    assert_eq!(top_level(&reply), ["first", "one", "two", "one", "two"]);
    assert_eq!(host.text(new_id(&reply)).as_deref(), Some("two"));
    assert_eq!(host.parent(new_id(&reply)), Some(page));
}

#[test]
fn an_empty_page_takes_the_paste_as_its_first_blocks() {
    let host = TestHost::new();
    let page = host.page("empty");

    let reply = host.paste(page, None, None, "- a\n- b").expect("paste");
    assert_eq!(top_level(&reply), ["a", "b"]);
    assert_eq!(host.text(new_id(&reply)).as_deref(), Some("b"));
}

/// Zoomed into a block, `p` with the zoom root selected (or nothing in
/// the zoom) pastes inside it: the client passes the zoom root as
/// `parent_id`, and it wins over `after_id`.
#[test]
fn parent_id_appends_inside_the_zoom_root() {
    let host = TestHost::new();
    let page = host.page("zoom");
    let zoom = host.block(page, "zoomed");
    host.block(zoom, "existing child");

    let reply = host
        .paste(page, Some(zoom), Some(zoom), "- pasted")
        .expect("paste");
    let landed = new_id(&reply);
    assert_eq!(host.parent(landed), Some(zoom));
    assert_eq!(top_level(&reply), ["zoomed"]);
    let children: Vec<&str> = reply.view.outline[0]
        .children
        .iter()
        .map(|n| n.text.as_str())
        .collect();
    assert_eq!(children, ["existing child", "pasted"]);
}

#[test]
fn a_trashed_anchor_falls_back_to_the_end_of_the_page() {
    let host = TestHost::new();
    let page = host.page("t");
    host.block(page, "kept");
    let gone = host.block(page, "gone");
    host.mutate(|ws, hlc| delete(ws, hlc, gone)).expect("trash");

    let reply = host
        .paste(page, Some(gone), Some(gone), "- pasted")
        .expect("paste");
    assert_eq!(top_level(&reply), ["kept", "pasted"]);
    assert_eq!(host.parent(new_id(&reply)), Some(page));
}

/// A peer moved the selected block to another page while the clipboard
/// read was in flight. It is live, so a liveness check alone would paste
/// next to it — on a page the user is not looking at, behind a commit
/// scoped to this one.
#[test]
fn an_anchor_moved_to_another_page_falls_back_to_this_page() {
    let host = TestHost::new();
    let page = host.page("here");
    let other = host.page("there");
    let moved = host.block(page, "moved");
    host.mutate(|ws, hlc| move_under(ws, hlc, moved, other))
        .expect("move");

    let reply = host
        .paste(page, Some(moved), None, "- pasted")
        .expect("paste");
    let landed = new_id(&reply);
    assert_eq!(host.parent(landed), Some(page));
    assert_eq!(top_level(&reply), ["pasted"]);
}

#[test]
fn a_trashed_page_refuses_and_writes_nothing() {
    let host = TestHost::new();
    let page = host.page("doomed");
    let block = host.block(page, "inside");
    host.mutate(|ws, hlc| delete(ws, hlc, page))
        .expect("trash the page");
    let before = host.log_len();

    let refused = host.paste(page, Some(block), None, "- pasted");
    assert!(refused.is_err(), "a paste under a trashed page must refuse");
    assert_eq!(host.log_len(), before, "a refused paste wrote an op");
}

#[test]
fn a_whitespace_only_paste_writes_no_op_and_reports_no_landing() {
    let host = TestHost::new();
    let page = host.page("ws");
    let block = host.block(page, "only");
    let before = host.log_len();

    let reply = host
        .paste(page, Some(block), None, "  \n\r\n\t")
        .expect("paste");
    assert_eq!(reply.new_id, None);
    assert_eq!(top_level(&reply), ["only"]);
    assert_eq!(host.log_len(), before);
}
