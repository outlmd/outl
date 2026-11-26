//! A fold made from a backlink row: `BacklinkRowOp::Collapse` / `Expand`.
//!
//! Three things have to hold, and each one is a different way for the
//! chevron to look like it did nothing:
//!
//! - the op is `Op::SetCollapsed` on the **source** page, so the fold
//!   syncs and the page itself opens folded;
//! - it writes no `.md` and no sidecar, the outline's own
//!   `set_block_collapsed` shape (a fold is not part of the file);
//! - `page_backlinks` reads the flag **back**. The index is built from
//!   the `.md`, which never carries a fold, so every row came back
//!   expanded until the lookup overlaid the tree. Asserted on a *warm*
//!   index, since a fold drops nothing and a cold rebuild would hide a
//!   cache that stopped telling the truth.
//!
//! The refusal half (a dead row, a transient id, a row moved off its
//! page) is the same gate every row op takes, so it lives in
//! `backlink_row_ops.rs`, `backlink_row_ownership.rs` and
//! `backlink_gate_order.rs`, which list both variants.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{
    append_block, apply_page_md_with_sidecar, open_or_create_page, BacklinkIndex, PageKind,
    SyncTransport,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::Op;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::commands::backlink_edit::{backlink_row_op, BacklinkRowOp};
use outl_tauri_shared::commands::page::page_backlinks;
use outl_tauri_shared::host::AppHost;
use outl_tauri_shared::state::BacklinksReply;
use parking_lot::Mutex;
use tauri::async_runtime::block_on;
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

struct Fixture {
    _tmp: TempDir,
    host: TestHost,
    source: NodeId,
    cited: NodeId,
    kid: NodeId,
}

/// `notes` cites `target` from a block with a child that has a child of
/// its own, so a fold of the citing block and a fold of the child are
/// both visible in the reply.
fn fixture() -> Fixture {
    let tmp = TempDir::new().expect("tmp");
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).expect("workspace");
    let target = open_or_create_page(&mut ws, &hlc, "target", "target", PageKind::Page).unwrap();
    let source = open_or_create_page(&mut ws, &hlc, "notes", "notes", PageKind::Page).unwrap();
    let cited = append_block(&mut ws, &hlc, Some(source), Some("cites [[target]]")).unwrap();
    let kid = append_block(&mut ws, &hlc, Some(cited), Some("kid")).unwrap();
    append_block(&mut ws, &hlc, Some(kid), Some("grandkid")).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), target).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), source).unwrap();
    let host = TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root: tmp.path().to_path_buf(),
        registry: Arc::new(RuntimeRegistry::new()),
        backlinks: Arc::new(Mutex::new(None)),
    };
    Fixture {
        _tmp: tmp,
        host,
        source,
        cited,
        kid,
    }
}

fn backlinks(host: &TestHost) -> BacklinksReply {
    block_on(page_backlinks(host, "target".into())).expect("the target's backlinks")
}

/// The `.md` and the sidecar, as bytes.
fn projection(host: &TestHost) -> (Vec<u8>, Vec<u8>) {
    let md = host.root.join("pages").join("notes.md");
    let sidecar = outl_md::sidecar::sidecar_path_for(&md);
    (std::fs::read(&md).unwrap(), std::fs::read(sidecar).unwrap())
}

fn fold(f: &Fixture, node: NodeId, op: BacklinkRowOp) {
    backlink_row_op(&f.host, f.source.to_string(), node.to_string(), op)
        .unwrap_or_else(|e| panic!("{op:?} on a live row: {e}"));
}

#[test]
fn a_fold_from_a_row_lands_on_the_source_page_and_reads_back() {
    let f = fixture();
    let before = backlinks(&f.host);
    assert!(
        !before.backlinks[0].source_block.collapsed,
        "the fixture starts expanded"
    );
    assert!(
        f.host.backlinks.lock().is_some(),
        "the index is warm before the fold"
    );
    let bytes = projection(&f.host);

    fold(&f, f.cited, BacklinkRowOp::Collapse);

    {
        let guard = f.host.workspace.lock();
        let ws = guard.as_ref().unwrap();
        assert!(ws.tree().is_collapsed(f.cited), "folded in the op log");
        assert!(
            matches!(
                ws.log().last().map(|o| &o.op),
                Some(Op::SetCollapsed { node, value: true, .. }) if *node == f.cited
            ),
            "the fold is one `Op::SetCollapsed`, so it syncs"
        );
    }
    assert_eq!(
        projection(&f.host),
        bytes,
        "a fold is not part of the `.md` or the sidecar, so neither is rewritten"
    );
    let after = backlinks(&f.host);
    assert!(
        after.backlinks[0].source_block.collapsed,
        "the row has to read the fold back, or the chevron looks dead"
    );
    assert_eq!(
        after.backlinks[0].source_block.children.len(),
        1,
        "the children still ship: hiding them is the client's flatten, not the wire's"
    );

    fold(&f, f.cited, BacklinkRowOp::Expand);
    assert!(!backlinks(&f.host).backlinks[0].source_block.collapsed);
}

#[test]
fn a_fold_on_a_child_row_reads_back_inside_the_subtree() {
    let f = fixture();
    let _warm = backlinks(&f.host);

    fold(&f, f.kid, BacklinkRowOp::Collapse);

    let reply = backlinks(&f.host);
    let citing = &reply.backlinks[0].source_block;
    assert!(!citing.collapsed, "only the child was folded");
    assert!(
        citing.children[0].collapsed,
        "a child row's fold is read back too, so the flatten hides the grandkid"
    );
}
