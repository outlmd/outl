//! `edit_block` against a block that left the tree.
//!
//! [Issue #473](https://github.com/outlmd/outl/issues/473)'s second bullet:
//! "text typed into a block vanishes as soon as the block loses focus. No
//! error is shown." `edit_text` refuses a trashed or re-minted node with
//! `ActionError::NotInTree` (invariant 6 keeps the record, so `contains`
//! still says yes — `block/mod.rs`'s `ensure_live` is what catches it), and
//! this command used to turn that into `Ok(())` and drop the keystroke.
//!
//! Returning the error instead is worse rather than better, and the clients
//! are why: mobile pins a **Retry** to a failed commit, so a dead id becomes
//! a retry that can never succeed; the desktop skips `applyView` on a
//! rejection, so it never drops the stale `editingId` either. Both leave the
//! user stuck, and neither shows them their text.
//!
//! So **two** things happen, and both are load-bearing. The text is kept
//! where it still can be, at the end of the page, through
//! `create_after_or_append` — the function that already owns this tolerance
//! for `create_block`, where the content was never thrown away. And the
//! reply carries a `DroppedEdit`: keeping the text silently is a new
//! silence, not a fixed one, because the end of a journal day is below the
//! fold and "my text vanished from where I typed it" is the same
//! observation from the user's seat.
//!
//! The case with teeth is a trashed **page**, not a trashed block.
//! `append_block` does not check its parent and `create_with_explicit_id`
//! applies the `Op::Create` before `edit_text` refuses, so appending into a
//! trashed page used to land a permanent empty block in the op log, discard
//! the text anyway, **and** return the error this arm exists to avoid — one
//! junk op per keypress of a Retry that can never succeed. That is why
//! `a_trashed_page_keeps_the_log_clean` asserts on the log length and not
//! only on the reply.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{
    append_block, apply_page_md_with_sidecar, open_or_create_page, PageKind, SyncTransport,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::commands::block::edit_block;
use outl_tauri_shared::host::AppHost;
use parking_lot::Mutex;
use tempfile::TempDir;

/// Minimal [`AppHost`]: a workspace, an HLC and a storage root. No
/// projection writer, so the reply is read back off the `.md` — the
/// synchronous path.
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

/// A projected page holding one block, plus the id of a block that is **in
/// the trash** — the state a peer's delete leaves behind while the client
/// still holds the id it was editing.
fn host_with_a_trashed_block() -> (TempDir, TestHost, NodeId, NodeId) {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    let page = open_or_create_page(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("kept")).unwrap();
    let doomed = append_block(&mut ws, &hlc, Some(page), Some("the peer deleted this")).unwrap();
    outl_actions::delete(&mut ws, &hlc, doomed).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();

    let host = TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root: tmp.path().to_path_buf(),
        registry: Arc::new(RuntimeRegistry::new()),
    };
    (tmp, host, page, doomed)
}

/// The same page, but **the page itself** is in the trash — a peer deleted
/// it while an editor was open here, so the anchor went with it. The
/// reporter's setup, and the case `host_with_a_trashed_block` cannot reach.
fn host_with_a_trashed_page() -> (TempDir, TestHost, NodeId, NodeId) {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    let page = open_or_create_page(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    let block = append_block(&mut ws, &hlc, Some(page), Some("was being edited")).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
    outl_actions::delete_page(&mut ws, &hlc, "notes").unwrap();

    let host = TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root: tmp.path().to_path_buf(),
        registry: Arc::new(RuntimeRegistry::new()),
    };
    (tmp, host, page, block)
}

/// Every block's text in the page view, in outline order.
fn texts(view: &outl_tauri_shared::state::PageView) -> Vec<String> {
    view.outline.iter().map(|node| node.text.clone()).collect()
}

/// How many ops the workspace holds right now.
fn op_count(host: &TestHost) -> usize {
    host.workspace.lock().as_ref().unwrap().log().len()
}

