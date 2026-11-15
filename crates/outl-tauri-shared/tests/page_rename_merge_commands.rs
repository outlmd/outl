//! `rename_page` / `merge_pages` through the IPC layer (RFC 0557).
//!
//! The policy (what a collision means, which mentions are rewritten, how
//! properties merge) is tested in `outl-actions`. What only this layer can
//! get wrong is the tail around it: every touched page re-projected, not
//! just the one that moved, the stale `.md` + sidecar gone, a taken name
//! answered without a write, and every refusal reaching the client as a
//! string. Synthetic fixtures only.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{
    append_block, apply_page_md_with_sidecar_guarded, open_journal, open_or_create_page, PageKind,
    SyncTransport,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::commands::page_merge::{merge_pages, rename_page, RenamePageReply};
use outl_tauri_shared::host::AppHost;
use parking_lot::Mutex;
use tauri::async_runtime::block_on;
use tempfile::TempDir;

/// Minimal [`AppHost`]: no projection writer, no undo stacks, no transport.
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
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let host = TestHost {
            workspace: Arc::new(Mutex::new(Some(Workspace::open_in_memory(actor).unwrap()))),
            hlc: HlcGenerator::new(actor),
            root: tmp.path().to_path_buf(),
            registry: Arc::new(RuntimeRegistry::new()),
        };
        (tmp, host)
    }

    fn with_ws<T>(&self, f: impl FnOnce(&Workspace) -> T) -> T {
        f(self.workspace.lock().as_ref().unwrap())
    }

    fn mutate<T>(&self, f: impl FnOnce(&mut Workspace, &HlcGenerator) -> T) -> T {
        let hlc = self.hlc.clone();
        f(self.workspace.lock().as_mut().unwrap(), &hlc)
    }

    /// A page with `blocks` as top-level children, projected to disk the
    /// way an ordinary edit would have left it.
    fn page(&self, slug: &str, title: &str, blocks: &[&str]) -> NodeId {
        let id = self.mutate(|ws, hlc| open_or_create_page(ws, hlc, slug, title, PageKind::Page));
        let id = id.unwrap();
        for text in blocks {
            self.mutate(|ws, hlc| append_block(ws, hlc, Some(id), Some(text)).unwrap());
        }
        self.project(id);
        id
    }

    fn project(&self, id: NodeId) {
        self.with_ws(|ws| apply_page_md_with_sidecar_guarded(ws, &self.root, id).unwrap());
    }

    fn md(&self, slug: &str) -> PathBuf {
        self.root.join("pages").join(format!("{slug}.md"))
    }

    fn sidecar(&self, slug: &str) -> PathBuf {
        self.root.join("pages").join(format!("{slug}.outl"))
    }

    fn read_md(&self, slug: &str) -> String {
        std::fs::read_to_string(self.md(slug))
            .unwrap_or_else(|e| panic!("pages/{slug}.md should exist: {e}"))
    }

    fn ops(&self) -> usize {
        self.with_ws(|ws| ws.log().len())
    }
}

#[test]
fn a_rename_rewrites_links_in_other_pages_and_moves_the_file() {
    let (_tmp, host) = TestHost::new();
    let alpha = host.page("alpha", "Alpha", &["first"]);
    let other = host.page("other", "Other", &["see [[alpha]] today"]);
    assert!(host.md("alpha").exists() && host.sidecar("alpha").exists());

    let reply = block_on(rename_page(&host, "alpha".into(), "  Beta Project ".into()))
        .expect("an unused name renames");

    let (view, report) = match reply {
        RenamePageReply::Renamed { view, report } => (view, report),
        other => panic!("expected a rename, got {other:?}"),
    };
    assert_eq!(view.page.id, alpha.to_string(), "the page keeps its id");
    assert_eq!(view.page.slug, "beta-project");
    assert_eq!(view.page.title, "Beta Project");
    assert_eq!(report.old_slug, "alpha");
    assert_eq!(report.old_title, "Alpha");
    assert_eq!(report.rewritten_blocks, 1);

    // The page moved: new projection present, the old one gone with its sidecar.
    assert!(host.read_md("beta-project").contains("first"));
    assert!(host.sidecar("beta-project").exists());
    assert!(!host.md("alpha").exists(), "the stale .md is removed");
    assert!(!host.sidecar("alpha").exists(), "and so is its sidecar");

    // The page that only *mentioned* it was re-projected too, not just the
    // one that moved: a `.md` left behind the op log reads "link to nothing".
    let other_md = host.read_md("other");
    assert!(other_md.contains("[[Beta Project]]"), "got: {other_md}");
    assert!(!other_md.contains("[[alpha]]"), "got: {other_md}");
    host.with_ws(|ws| {
        assert_eq!(
            outl_actions::find_by_slug(ws, "other"),
            Some(other),
            "the mentioning page itself is untouched structurally"
        );
    });
}

