//! Running a code block refreshes the backlinks of the page it wrote to.
//!
//! `run_code_block` mutates the op log and projects the page inline, but
//! it does not go through the commit pipeline, so it never touched the
//! host's cached backlink index. The next `page_backlinks` then served
//! the cached rows: a Run pressed on a backlink row came back without the
//! `> **result:**` children it had just written. Already wrong for a Run
//! on the open page whose block cites something; the backlink row is
//! what made it visible.
//!
//! The rule is `BacklinkRefresh::PatchPage`'s: patch the page from its
//! `.md` when the run projected it, drop the index when it did not.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{
    append_block, apply_page_md_with_sidecar, open_or_create_page, set_property, BacklinkIndex,
    OutlineNode, PageKind, SyncTransport, TEMPLATE_KEY,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::commands::exec::run_code_block;
use outl_tauri_shared::commands::page::page_backlinks;
use outl_tauri_shared::host::AppHost;
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

const RESULT: &str = "result-from-template";

/// A callable template `calc` (an `echo` fence, so the result is its
/// body) and a `notes` page whose block calls it. The call fence is a
/// backlink of the template page.
fn fixture() -> (TempDir, TestHost, NodeId, NodeId) {
    let tmp = TempDir::new().expect("tmp");
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).expect("workspace");

    let tpl = open_or_create_page(&mut ws, &hlc, "template-calc", "calc", PageKind::Page).unwrap();
    set_property(
        &mut ws,
        &hlc,
        tpl,
        TEMPLATE_KEY,
        Some(PropValue::Text("calc".into())),
    )
    .unwrap();
    append_block(
        &mut ws,
        &hlc,
        Some(tpl),
        Some(&format!("```echo\n{RESULT}\n```")),
    )
    .unwrap();
    let notes = open_or_create_page(&mut ws, &hlc, "notes", "notes", PageKind::Page).unwrap();
    let call = append_block(&mut ws, &hlc, Some(notes), Some("```call:calc\na: 1\n```")).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), tpl).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), notes).unwrap();

    let host = TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root: tmp.path().to_path_buf(),
        registry: Arc::new(RuntimeRegistry::with_builtins()),
        backlinks: Arc::new(Mutex::new(None)),
    };
    (tmp, host, notes, call)
}

fn texts(nodes: &[OutlineNode], out: &mut Vec<String>) {
    for n in nodes {
        out.push(n.text.clone());
        texts(&n.children, out);
    }
}

fn citing_subtree(host: &TestHost) -> Vec<String> {
    let reply = block_on(page_backlinks(host, "template-calc".into())).expect("backlinks");
    let row = reply
        .backlinks
        .iter()
        .find(|b| b.block_text.contains("call:calc"))
        .expect("the call fence is a backlink of the template page");
    let mut out = Vec::new();
    texts(&row.source_block.children, &mut out);
    out
}

#[test]
fn a_run_shows_its_result_under_the_backlink_row_on_a_warm_index() {
    let (_tmp, host, notes, call) = fixture();
    assert!(
        citing_subtree(&host).is_empty(),
        "nothing has run yet, and this read warms the index"
    );
    assert!(host.backlinks.lock().is_some(), "the index is cached");

    let reply = block_on(run_code_block(&host, notes.to_string(), call.to_string()))
        .expect("the call runs");
    assert!(reply.error.is_none(), "runtime error: {:?}", reply.error);

    let after = citing_subtree(&host);
    assert!(
        after.iter().any(|t| t.contains(RESULT)),
        "the result children must reach the backlink row, got {after:?}"
    );
    assert!(
        host.backlinks.lock().is_some(),
        "a projected run patches the page; it does not throw the index away"
    );
}

/// The other half of the rule: a run whose projection is refused leaves
/// nothing on disk to patch from, so the index goes.
#[test]
fn a_refused_projection_drops_the_index() {
    let (tmp, host, notes, call) = fixture();
    let _warm = citing_subtree(&host);

    // Put the source page ahead of its log (root `CLAUDE.md` invariant
    // 8): a line no op produced, under a sidecar whose hash agrees.
    let md_path = tmp.path().join("pages").join("notes.md");
    let mut md = std::fs::read_to_string(&md_path).unwrap();
    md.push_str("- only ever on disk\n");
    std::fs::write(&md_path, &md).unwrap();
    let sidecar_path = outl_md::sidecar::sidecar_path_for(&md_path);
    let mut sc = outl_md::sidecar::read(&sidecar_path).unwrap();
    sc.last_synced_hash = outl_md::sidecar::file_hash(&md);
    outl_md::sidecar::write(&sidecar_path, &sc).unwrap();

    block_on(run_code_block(&host, notes.to_string(), call.to_string()))
        .expect_err("the projection of a page ahead of its log is refused");

    assert!(
        host.backlinks.lock().is_none(),
        "an index describing a page this run could not project is dropped"
    );
}
