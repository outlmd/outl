//! RFC 0254 phase 4 — `cut_block` and `copy_block_ref`, the two block
//! commands neither GUI client had a backend for.
//!
//! `cut_block` pins the invariant that matters most here: the cut block
//! goes to the trash root (`Op::Move`, root `CLAUDE.md` invariant 6),
//! never a physical removal, and the returned markdown round-trips
//! through `paste_block_after` exactly like an external-clipboard paste
//! would.
//!
//! `copy_block_ref` pins that the handle it returns is the **same** one
//! `WorkspaceIndex` (and therefore `search_blocks`'s autocomplete, and
//! the TUI's `y r`) would resolve — reusing the index instead of
//! deriving the handle straight from the `NodeId` is the whole point,
//! since a collision-expanded handle would otherwise disagree.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{
    append_block, apply_page_md_with_sidecar, open_or_create_page, PageKind, SyncTransport,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::commands::block::{copy_block_ref, cut_block, paste_block_after};
use outl_tauri_shared::host::AppHost;
use parking_lot::Mutex;
use tempfile::TempDir;

/// Minimal [`AppHost`] — no projection writer, no backlink index slot,
/// so every mutation reads back off a synchronous `.md` render.
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

/// A page with one parent block (`text`) carrying one child
/// (`"child of {text}"`), already projected to disk so the sidecar
/// exists and `copy_block_ref` / `WorkspaceIndex::build` have
/// something to read.
fn host_with_block(text: &str) -> (TempDir, TestHost, NodeId, NodeId) {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let page = open_or_create_page(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    let block = append_block(&mut ws, &hlc, Some(page), Some(text)).unwrap();
    append_block(
        &mut ws,
        &hlc,
        Some(block),
        Some(&format!("child of {text}")),
    )
    .unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();

    let host = TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root: tmp.path().to_path_buf(),
        registry: Arc::new(RuntimeRegistry::new()),
    };
    (tmp, host, page, block)
}

#[test]
fn cut_block_moves_the_block_to_trash_rather_than_deleting_it() {
    let (_tmp, host, page, block) = host_with_block("move me");

    let reply = cut_block(&host, page.to_string(), block.to_string()).expect("cut_block");

    assert!(
        reply.markdown.contains("move me"),
        "clipboard markdown must carry the cut text, got {:?}",
        reply.markdown
    );
    assert!(
        reply.markdown.contains("child of move me"),
        "clipboard markdown must carry the subtree, got {:?}",
        reply.markdown
    );

    let guard = host.workspace.lock();
    let ws = guard.as_ref().unwrap();
    assert_eq!(
        ws.tree().parent(block),
        Some(NodeId::trash()),
        "cut must be a Move to the trash root, never a physical removal (invariant 6)"
    );
    // The block is still resolvable in the tree — trashed, not gone.
    assert!(
        ws.block_text(block).is_some(),
        "a trashed block must still carry its text; the op log, not deletion, is the source of truth"
    );
}

#[test]
fn cut_block_clipboard_round_trips_through_paste_block_after() {
    let (_tmp, host, page, block) = host_with_block("round trip me");
    let anchor = append_block(
        host.workspace.lock().as_mut().unwrap(),
        &host.hlc,
        Some(page),
        Some("anchor"),
    )
    .unwrap();
    apply_page_md_with_sidecar(host.workspace.lock().as_ref().unwrap(), &host.root, page).unwrap();

    let cut = cut_block(&host, page.to_string(), block.to_string()).expect("cut_block");

    let view = paste_block_after(&host, page.to_string(), anchor.to_string(), cut.markdown)
        .expect("paste_block_after");

    let texts: Vec<&str> = view.outline.iter().map(|n| n.text.as_str()).collect();
    assert!(
        texts.contains(&"round trip me"),
        "the pasted copy must carry the cut block's text back, got {texts:?}"
    );
    assert!(
        view.outline.iter().any(|n| n
            .children
            .iter()
            .any(|c| c.text == "child of round trip me")),
        "the pasted copy must carry the cut block's subtree, got {:#?}",
        view.outline
    );
}

#[test]
fn copy_block_ref_returns_the_same_handle_the_workspace_index_would_resolve() {
    let (_tmp, host, _page, block) = host_with_block("ref me");

    let token = copy_block_ref(&host, block.to_string()).expect("copy_block_ref");

    assert!(
        token.starts_with("((") && token.ends_with("))"),
        "must be the ((blk-XXXXXX)) wire form, got {token:?}"
    );
    let handle = &token[2..token.len() - 2];

    let index = outl_md::index::WorkspaceIndex::build(&host.root);
    let resolved = index
        .block_index()
        .resolve(handle)
        .expect("the handle copy_block_ref produced must resolve through the same index");
    assert_eq!(
        resolved.id, block,
        "the resolved handle must point back at the same block"
    );
}

#[test]
fn copy_block_ref_errors_on_a_block_with_no_sidecar_entry_yet() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    // Created but never projected to disk — no sidecar entry exists.
    let page = open_or_create_page(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    let block = append_block(&mut ws, &hlc, Some(page), Some("brand new")).unwrap();

    let host = TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root: tmp.path().to_path_buf(),
        registry: Arc::new(RuntimeRegistry::new()),
    };

    let err = copy_block_ref(&host, block.to_string()).unwrap_err();
    assert!(
        err.contains("save and retry"),
        "an unresolvable ref must fail loudly instead of returning a dead handle, got {err:?}"
    );
}