#[test]
fn a_rename_onto_a_taken_name_answers_exists_and_writes_nothing() {
    let (_tmp, host) = TestHost::new();
    host.page("alpha", "Alpha", &["a"]);
    let beta = host.page("beta", "Beta", &["b"]);
    host.page("other", "Other", &["see [[alpha]]"]);
    let ops_before = host.ops();
    let md_before = host.read_md("alpha");

    let reply = block_on(rename_page(&host, "alpha".into(), "Beta".into()))
        .expect("a taken name is a typed reply, not an error");

    let existing = match reply {
        RenamePageReply::Exists { existing } => existing,
        other => panic!("expected Exists, got {other:?}"),
    };
    assert_eq!(existing.id, beta.to_string());
    assert_eq!(existing.slug, "beta");
    assert_eq!(host.ops(), ops_before, "no op was written");
    assert_eq!(host.read_md("alpha"), md_before, "the page was not touched");
    assert!(host.read_md("other").contains("[[alpha]]"));
}

#[test]
fn a_blank_name_and_a_journal_are_refused_with_a_message() {
    let (_tmp, host) = TestHost::new();
    host.page("alpha", "Alpha", &["a"]);
    let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 9).unwrap();
    host.mutate(|ws, hlc| open_journal(ws, hlc, date).unwrap());
    let ops_before = host.ops();

    let blank = block_on(rename_page(&host, "alpha".into(), "   ".into())).unwrap_err();
    assert!(!blank.is_empty());
    let journal = block_on(rename_page(&host, "2026-07-09".into(), "Diary".into())).unwrap_err();
    assert!(journal.contains("journal"), "got: {journal}");
    let missing = block_on(rename_page(&host, "ghost".into(), "Anything".into())).unwrap_err();
    assert!(missing.contains("ghost"), "got: {missing}");

    assert_eq!(host.ops(), ops_before, "refusals write nothing");
}

#[test]
fn a_merge_moves_the_blocks_deletes_the_source_and_reprojects_who_linked_it() {
    let (_tmp, host) = TestHost::new();
    host.page("draft", "Draft", &["one", "two"]);
    let main = host.page("main", "Main", &["kept"]);
    host.page("refs", "Refs", &["see [[draft]]"]);

    let reply = block_on(merge_pages(&host, "draft".into(), "main".into()))
        .expect("two ordinary pages merge");

    assert_eq!(reply.view.page.id, main.to_string(), "lands on the target");
    assert_eq!(reply.report.source_slug, "draft");
    assert_eq!(reply.report.target_slug, "main");
    assert_eq!(reply.report.moved_blocks, 2);
    assert_eq!(reply.report.rewritten_blocks, 1);
    assert!(reply.report.property_conflicts.is_empty());
    assert!(reply.report.aliases_added.iter().any(|a| a == "draft"));
    assert!(!reply.report.already_merged);

    let main_md = host.read_md("main");
    for text in ["kept", "one", "two"] {
        assert!(main_md.contains(text), "{text} missing from: {main_md}");
    }
    assert!(!host.md("draft").exists(), "the source projection is gone");
    assert!(!host.sidecar("draft").exists());
    host.with_ws(|ws| assert_eq!(outl_actions::find_by_slug(ws, "draft"), None));

    let refs_md = host.read_md("refs").to_lowercase();
    assert!(!refs_md.contains("draft"), "got: {refs_md}");
    assert!(refs_md.contains("main"), "got: {refs_md}");
}

#[test]
fn a_merge_into_itself_a_journal_source_or_a_stranger_is_an_error_string() {
    let (_tmp, host) = TestHost::new();
    host.page("alpha", "Alpha", &["a"]);
    host.page("beta", "Beta", &["b"]);
    let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 9).unwrap();
    host.mutate(|ws, hlc| open_journal(ws, hlc, date).unwrap());
    let ops_before = host.ops();

    let own = block_on(merge_pages(&host, "alpha".into(), "alpha".into())).unwrap_err();
    assert!(own.contains("itself"), "got: {own}");
    let journal = block_on(merge_pages(&host, "2026-07-09".into(), "beta".into())).unwrap_err();
    assert!(journal.contains("journal"), "got: {journal}");
    let ghost = block_on(merge_pages(&host, "ghost".into(), "beta".into())).unwrap_err();
    assert!(ghost.contains("ghost"), "got: {ghost}");

    assert_eq!(host.ops(), ops_before, "refusals write nothing");
    assert!(host.md("alpha").exists() && host.md("beta").exists());
}
