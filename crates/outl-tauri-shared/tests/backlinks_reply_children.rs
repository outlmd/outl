//! What `page_backlinks` ships for a citing block's children, and how
//! often it pays for them.
//!
//! The reply's direct hits carry the capped subtree; the namespace set
//! does not. Both halves are asserted on the real command, not on the
//! helper behind it, because the cost of this change is in *where* the
//! copy happens (after the locks drop) and a helper cannot say that.
//!
//! The *cheap once* half is a census in the style of
//! `backlink_commit_cost.rs`: a type cannot bound how many times a caller
//! asks, so the call sites are counted.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use outl_actions::{
    append_block, apply_page_md_with_sidecar, list_pages, open_or_create_page, BacklinkIndex,
    PageKind, SyncTransport, SUBTREE_MAX_DEPTH,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::commands::page::page_backlinks;
use outl_tauri_shared::host::AppHost;
use outl_tauri_shared::ProjectionWriter;
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
    fn projection_writer(&self) -> Option<&ProjectionWriter> {
        None
    }
}

/// `target` is cited by one block that has a chain of six descendants, and
/// (through its namespace) by another block that also has a child.
fn host() -> (TempDir, TestHost) {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path().to_path_buf();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).expect("in-memory workspace");

    let target = open_or_create_page(&mut ws, &hlc, "target", "target", PageKind::Page)
        .expect("the read page");
    let source =
        open_or_create_page(&mut ws, &hlc, "notes", "notes", PageKind::Page).expect("the source");

    let direct = append_block(&mut ws, &hlc, Some(source), Some("direct cites [[target]]"))
        .expect("the direct citing block");
    let mut parent = direct;
    for level in 1..=6 {
        parent = append_block(&mut ws, &hlc, Some(parent), Some(&format!("level {level}")))
            .expect("a descendant");
    }

    let via_namespace = append_block(
        &mut ws,
        &hlc,
        Some(source),
        Some("namespaced cites [[target/child]]"),
    )
    .expect("the namespace-only citing block");
    append_block(
        &mut ws,
        &hlc,
        Some(via_namespace),
        Some("kid of the namespaced hit"),
    )
    .expect("its child");

    apply_page_md_with_sidecar(&ws, &root, target).expect("target projected");
    apply_page_md_with_sidecar(&ws, &root, source).expect("source projected");
    let _ = list_pages(&ws);

    let host = TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root,
        registry: Arc::new(RuntimeRegistry::new()),
        backlinks: Arc::new(Mutex::new(None)),
    };
    (tmp, host)
}

fn depth(nodes: &[outl_actions::OutlineNode]) -> usize {
    nodes
        .iter()
        .map(|n| 1 + depth(&n.children))
        .max()
        .unwrap_or(0)
}

#[test]
fn a_direct_hit_ships_its_children_capped_and_a_namespace_hit_ships_none() {
    let (_tmp, host) = host();
    let reply = block_on(page_backlinks(&host, "target".into())).expect("the reply");

    assert_eq!(reply.backlinks.len(), 1, "one block names the page");
    let direct = &reply.backlinks[0];
    assert_eq!(direct.source_block.text, "direct cites [[target]]");
    assert_eq!(
        depth(&direct.source_block.children),
        SUBTREE_MAX_DEPTH,
        "six levels were written, four ride the wire"
    );

    assert!(
        !reply.namespace_backlinks.is_empty(),
        "the namespaced mention reaches the page through its namespace"
    );
    for hit in &reply.namespace_backlinks {
        assert!(
            hit.source_block.children.is_empty(),
            "a namespace row is capped, collapsed and text-only: no subtree on the wire"
        );
    }
}

#[test]
fn the_warm_index_serves_the_same_children_as_a_cold_one() {
    // The slot is empty on the first call (built from disk, stored) and
    // warm on the second (served from the cached index). Both ride the
    // same `look_up`, and must say the same thing.
    let (_tmp, host) = host();
    let cold = block_on(page_backlinks(&host, "target".into())).expect("cold");
    assert!(host.backlinks.lock().is_some(), "the first call cached it");
    let warm = block_on(page_backlinks(&host, "target".into())).expect("warm");

    let kids = |r: &outl_tauri_shared::state::BacklinksReply| {
        serde_json::to_value(&r.backlinks[0].source_block.children).unwrap()
    };
    assert_eq!(kids(&cold), kids(&warm));
}

// ---------------------------------------------------------------------
// Cheap once.
// ---------------------------------------------------------------------

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root is two levels above this crate")
}

/// Occurrences of `needle` in `src` outside `//` comments.
fn count(src: &str, needle: &str) -> usize {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .map(|l| l.matches(needle).count())
        .sum()
}

/// A subtree is copied once per direct hit, in `shape_reply`, and
/// fetched (as a refcount) once per direct hit, in `look_up`. Anything
/// else is a second ask: the `Backlink` clones inside `collect_by_keys`
/// run under the workspace lock, so a deep copy there is the #179 shape.
#[test]
fn a_subtree_is_copied_once_and_only_after_the_locks_drop() {
    let path = workspace_root().join("crates/outl-tauri-shared/src/commands/page_backlinks.rs");
    let src = std::fs::read_to_string(&path).expect("page_backlinks.rs");

    assert_eq!(
        count(&src, ".with_children("),
        1,
        "the deep copy into the reply is `shape_reply`'s and nobody else's"
    );
    assert_eq!(
        count(&src, ".subtree("),
        1,
        "the lookup fetches each hit's Arc once, in `look_up`"
    );
    assert_eq!(
        count(&src, "look_up("),
        1 /* the definition */ + 3, /* the three phase-3 call sites */
        "every phase-3 branch goes through the one lookup"
    );
}
