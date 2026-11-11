//! Root `CLAUDE.md` invariant 4's third clause for `trash::restore`:
//! no node ends up outside `root()` / `TRASH_ROOT`.
//!
//! `landing_for`'s other guards ask whether the folded parent is inside
//! the subtree, trashed, or absent from the tree. None of those answers
//! "is it **reachable**": a `Create` this replica never received leaves
//! a parent that is present, untrashed, and on an ancestor chain that
//! dead-ends at a node which does not exist here.
//!
//! The sharp part is not that the state is odd, it is the direction of
//! the trade. Before the restore the block is a **top-level trash row**,
//! so the user has a handle on it. Landing it under an unreachable
//! parent takes that handle away — the block is then on no page, in no
//! trash listing, and reachable from no surface — while the listing had
//! promised `refusal: None`. A refusal keeps it listed and recoverable.
//!
//! Promoted from the CRDT review's probe with its assertion inverted;
//! that probe recorded the pre-guard behaviour deliberately, so pinning
//! it would have failed this fix.

use outl_actions::block::{append_block, delete as delete_block};
use outl_actions::refusal::Surface;
use outl_actions::trash;
use outl_actions::tree::is_trashed;
use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::workspace::Workspace;

/// A node whose ancestor chain dead-ends: its parent's `Create` never
/// arrived on this replica (a torn `ops-*.jsonl`, a partial sync, a
/// compaction that dropped a line).
fn workspace_with_a_detached_parent() -> (Workspace, HlcGenerator, NodeId) {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    let ghost = NodeId::new();
    let detached = NodeId::new();
    let ts = hlc.next();
    ws.apply(LogOp {
        ts,
        actor: ts.actor,
        op: Op::Create {
            node: detached,
            parent: ghost,
            position: Fractional::first(),
        },
    })
    .unwrap();
    assert!(ws.tree().contains(detached));
    assert!(!ws.tree().contains(ghost), "the parent never arrived");
    (ws, hlc, detached)
}

#[test]
fn a_restore_into_an_unreachable_parent_is_refused_not_hidden() {
    let (mut ws, hlc, detached) = workspace_with_a_detached_parent();
    let block = append_block(&mut ws, &hlc, Some(detached), Some("conteúdo")).unwrap();
    delete_block(&mut ws, &hlc, block).unwrap();

    let err = trash::restore(&mut ws, &hlc, block)
        .expect_err("the landing parent is reachable from nothing");
    assert!(
        err.to_string().contains("no longer reachable"),
        "the refusal has to say why, got: {err}"
    );

    // The handle the user had is still there: still trashed, still a row.
    assert!(is_trashed(&ws, block), "a refused restore moves nothing");
    assert!(
        trash::list(&ws, Surface::Cli)
            .iter()
            .any(|e| e.node == block),
        "and the block stays listed, which is the whole point of refusing"
    );
}

/// The listing and the mutation agree, which is what stops the panel
/// offering a restore that would hide the block.
#[test]
fn the_listing_shows_the_same_refusal_for_an_unreachable_parent() {
    let (mut ws, hlc, detached) = workspace_with_a_detached_parent();
    let block = append_block(&mut ws, &hlc, Some(detached), Some("conteúdo")).unwrap();
    delete_block(&mut ws, &hlc, block).unwrap();

    let entry = trash::list(&ws, Surface::Cli)
        .into_iter()
        .find(|e| e.node == block)
        .expect("listed");
    // Through the same wording function the row used. A bare
    // `to_string()` here would pin the row to the surface-blind
    // diagnosis and defeat the `Surface` the listing was asked for; the
    // property is that there is one verdict, not one sentence.
    assert_eq!(
        entry.refusal,
        trash::refusal_for(&ws, block)
            .map(|e| outl_actions::refusal::trash_refusal_text(Surface::Cli, &e)),
        "the row carries `refusal_for`'s own verdict, never a second one"
    );
    assert!(
        entry.refusal.is_some(),
        "and it refuses: the listing used to promise `None` here"
    );
}

/// The guard is about *reachability*, not about having a page — a block
/// deleted from directly under `root()` still restores.
///
/// Without this the new arm would read as "every restore needs a page",
/// which would re-break `tests/root_is_a_legal_parent.rs`'s case.
#[test]
fn a_block_deleted_from_under_root_still_restores() {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    let block = append_block(&mut ws, &hlc, None, Some("solto")).unwrap();
    assert_eq!(ws.tree().parent(block), Some(NodeId::root()));
    delete_block(&mut ws, &hlc, block).unwrap();

    trash::restore(&mut ws, &hlc, block).expect("root() is a legal landing parent");
    assert_eq!(ws.tree().parent(block), Some(NodeId::root()));
    assert!(!is_trashed(&ws, block));
}

/// The one rule `trash::landing_page` adds over the inline resolution it
/// replaced: it asks `page_meta`, so a **legacy page-less root child**
/// answers `None` instead of being handed to `commit_page`.
///
/// Without it `enclosing_page_id` returns the first node whose parent is
/// `root()` — which on a workspace `migrate_legacy_into_today` has not
/// swept is an ordinary block, not a page. `commit_page` then projects
/// `NotInTree`, and a client reports "this page has stopped syncing"
/// about a page that does not exist.
///
/// Unpinned until now: replacing the `page_meta` filter with
/// `Some(page)` left every trash suite green, because both call sites
/// pass the answer into `trash::restore`, which re-asks `landing_for`.
/// The looseness is safe and invisible, which is exactly why it needs
/// its own test rather than riding on another.
#[test]
fn landing_page_is_none_for_a_legacy_root_child_that_is_not_a_page() {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    // A pre-page-model block: directly under root, carrying no
    // `page-slug`, with a child of its own.
    let legacy = append_block(&mut ws, &hlc, None, Some("bloco antigo")).unwrap();
    let child = append_block(&mut ws, &hlc, Some(legacy), Some("filho")).unwrap();
    assert_eq!(ws.tree().parent(legacy), Some(NodeId::root()));
    assert!(
        outl_actions::page_meta(&ws, legacy).is_none(),
        "precondition: it is a block, not a page"
    );

    delete_block(&mut ws, &hlc, child).unwrap();

    assert_eq!(
        outl_actions::enclosing_page_id(&ws, legacy),
        Some(legacy),
        "the walk stops at the first root child, page or not — which is the trap"
    );
    assert_eq!(
        trash::landing_page(&ws, child),
        None,
        "so `landing_page` has to ask `page_meta`, or `commit_page` gets a non-page"
    );

    // And the restore itself still works: there is simply no page to
    // project, which is what `None` means here.
    trash::restore(&mut ws, &hlc, child).expect("a legacy parent is still a legal landing");
    assert!(!is_trashed(&ws, child));
}
