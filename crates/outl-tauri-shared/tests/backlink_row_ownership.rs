//! A backlink row's block must live **under the page the row names**.
//!
//! Split out of `backlink_row_ops.rs` (the ratchet), which owns the
//! staleness side of the same gate. Editing a citing block's children
//! made this reachable: a child moved to another page after the index was
//! built would otherwise commit on the wrong page (wrong undo snapshot,
//! wrong projection). `stale_row_block` asks `row_is_live_on`, so the
//! answer is the same stale wording as a deleted block, never a new one.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{
    append_block, apply_page_md_with_sidecar, open_or_create_page, BacklinkIndex, PageKind,
    SyncTransport,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::commands::backlink_edit::{
    backlink_row_op, create_backlink_sibling, BacklinkRowOp, BacklinkSide,
};
use outl_tauri_shared::host::AppHost;
use parking_lot::Mutex;
use tempfile::TempDir;

/// Every `BacklinkRowOp`, written out rather than iterated from the
/// type: a variant added without a row here is a gesture whose refusal
/// nothing checks, and the `match` in `backlink_row_op` is what forces
/// the author to this file.
const EVERY_OP: &[BacklinkRowOp] = &[
    BacklinkRowOp::ToggleTodo,
    BacklinkRowOp::MarkDone,
    BacklinkRowOp::Delete,
    BacklinkRowOp::Indent,
    BacklinkRowOp::Outdent,
    BacklinkRowOp::MoveUp,
    BacklinkRowOp::MoveDown,
    BacklinkRowOp::Collapse,
    BacklinkRowOp::Expand,
];

const EVERY_SIDE: &[BacklinkSide] = &[BacklinkSide::Below, BacklinkSide::Above];

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

fn op_count(state: &TestHost) -> usize {
    let workspace = state.workspace_arc();
    let ops = workspace.lock().as_ref().unwrap().log().len();
    ops
}

/// `notes` (the source page) holds a citing block with a child and a
/// grandchild chain three levels deep; `elsewhere` is a second page.
/// Returns `(tmp, host, notes, elsewhere, citing, child, deep)`.
fn host_with_children_on_two_pages() -> (TempDir, TestHost, NodeId, NodeId, NodeId, NodeId, NodeId)
{
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
    let deep = append_block(&mut ws, &hlc, Some(grand), Some("great-grandchild")).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), notes).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), elsewhere).unwrap();

    let host = TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root: tmp.path().to_path_buf(),
        registry: Arc::new(RuntimeRegistry::new()),
        backlinks: Arc::new(Mutex::new(None)),
    };
    (tmp, host, notes, elsewhere, citing, child, deep)
}

fn md_of(tmp: &TempDir, slug: &str) -> String {
    std::fs::read_to_string(tmp.path().join("pages").join(format!("{slug}.md")))
        .unwrap_or_else(|e| panic!("{slug}.md: {e}"))
}

/// A row built while `child` lived under `notes` keeps `(notes, child)`
/// after a peer (or a cross-page move) puts `child` on `elsewhere`.
/// Committing it would snapshot and project the **wrong page**, so every
/// gesture refuses in the stale wording, and nothing is written.
#[test]
fn a_block_moved_to_another_page_is_refused_by_every_row_gesture() {
    let (tmp, host, notes, elsewhere, _citing, child, _deep) = host_with_children_on_two_pages();
    {
        let workspace = host.workspace_arc();
        let mut guard = workspace.lock();
        let ws = guard.as_mut().unwrap();
        outl_actions::move_under(ws, &host.hlc, child, elsewhere).unwrap();
    }
    let (notes_md, elsewhere_md) = (md_of(&tmp, "notes"), md_of(&tmp, "elsewhere"));
    let ops = op_count(&host);

    for op in EVERY_OP {
        let err = backlink_row_op(&host, notes.to_string(), child.to_string(), *op)
            .expect_err("a block that left the page is not this row's block");
        assert_eq!(
            err, "That block could not be found, so nothing changed.",
            "{op:?}: the stale wording, no new sentence"
        );
    }
    for side in EVERY_SIDE {
        let err = create_backlink_sibling(&host, notes.to_string(), child.to_string(), *side)
            .expect_err("a sibling beside a block of another page is refused");
        assert_eq!(
            err, "That block could not be found, so nothing changed.",
            "{side:?}"
        );
    }
    assert_eq!(op_count(&host), ops, "nothing may reach the op log");
    assert_eq!(md_of(&tmp, "notes"), notes_md, "notes.md untouched");
    assert_eq!(
        md_of(&tmp, "elsewhere"),
        elsewhere_md,
        "elsewhere.md untouched"
    );
}

/// The other half, so the check cannot be satisfied by refusing every
/// child: a child of the right page commits, and so does one three
/// levels under the citing block.
#[test]
fn a_child_and_a_deep_child_of_the_right_page_still_commit() {
    let (_tmp, host, notes, _elsewhere, _citing, child, deep) = host_with_children_on_two_pages();

    for node in [child, deep] {
        let before = op_count(&host);
        backlink_row_op(
            &host,
            notes.to_string(),
            node.to_string(),
            BacklinkRowOp::ToggleTodo,
        )
        .expect("a live descendant of the page takes its chord");
        assert!(op_count(&host) > before, "the toggle must land an op");

        let before = op_count(&host);
        create_backlink_sibling(
            &host,
            notes.to_string(),
            node.to_string(),
            BacklinkSide::Below,
        )
        .expect("and a sibling");
        assert!(op_count(&host) > before, "the sibling must land an op");
    }
}
