//! The commit sequence, without Tauri.
//!
//! Every assertion here used to be untestable: the sequence lived in a
//! function generic over a trait that required Tauri's managed state, so
//! the only way to observe the ordering was to run a GUI client.

use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;

use super::{commit_page, CommitHooks};
use crate::block::{append_block, edit_text};
use crate::error::ActionError;
use crate::page::{open_or_create, PageKind};

/// Records the sequence as it happens, so the *order* is what the test
/// asserts, not just that each step ran.
#[derive(Default)]
struct Spy {
    undo: bool,
    log: Vec<String>,
    snapshots: Vec<(NodeId, String)>,
}

impl Spy {
    fn with_undo() -> Self {
        Spy {
            undo: true,
            ..Spy::default()
        }
    }

    fn steps(&self) -> &[String] {
        &self.log
    }
}

impl CommitHooks for Spy {
    fn project(&mut self, _workspace: &Workspace, _page: NodeId) {
        self.log.push("project".into());
    }

    fn records_undo(&self) -> bool {
        self.undo
    }

    fn record_undo(&mut self, page: NodeId, before: String) {
        self.log.push("record_undo".into());
        self.snapshots.push((page, before));
    }

    fn invalidate_backlinks(&mut self) {
        self.log.push("invalidate_backlinks".into());
    }

    fn announce(&mut self, workspace: &Workspace, page: NodeId) {
        // Resolving the slug here, not in `commit_page`, is the contract
        // — a host with no transport never pays for it.
        let slug = crate::page::page_meta(workspace, page)
            .map(|m| m.slug)
            .unwrap_or_default();
        self.log.push(format!("announce:{slug}"));
    }
}

fn workspace_with_page() -> (Workspace, HlcGenerator, NodeId, NodeId) {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).expect("in-memory workspace");
    let page = open_or_create(&mut ws, &hlc, "infra", "Infra", PageKind::Page).expect("page");
    let block = append_block(&mut ws, &hlc, Some(page), Some("first")).expect("block");
    (ws, hlc, page, block)
}

#[test]
fn the_five_steps_run_in_the_order_the_pipeline_promises() {
    let (mut ws, hlc, page, block) = workspace_with_page();
    let mut spy = Spy::with_undo();

    commit_page(&mut ws, &mut spy, page, |ws| {
        edit_text(ws, &hlc, block, "changed")
    })
    .expect("commit");

    assert_eq!(
        spy.steps(),
        vec![
            "record_undo",
            "invalidate_backlinks",
            "announce:infra",
            "project",
        ],
        "the order is the contract: an announce before the projection is \
         what lets a peer pull while this device is still writing, and \
         invalidating the index after the mutation is what stops a stale \
         one being rebuilt from pre-mutation state"
    );
}

#[test]
fn the_snapshot_is_the_render_from_before_the_mutation() {
    let (mut ws, hlc, page, block) = workspace_with_page();
    let mut spy = Spy::with_undo();

    commit_page(&mut ws, &mut spy, page, |ws| {
        edit_text(ws, &hlc, block, "changed")
    })
    .expect("commit");

    let snapshots = spy.snapshots;
    let (snapshot_page, before) = snapshots.first().expect("one snapshot");
    assert_eq!(*snapshot_page, page);
    assert!(
        before.contains("first") && !before.contains("changed"),
        "undo restores the page as it was, so the snapshot must predate \
         the mutation: {before:?}"
    );
}

/// A command that changes nothing must not push an undo step, or the
/// user's next `Cmd+Z` appears to do nothing at all.
#[test]
fn a_mutation_that_changes_no_render_records_no_snapshot() {
    let (mut ws, _hlc, page, _block) = workspace_with_page();
    let mut spy = Spy::with_undo();

    commit_page(&mut ws, &mut spy, page, |_ws| Ok::<(), ActionError>(())).expect("commit");

    assert!(
        spy.snapshots.is_empty(),
        "nothing changed — there is nothing to undo"
    );
    assert_eq!(
        spy.steps(),
        vec!["invalidate_backlinks", "announce:infra", "project"],
        "the rest of the sequence still runs: a no-op mutation can still \
         follow a peer's ops into the log"
    );
}

/// A host without undo pays no render at all, which is why
/// `records_undo` exists as its own question.
#[test]
fn a_host_without_undo_never_records_one() {
    let (mut ws, hlc, page, block) = workspace_with_page();
    let mut spy = Spy::default();

    commit_page(&mut ws, &mut spy, page, |ws| {
        edit_text(ws, &hlc, block, "changed")
    })
    .expect("commit");

    assert!(spy.snapshots.is_empty());
    assert_eq!(
        spy.steps(),
        vec!["invalidate_backlinks", "announce:infra", "project"]
    );
}

/// The mutation is the only step allowed to fail the commit — and when
/// it does, nothing downstream may run. Announcing ops that were never
/// written makes a peer pull an empty delta; projecting would rewrite
/// the `.md` for a change that did not happen.
#[test]
fn a_failed_mutation_runs_no_later_step() {
    let (mut ws, _hlc, page, _block) = workspace_with_page();
    let mut spy = Spy::with_undo();

    let result: Result<(), ActionError> = commit_page(&mut ws, &mut spy, page, |_ws| {
        Err(ActionError::NotInTree("01ABC".into()))
    });

    assert!(result.is_err());
    assert!(
        spy.steps().is_empty(),
        "a failed mutation must leave the pipeline untouched, got {:?}",
        spy.steps()
    );
}

/// The mutation's return value comes back untouched — this is what lets
/// `create_block` hand the frontend the new id instead of making it
/// re-discover the node by diffing the outline.
#[test]
fn the_mutation_value_is_returned() {
    let (mut ws, hlc, page, _block) = workspace_with_page();
    let mut spy = Spy::default();

    let new_id = commit_page(&mut ws, &mut spy, page, |ws| {
        append_block(ws, &hlc, Some(page), Some("second"))
    })
    .expect("commit");

    assert!(
        ws.tree().parent(new_id).is_some(),
        "the returned id must name a node that is actually in the tree"
    );
}

/// A page whose root left the tree has no slug. The hook still runs —
/// resolving is its job — and must not be able to take the commit down
/// with it.
#[test]
fn a_page_with_no_meta_still_completes_the_commit() {
    let (mut ws, _hlc, page, _block) = workspace_with_page();
    let mut spy = Spy::default();

    let missing = NodeId::new();
    commit_page(&mut ws, &mut spy, missing, |_ws| Ok::<(), ActionError>(())).expect("commit");

    assert_eq!(
        spy.steps(),
        vec!["invalidate_backlinks", "announce:", "project"],
        "the announce hook is still called; resolving to nothing is its \
         business, not the pipeline's"
    );
    let _ = page;
}
