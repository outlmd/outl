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
use outl_tauri_shared::commands::page::{page_backlinks, SUBTREE_NODE_BUDGET};
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
    host_with(|_, _, _, _| {})
}

/// [`host`], plus whatever `extra` writes into the source page before it
/// is projected. Takes the workspace, the clock, the source page and the
/// target page's slug-independent handles it needs.
fn host_with(
    extra: impl FnOnce(&mut Workspace, &HlcGenerator, outl_core::id::NodeId, outl_core::id::NodeId),
) -> (TempDir, TestHost) {
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

    extra(&mut ws, &hlc, source, target);
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
// Bounded: `SUBTREE_NODE_BUDGET` nodes per reply, spent in display order.
// ---------------------------------------------------------------------

fn nodes(forest: &[outl_actions::OutlineNode]) -> usize {
    forest.iter().map(|n| 1 + nodes(&n.children)).sum()
}

/// `hits` extra citing blocks on the source page, each with `kids` flat
/// children.
fn cite_with_kids(
    hits: usize,
    kids: usize,
) -> impl FnOnce(&mut Workspace, &HlcGenerator, outl_core::id::NodeId, outl_core::id::NodeId) {
    move |ws, hlc, source, _target| {
        for h in 0..hits {
            let cite = append_block(ws, hlc, Some(source), Some(&format!("cite {h} [[target]]")))
                .expect("a citing block");
            for k in 0..kids {
                append_block(ws, hlc, Some(cite), Some(&format!("kid {h}.{k}"))).expect("a child");
            }
        }
    }
}

/// What the index holds for each hit, in the reply's order.
fn indexed_sizes(host: &TestHost, reply: &outl_tauri_shared::state::BacklinksReply) -> Vec<usize> {
    let guard = host.backlinks.lock();
    let idx = guard.as_ref().expect("the reply cached the index");
    reply
        .backlinks
        .iter()
        .map(|b| idx.subtree(&b.block_id).map_or(0, |s| nodes(&s)))
        .collect()
}

#[test]
fn under_the_budget_the_reply_is_exactly_the_uncapped_one() {
    // Equivalence: 3 hits x 20 children is far below the budget, so every
    // hit ships what the index holds, node for node.
    let (_tmp, host) = host_with(cite_with_kids(3, 20));
    let reply = block_on(page_backlinks(&host, "target".into())).expect("the reply");
    let sizes = indexed_sizes(&host, &reply);

    assert!(sizes.iter().sum::<usize>() < SUBTREE_NODE_BUDGET);
    for (hit, want) in reply.backlinks.iter().zip(&sizes) {
        assert_eq!(nodes(&hit.source_block.children), *want, "{}", hit.block_id);
    }
}

#[test]
fn over_the_budget_the_rows_shown_first_keep_their_children_and_the_rest_ship_shallow() {
    // Structural: 8 x 400 = 3,200 subtree nodes against a budget of
    // 2,000. Asserted on the nodes in the reply, not on its size or its
    // timing.
    let (_tmp, host) = host_with(cite_with_kids(8, 400));
    let reply = block_on(page_backlinks(&host, "target".into())).expect("the reply");
    let sizes = indexed_sizes(&host, &reply);
    assert!(
        sizes.iter().sum::<usize>() > SUBTREE_NODE_BUDGET,
        "the fixture has to overflow the budget or this proves nothing"
    );

    // The expected shape, worked out independently: the longest prefix of
    // hits whose subtrees sum to at most the budget.
    let mut left = SUBTREE_NODE_BUDGET;
    let mut open = true;
    let mut shallow_with_a_subtree = 0;
    for (hit, size) in reply.backlinks.iter().zip(&sizes) {
        let fits = open && *size <= left;
        if fits {
            left -= size;
        } else {
            open = false;
        }
        let shipped = nodes(&hit.source_block.children);
        assert_eq!(shipped, if fits { *size } else { 0 }, "{}", hit.block_id);
        if !fits && *size > 0 {
            shallow_with_a_subtree += 1;
        }
    }
    assert!(shallow_with_a_subtree > 0, "some hit was turned away");

    let total: usize = reply
        .backlinks
        .iter()
        .map(|b| nodes(&b.source_block.children))
        .sum();
    assert!(total <= SUBTREE_NODE_BUDGET, "{total} nodes shipped");
    // A hit past the budget is still a hit: the count the panel shows
    // does not move.
    assert_eq!(reply.backlinks.len(), sizes.len());
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
    // Production code only: the module's own unit tests build budgets too.
    let src = src.split("#[cfg(test)]").next().expect("a first segment");

    assert_eq!(
        count(src, ".with_children("),
        1,
        "the deep copy into the reply is `shape_reply`'s and nobody else's"
    );
    assert_eq!(
        count(src, ".subtree("),
        1,
        "the lookup fetches each hit's Arc once, in `look_up`"
    );
    assert_eq!(
        count(src, "SubtreeBudget::new("),
        1, /* the shape_reply spend, once per reply */
        "one budget per reply: a second would hand a hit its children twice over"
    );
    assert_eq!(
        count(src, "look_up("),
        1 /* the definition */ + 3, /* the three phase-3 call sites */
        "every phase-3 branch goes through the one lookup"
    );
}