#[test]
fn an_edit_to_a_block_that_left_the_tree_keeps_the_text() {
    let (_tmp, host, page, doomed) = host_with_a_trashed_block();

    let view = edit_block(
        &host,
        page.to_string(),
        doomed.to_string(),
        "what the user actually typed".to_string(),
    )
    .expect("a dead id must not fail the commit — mobile pins a Retry to that");

    assert!(
        texts(&view)
            .iter()
            .any(|text| text == "what the user actually typed"),
        "the keystroke has to survive somewhere on the page: {:?}",
        texts(&view)
    );
    assert!(
        texts(&view).iter().any(|text| text == "kept"),
        "nothing else may move: {:?}",
        texts(&view)
    );
}

/// Keeping the text is half the job. The end of the page is below the fold
/// on a journal day, so a silent re-target is the same "my text vanished"
/// from the user's seat — with a block they did not ask for and a retype
/// that becomes a duplicate.
#[test]
fn a_re_targeted_edit_says_so_and_names_the_block_it_landed_in() {
    let (_tmp, host, page, doomed) = host_with_a_trashed_block();

    let view = edit_block(
        &host,
        page.to_string(),
        doomed.to_string(),
        "what the user actually typed".to_string(),
    )
    .unwrap();

    let notice = view
        .dropped_edit
        .as_ref()
        .expect("a re-targeted edit must not be silent");
    assert_eq!(
        notice.block,
        doomed.to_string(),
        "the notice names the block the client is holding, so it can clear it"
    );
    let kept_as = notice
        .kept_as
        .as_ref()
        .expect("the text was kept, so the notice says where");
    assert!(
        texts(&view)
            .iter()
            .any(|text| text == "what the user actually typed"),
        "and that block is the one on the page"
    );
    assert!(!notice.message.trim().is_empty(), "the user gets words");
    assert_ne!(kept_as, &doomed.to_string());
}

/// The blocking case. Nothing can be written, so nothing is: no `Op::Create`
/// under a trashed parent, no error that pins a dead Retry, and a notice
/// that admits the text is gone rather than implying it was saved.
#[test]
fn a_trashed_page_keeps_the_log_clean() {
    let (_tmp, host, page, block) = host_with_a_trashed_page();
    let before = op_count(&host);

    let view = edit_block(
        &host,
        page.to_string(),
        block.to_string(),
        "typed into a page a peer deleted".to_string(),
    )
    .expect("the commit must not fail: mobile pins a Retry to a failure");

    assert_eq!(
        op_count(&host),
        before,
        "an empty `Op::Create` under a trashed page is permanent, invisible, \
         and arrives again on every retry"
    );
    let notice = view
        .dropped_edit
        .as_ref()
        .expect("the user has to be told the text could not be saved");
    assert_eq!(notice.block, block.to_string());
    assert!(
        notice.kept_as.is_none(),
        "nothing was kept, so the notice must not claim a block"
    );
    assert!(!notice.message.trim().is_empty());
}

#[test]
fn an_empty_edit_to_a_block_that_left_the_tree_adds_nothing() {
    let (_tmp, host, page, doomed) = host_with_a_trashed_block();
    let before = op_count(&host);

    let view = edit_block(&host, page.to_string(), doomed.to_string(), String::new())
        .expect("an empty edit on a dead id is a no-op, not a failure");

    assert_eq!(
        texts(&view),
        vec!["kept".to_string()],
        "an empty edit has nothing to preserve, and a blank block is the \
         third bullet of #473"
    );
    assert_eq!(op_count(&host), before, "and it costs no op");
    assert!(
        view.dropped_edit.is_none(),
        "nothing was dropped — there was nothing to drop"
    );
}

#[test]
fn an_edit_to_a_live_block_still_edits_that_block() {
    let (_tmp, host, page, _doomed) = host_with_a_trashed_block();
    let live = {
        let guard = host.workspace.lock();
        let ws = guard.as_ref().unwrap();
        outl_actions::children_of(ws, page)
            .into_iter()
            .map(|(id, _)| id)
            .next()
            .unwrap()
    };

    let view_live = edit_block(
        &host,
        page.to_string(),
        live.to_string(),
        "edited in place".to_string(),
    )
    .unwrap();

    assert_eq!(
        texts(&view_live),
        vec!["edited in place".to_string()],
        "the ordinary path must not grow a second block"
    );
    assert!(
        view_live.dropped_edit.is_none(),
        "an ordinary edit must not carry a notice — a banner on every \
         keystroke is how the user learns to ignore it"
    );
}
