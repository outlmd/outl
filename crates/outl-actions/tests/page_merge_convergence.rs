//! Does `merge_pages` converge, and does it lose nothing a peer did
//! concurrently?
//!
//! The merge is a handful of ordinary `Op::Move`s, `Op::SetProp`s and
//! `Op::Edit`s emitted by one replica while another is still working on
//! the page being folded away. Single-replica tests cannot see what that
//! costs: a peer's edit to a block that gets moved, and a block a peer
//! appends to a page that is already in the trash on the merging device.
//! `crates/outl-core/tests/` owns the CRDT's own convergence; this owns
//! one *composite action* built on it, the way `revive_convergence.rs`
//! does for the revive.

mod trash_restore;

use outl_actions::block::{append_block, edit_text};
use outl_actions::merge_pages;
use outl_actions::page::{find_by_slug, open_or_create, PageKind};
use outl_actions::tree::{children_of, is_trashed};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::LogOp;
use outl_core::workspace::Workspace;
use trash_restore::{assert_converges, ops, replay, replica};

struct Seed {
    prefix: Vec<LogOp>,
    a1: NodeId,
    b1: NodeId,
    alpha: NodeId,
    beta: NodeId,
}

/// `alpha` holds two blocks (the first with a child), `beta` holds one.
fn seed() -> Seed {
    let (mut ws, hlc) = replica();
    let alpha = open_or_create(&mut ws, &hlc, "alpha", "Alpha", PageKind::Page).expect("alpha");
    let beta = open_or_create(&mut ws, &hlc, "beta", "Beta", PageKind::Page).expect("beta");
    let b1 = append_block(&mut ws, &hlc, Some(beta), Some("b1")).expect("b1");
    let a1 = append_block(&mut ws, &hlc, Some(alpha), Some("a1")).expect("a1");
    append_block(&mut ws, &hlc, Some(a1), Some("a1.1")).expect("a1.1");
    append_block(&mut ws, &hlc, Some(alpha), Some("a2")).expect("a2");
    Seed {
        prefix: ops(&ws),
        a1,
        b1,
        alpha,
        beta,
    }
}

/// One device's independent work after the causal `prefix`.
///
/// Not `trash_restore::branch`: that pairs a replayed workspace with a
/// generator of a *different* actor, which a batched composite action
/// (every action here) rejects, since a batch only carries the local
/// actor's ops. Here the workspace and its clock share one actor, and the
/// clock has observed the prefix so the new ops sort after it.
fn branch(prefix: &[LogOp], work: impl FnOnce(&mut Workspace, &HlcGenerator)) -> Vec<LogOp> {
    let actor = ActorId::new();
    let mut ws = Workspace::open_in_memory(actor).expect("in-memory workspace");
    let hlc = HlcGenerator::new(actor);
    for op in prefix {
        ws.apply(op.clone()).expect("apply prefix");
        hlc.observe(op.ts);
    }
    work(&mut ws, &hlc);
    ops(&ws)[prefix.len()..].to_vec()
}

/// The union applied to a workspace of its own actor, with a clock past
/// every op it holds, so what it writes next sorts after what it has seen.
fn writer(union: &[LogOp]) -> (Workspace, HlcGenerator) {
    let actor = ActorId::new();
    let mut ws = Workspace::open_in_memory(actor).expect("in-memory workspace");
    let hlc = HlcGenerator::new(actor);
    for op in union {
        ws.apply(op.clone()).expect("apply union");
        hlc.observe(op.ts);
    }
    (ws, hlc)
}

fn texts(ws: &Workspace, parent: NodeId) -> Vec<String> {
    children_of(ws, parent)
        .into_iter()
        .map(|(id, _)| ws.block_text(id).unwrap_or_default())
        .collect()
}

#[test]
fn a_merge_racing_an_edit_and_a_late_append_converges_and_loses_neither() {
    let s = seed();

    let merged = branch(&s.prefix, |ws, hlc| {
        merge_pages(ws, hlc, "alpha", "beta").expect("merge");
    });
    let peer = branch(&s.prefix, |ws, hlc| {
        edit_text(ws, hlc, s.a1, "a1 edited").expect("edit a block that is being moved");
        // A block the merging device never saw.
        append_block(ws, hlc, Some(s.alpha), Some("late")).expect("late append");
    });

    let mut union = s.prefix.clone();
    union.extend(merged);
    union.extend(peer);
    assert_converges("merge vs edit and append", &union);

    let ws = replay(&union);
    assert!(is_trashed(&ws, s.alpha), "the source ended in the trash");
    assert_eq!(
        ws.block_text(s.a1).as_deref(),
        Some("a1 edited"),
        "the concurrent edit survived on the block that moved"
    );
    assert_eq!(
        texts(&ws, s.beta),
        vec!["b1", "a1 edited", "a2"],
        "beta holds its own block, then the moved ones"
    );
    assert_eq!(
        texts(&ws, s.alpha),
        vec!["late"],
        "the late block is stranded under the trashed source"
    );
}

#[test]
fn rerunning_the_merge_on_the_converged_state_rescues_the_late_block_once() {
    let s = seed();
    let merged = branch(&s.prefix, |ws, hlc| {
        merge_pages(ws, hlc, "alpha", "beta").expect("merge");
    });
    let peer = branch(&s.prefix, |ws, hlc| {
        edit_text(ws, hlc, s.a1, "a1 edited").expect("edit");
        append_block(ws, hlc, Some(s.alpha), Some("late")).expect("late append");
    });
    let mut union = s.prefix.clone();
    union.extend(merged);
    union.extend(peer);

    let (mut ws, hlc) = writer(&union);
    let nodes_before = ws.tree().iter_nodes().count();

    let rerun = merge_pages(&mut ws, &hlc, "alpha", "beta").expect("rerun");

    assert!(rerun.already_merged);
    assert!(!rerun.source_deleted);
    assert_eq!(rerun.moved_blocks, 1, "only the late block was left");
    assert_eq!(texts(&ws, s.beta), vec!["b1", "a1 edited", "a2", "late"]);
    assert!(children_of(&ws, s.alpha).is_empty());
    assert_eq!(
        ws.tree().iter_nodes().count(),
        nodes_before,
        "nothing was copied"
    );
    assert_eq!(find_by_slug(&ws, "alpha"), None);

    let log_len = ws.log().iter().count();
    let third = merge_pages(&mut ws, &hlc, "alpha", "beta").expect("third run");
    assert!(third.nothing_changed(), "the rescue is itself idempotent");
    assert_eq!(ws.log().iter().count(), log_len);
}

#[test]
fn two_replicas_merging_the_same_pages_concurrently_converge_on_one_placement() {
    let s = seed();
    let one = branch(&s.prefix, |ws, hlc| {
        merge_pages(ws, hlc, "alpha", "beta").expect("merge on device one");
    });
    let two = branch(&s.prefix, |ws, hlc| {
        merge_pages(ws, hlc, "alpha", "beta").expect("merge on device two");
    });
    let mut union = s.prefix.clone();
    union.extend(one);
    union.extend(two);

    assert_converges("two concurrent merges", &union);

    let ws = replay(&union);
    let beta_kids = texts(&ws, s.beta);
    assert_eq!(beta_kids.len(), 3, "each block once: {beta_kids:?}");
    assert!(beta_kids.contains(&"b1".to_string()));
    assert_eq!(ws.tree().parent(s.b1), Some(s.beta));
    assert!(is_trashed(&ws, s.alpha));
}
