//! Does the #473 revive converge, and is it order-independent?
//!
//! Every scenario builds op sets on independent replicas, then replays the
//! union onto fresh replicas in four delivery orders and compares
//! materialized state.
//!
//! **Why this file exists at all.** The revive is three `Op::Move`s emitted
//! on a page-open path every client runs every morning, and the property it
//! risks — that the three converge against a concurrent delete, a peer's
//! edit, and a child it never enumerated — had no test anywhere in the
//! repo. Single-replica tests cannot see it: the revive is correct in any
//! one order and nothing in CI would have noticed a later change making it
//! order-dependent. `crates/outl-core/tests/` owns the CRDT's own
//! convergence; this owns one *composite action* built on it.
//!
//! Three of these started as throwaway probes that printed an outcome
//! instead of asserting one. The outcomes are settled now, so they assert:
//! a probe that only prints is a probe, and a probe does not fail.

use outl_actions::block::{append_block, delete as delete_block, edit_text};
use outl_actions::page::{delete as delete_page, find_by_slug, open_or_create, PageKind};
use outl_actions::refusal::Surface;
use outl_actions::tree::is_trashed;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::LogOp;
use outl_core::workspace::Workspace;

mod trash_restore;

use trash_restore::{ops, replay};

/// The delivery-order harness is `tests/trash_restore/mod.rs`.
///
/// This file carried its own copy, from before that module existed —
/// it was the only file that needed one. The copy's reason expired the
/// moment the shared module landed, and the shared one is strictly
/// stronger: it compares the **whole tree** rather than a universe of
/// ids this file derives, and it walks full permutations rather than
/// four hand-picked orders.
///
/// `replay(&union)` afterwards is how a test that wants to go on
/// asserting gets a reference replica — `assert_converges` answers the
/// convergence question and deliberately returns only the shape.
fn replica() -> (Workspace, HlcGenerator, ActorId) {
    let actor = ActorId::new();
    (
        Workspace::open_in_memory(actor).unwrap(),
        HlcGenerator::new(actor),
        actor,
    )
}

/// Assert every delivery order converges, then hand back a replica with
/// the union applied so the caller can keep asserting on it.
fn assert_all_orders_converge(label: &str, union: &[LogOp]) -> Workspace {
    trash_restore::assert_converges(label, union);
    trash_restore::replay(union)
}

fn deleted_page_history() -> Vec<LogOp> {
    let (mut ws, hlc, _) = replica();
    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    let a = append_block(&mut ws, &hlc, Some(page), Some("a")).unwrap();
    append_block(&mut ws, &hlc, Some(a), Some("a.1")).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("b")).unwrap();
    delete_page(&mut ws, &hlc, "notes").unwrap();
    ops(&ws)
}

fn seeded(history: &[LogOp]) -> (Workspace, HlcGenerator) {
    let actor = ActorId::new();
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let hlc = HlcGenerator::new(actor);
    for op in history {
        ws.apply(op.clone()).unwrap();
        hlc.observe(op.ts);
    }
    (ws, hlc)
}

/// Q1: two devices revive the same deleted journal day offline.
#[test]
fn q1_concurrent_revive_on_two_devices_converges_to_a_live_page() {
    let history = deleted_page_history();

    let (mut a, hlc_a) = seeded(&history);
    open_or_create(&mut a, &hlc_a, "notes", "Notes", PageKind::Page).unwrap();
    let a_new: Vec<LogOp> = ops(&a).into_iter().skip(history.len()).collect();

    let (mut b, hlc_b) = seeded(&history);
    open_or_create(&mut b, &hlc_b, "notes", "Notes", PageKind::Page).unwrap();
    let b_new: Vec<LogOp> = ops(&b).into_iter().skip(history.len()).collect();

    assert_eq!(a_new.len(), 3, "revive = trash a, trash b, move root");
    assert_eq!(b_new.len(), 3);

    let mut union = history.clone();
    union.extend(a_new);
    union.extend(b_new);

    let ws = assert_all_orders_converge("q1", &union);

    let root_id = find_by_slug(&ws, "notes").expect("page resolves after a double revive");
    assert_eq!(
        ws.tree().parent(root_id),
        Some(NodeId::root()),
        "q1: a double revive must land live under root, not half-trashed"
    );
    assert!(!is_trashed(&ws, root_id));
    let live_children: Vec<NodeId> = outl_actions::tree::children_of(&ws, root_id)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(
        live_children.is_empty(),
        "q1: two revives of one page must not resurrect its children — {live_children:?}"
    );
}

/// Q2: the revive races a delete of the same page root.
///
/// The revive only fires on a trashed root, so the concurrency is reached
/// via a *second* deletion route: device A deletes the root directly while
/// device B, which saw the root re-parented under a page A then deleted,
/// revives it.
#[test]
fn q2_a_revive_concurrent_with_a_delete_is_order_independent() {
    let history = deleted_page_history();

    // Device B revives.
    let (mut b, hlc_b) = seeded(&history);
    let page = open_or_create(&mut b, &hlc_b, "notes", "Notes", PageKind::Page).unwrap();
    let b_new: Vec<LogOp> = ops(&b).into_iter().skip(history.len()).collect();

    // Device A, having seen B's revive, deletes the page again.
    let (mut a, hlc_a) = seeded(&history);
    for op in &b_new {
        a.apply(op.clone()).unwrap();
        hlc_a.observe(op.ts);
    }
    delete_page(&mut a, &hlc_a, "notes").unwrap();
    let a_new: Vec<LogOp> = ops(&a)
        .into_iter()
        .skip(history.len() + b_new.len())
        .collect();

    let mut union = history.clone();
    union.extend(b_new);
    union.extend(a_new);

    let ws = assert_all_orders_converge("q2", &union);
    assert!(
        is_trashed(&ws, page),
        "q2: the later delete must win in every order"
    );
}

/// Q2b: delete and revive minted independently, neither device having seen
/// the other's op. Reachable because `is_trashed` is an ancestor walk: B's
/// root is trashed through its *parent page*, so B can revive without ever
/// seeing a delete aimed at the root itself.
#[test]
fn q2b_independent_delete_and_revive_of_one_root_converges() {
    // History: page "notes" nested under a container page, container deleted.
    let (mut ws, hlc, _) = replica();
    let container =
        open_or_create(&mut ws, &hlc, "container", "Container", PageKind::Page).unwrap();
    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("a")).unwrap();
    outl_actions::block::move_under(&mut ws, &hlc, page, container).unwrap();
    delete_page(&mut ws, &hlc, "container").unwrap();
    let history = ops(&ws);
    assert!(
        is_trashed(&ws, page),
        "setup: notes is trashed via container"
    );

    // Device B revives `notes` (it never sees a delete aimed at `page`).
    let (mut b, hlc_b) = seeded(&history);
    open_or_create(&mut b, &hlc_b, "notes", "Notes", PageKind::Page).unwrap();
    let b_new: Vec<LogOp> = ops(&b).into_iter().skip(history.len()).collect();
    assert!(!b_new.is_empty(), "setup: B must actually revive");

    // Device A deletes `notes` directly, concurrently (same history only).
    let (mut a, hlc_a) = seeded(&history);
    delete_block(&mut a, &hlc_a, page).unwrap();
    let a_new: Vec<LogOp> = ops(&a).into_iter().skip(history.len()).collect();

    let mut union = history.clone();
    union.extend(b_new);
    union.extend(a_new);
    assert_all_orders_converge("q2b", &union);

    // And with A's ops ahead of B's in the union, to force the other
    // HLC/insertion interleaving through `apply_op`'s reorder loop.
    let (mut a2, hlc_a2) = seeded(&history);
    delete_block(&mut a2, &hlc_a2, page).unwrap();
    let a2_new: Vec<LogOp> = ops(&a2).into_iter().skip(history.len()).collect();
    let mut union2 = history.clone();
    union2.extend(a2_new);
    union2.extend(ops(&b).into_iter().skip(history.len()));
    assert_all_orders_converge("q2b-swapped", &union2);
}

/// Q4: a peer edits a child block at the same moment this device trashes it
/// as part of a revive. The text must survive and converge.
#[test]
fn q4_a_peer_edit_racing_the_per_child_trash_keeps_the_text() {
    let (mut ws, hlc, _) = replica();
    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    let a = append_block(&mut ws, &hlc, Some(page), Some("a")).unwrap();
    let b = append_block(&mut ws, &hlc, Some(page), Some("b")).unwrap();
    let history_live = ops(&ws);

    // Peer P edits child `a` while the page is still live on its replica.
    let (mut p, hlc_p) = seeded(&history_live);
    edit_text(&mut p, &hlc_p, a, "a, edited by the peer").unwrap();
    let p_new: Vec<LogOp> = ops(&p).into_iter().skip(history_live.len()).collect();

    // Device D deletes the page, then revives it — never seeing P's edit.
    let (mut d, hlc_d) = seeded(&history_live);
    delete_page(&mut d, &hlc_d, "notes").unwrap();
    open_or_create(&mut d, &hlc_d, "notes", "Notes", PageKind::Page).unwrap();
    let d_new: Vec<LogOp> = ops(&d).into_iter().skip(history_live.len()).collect();

    let mut union = history_live.clone();
    union.extend(p_new);
    union.extend(d_new);

    let ws = assert_all_orders_converge("q4", &union);

    assert!(is_trashed(&ws, a), "q4: the revive's trashing must hold");
    assert!(is_trashed(&ws, b));
    assert_eq!(
        ws.block_text(a).as_deref(),
        Some("a, edited by the peer"),
        "q4: the peer's edit must survive being trashed"
    );
    let listed: Vec<String> = outl_actions::trash::list(&ws, Surface::Cli)
        .into_iter()
        .map(|e| e.preview)
        .collect();
    assert!(
        listed.iter().any(|p| p.contains("edited by the peer")),
        "q4: the edited text has to be recoverable, got {listed:?}"
    );
}

/// Q4b: a child the revive never saw, because the peer still had the page
/// live when it appended. The revive enumerates {a, b} only.
///
/// A peer appending under an already-trashed root is **not** the shape to
/// reach for, and the reason is worth stating precisely because getting it
/// wrong is what produced the junk-op bug this suite exists beside.
///
/// `append_block` has **no parent guard at all**. `create_with_explicit_id`
/// applies the `Op::Create` unconditionally and only then calls
/// `edit_text`, which is the half that asks `ensure_live` — so a non-empty
/// append under a trashed root fails *after* minting the block, and a
/// `text: None` append lands outright. `create_after_or_append` is where
/// the refusal lives now (`block::siblings::append_in_page`), not in
/// `append_block`.
///
/// So the honest race is this one: the peer appended while the page was
/// still live on its replica, and the revive enumerated {a, b} only.
#[test]
fn q4b_a_child_the_revive_never_saw_does_not_break_convergence() {
    let (mut ws, hlc, _) = replica();
    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    let a = append_block(&mut ws, &hlc, Some(page), Some("a")).unwrap();
    let b = append_block(&mut ws, &hlc, Some(page), Some("b")).unwrap();
    let history_live = ops(&ws);

    let (mut p, hlc_p) = seeded(&history_live);
    let late = append_block(&mut p, &hlc_p, Some(page), Some("arrived late")).unwrap();
    let p_new: Vec<LogOp> = ops(&p).into_iter().skip(history_live.len()).collect();

    let (mut d, hlc_d) = seeded(&history_live);
    delete_page(&mut d, &hlc_d, "notes").unwrap();
    open_or_create(&mut d, &hlc_d, "notes", "Notes", PageKind::Page).unwrap();
    let d_new: Vec<LogOp> = ops(&d).into_iter().skip(history_live.len()).collect();

    let mut union = history_live.clone();
    union.extend(p_new);
    union.extend(d_new);

    let ws = assert_all_orders_converge("q4b", &union);
    assert!(is_trashed(&ws, a));
    assert!(is_trashed(&ws, b));

    // The settled outcome, asserted rather than printed: the revive trashes
    // exactly the children it enumerated, so a block whose own `Op::Create`
    // the revive never saw stays where that op put it — live, under the page
    // that just came back.
    //
    // That is the honest semantics and worth pinning in both directions. The
    // revive is not "clear this page": it undoes nothing the user did after
    // the delete, and it hides nothing it was not looking at. A future change
    // that re-derived the child list at apply time, or trashed by subtree
    // instead of by enumeration, would swallow this peer's block — and the
    // peer has no way to know.
    assert!(
        !is_trashed(&ws, late),
        "a child the revive never enumerated must not be trashed by it"
    );
    assert_eq!(ws.tree().parent(late), Some(page));
    assert_eq!(ws.block_text(late).as_deref(), Some("arrived late"));
}

/// Q3: undo of the revive `Move` puts the root back in the trash, driven
/// through `apply_op`'s real reorder loop by a late-arriving op.
#[test]
fn q3_a_late_op_forcing_an_undo_of_the_revive_restores_the_trash() {
    let history = deleted_page_history();

    // Device L mints an op with a LOW hlc (it never saw the revive).
    let (mut l, hlc_l) = seeded(&history);
    let other = open_or_create(&mut l, &hlc_l, "other", "Other", PageKind::Page).unwrap();
    let late: Vec<LogOp> = ops(&l).into_iter().skip(history.len()).collect();

    // Device D revives, with HLCs above `late` (it observed the history
    // only, but its wall clock advances; force it by applying `late` later).
    let (mut d, hlc_d) = seeded(&history);
    for op in &late {
        hlc_d.observe(op.ts);
    }
    let page = open_or_create(&mut d, &hlc_d, "notes", "Notes", PageKind::Page).unwrap();
    let revive: Vec<LogOp> = ops(&d).into_iter().skip(history.len()).collect();
    assert!(
        revive.iter().all(|r| late.iter().all(|l| l.ts < r.ts)),
        "setup: the revive must sit above `late` in HLC order"
    );

    // Now deliver `late` AFTER the revive — forcing undo of all three
    // revive ops, then redo.
    let mut ws = replica().0;
    for op in history.iter().chain(revive.iter()) {
        ws.apply(op.clone()).unwrap();
    }
    assert_eq!(ws.tree().parent(page), Some(NodeId::root()));
    for op in &late {
        ws.apply(op.clone()).unwrap();
    }

    // After the reorder window the revive is re-applied, so the page is
    // live again and the undo left nothing behind.
    assert_eq!(
        ws.tree().parent(page),
        Some(NodeId::root()),
        "q3: the redo must restore the revive exactly"
    );
    assert!(!is_trashed(&ws, page));
    assert!(ws.tree().contains(other));

    // And the in-order replay agrees with the reordered one.
    let mut in_order = history.clone();
    in_order.extend(late.clone());
    in_order.extend(revive.clone());
    let straight = replay(&in_order);
    // `shape` compares the **whole tree**, where the copy this replaced
    // compared a universe of ids derived from the ops — so a node that
    // moved without being named by any op in the union was invisible to
    // it.
    assert_eq!(
        trash_restore::shape(&straight),
        trash_restore::shape(&ws),
        "q3: reordered delivery must equal in-order delivery"
    );
}

/// Invariant 3 against the revive's own ops: re-delivering every one of
/// them changes neither the materialized state nor the log length.
#[test]
fn q_idempotency_redelivering_the_revive_changes_nothing() {
    let history = deleted_page_history();
    let (mut d, hlc_d) = seeded(&history);
    let page = open_or_create(&mut d, &hlc_d, "notes", "Notes", PageKind::Page).unwrap();
    let all = ops(&d);

    let mut ws = replica().0;
    for op in &all {
        ws.apply(op.clone()).unwrap();
    }
    let before = trash_restore::shape(&ws);
    let log_before = ws.log().len();

    // Every op twice, then the whole set a third time in reverse.
    for op in all.iter().chain(all.iter().rev()) {
        ws.apply(op.clone()).unwrap();
    }

    assert_eq!(
        ws.log().len(),
        log_before,
        "a re-delivered revive op grew the log"
    );
    assert_eq!(
        trash_restore::shape(&ws),
        before,
        "a re-delivered revive op changed the tree"
    );
    assert_eq!(ws.tree().parent(page), Some(NodeId::root()));
}

/// Calling the revive path twice in a row emits nothing the second time.
#[test]
fn q_a_second_open_after_a_revive_writes_no_op() {
    let history = deleted_page_history();
    let (mut d, hlc_d) = seeded(&history);
    open_or_create(&mut d, &hlc_d, "notes", "Notes", PageKind::Page).unwrap();
    let after_revive = d.log().len();

    open_or_create(&mut d, &hlc_d, "notes", "Notes", PageKind::Page).unwrap();

    assert_eq!(
        d.log().len(),
        after_revive,
        "the revive is not idempotent across two opens"
    );
}

/// **There are two paths that revive a trashed page root, and they answer
/// differently about the deleted blocks. Both answers are correct.**
///
/// `outl_md::reconcile::page_root::ensure_page_root_in_tree` emits
/// `Op::Move(page, root())` for a page root whose parent is not `root()`,
/// and the trash is such a parent — so reconcile has revived one since long
/// before `page::revive` existed. It moves only the root, leaving the
/// children live. `page::revive` trashes each child first.
///
/// The asymmetry is not drift, because the inputs differ. Reconcile has a
/// file in hand with content in it, and that file is evidence of the user's
/// intent the op log does not hold, so it answers "make the tree match
/// this". `page::revive` has no such file — a GUI delete removed the `.md`
/// — so it answers "the user wants to write here today".
///
/// **Routing one through the other is the thing to refuse in review.**
/// Reconcile calling `revive_if_trashed` would trash blocks whose text is
/// in the `.md` it is reconciling against, and the parse immediately after
/// would recreate them with fresh ids: every `((blk-…))` handle pointing at
/// them breaks and the sidecar is rewritten. That is worse than two paths.
///
/// This test is the durable form of that paragraph. A doc sentence cannot
/// fail; this fails if either path starts answering the other's question.
#[test]
fn the_md_reconcile_revives_a_page_root_too_and_keeps_its_children() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (mut ws, hlc, _) = replica();

    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    let a = append_block(&mut ws, &hlc, Some(page), Some("yesterday a")).unwrap();
    let b = append_block(&mut ws, &hlc, Some(page), Some("yesterday b")).unwrap();
    outl_actions::journal::apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
    let md = outl_actions::journal::page_md_path(
        tmp.path(),
        &outl_actions::page::page_meta(&ws, page).unwrap(),
    );

    outl_actions::page::delete(&mut ws, &hlc, "notes").unwrap();
    assert!(is_trashed(&ws, page), "setup: the page is trashed");
    assert!(md.exists(), "setup: nothing removed the `.md`");

    // An external edit, so the hash short-circuit does not fire. The
    // realistic shape: a peer's delete removes no local projection, and any
    // editor then touches the file.
    let disk = std::fs::read_to_string(&md).unwrap();
    std::fs::write(&md, format!("{disk}- typed after the delete\n")).unwrap();

    outl_md::reconcile::reconcile_md(&mut ws, &hlc, &md, None).unwrap();

    assert_eq!(
        ws.tree().parent(page),
        Some(NodeId::root()),
        "reconcile revives the root as well — this is the TUI's accidental fix"
    );
    assert!(!is_trashed(&ws, a), "and it keeps the file's own content");
    assert!(!is_trashed(&ws, b));
    let kept: Vec<Option<String>> = outl_actions::tree::children_of(&ws, page)
        .into_iter()
        .map(|(id, _)| ws.block_text(id))
        .collect();
    assert!(
        kept.iter().any(|t| t.as_deref() == Some("yesterday a")),
        "the `.md` holds these lines, so the tree must too: {kept:?}"
    );

    // Side by side, so the two answers are visible together rather than one
    // of them being read as the rule.
    let (mut open_path, hlc2, _) = replica();
    let page2 = open_or_create(&mut open_path, &hlc2, "notes", "Notes", PageKind::Page).unwrap();
    append_block(&mut open_path, &hlc2, Some(page2), Some("yesterday a")).unwrap();
    outl_actions::page::delete(&mut open_path, &hlc2, "notes").unwrap();
    open_or_create(&mut open_path, &hlc2, "notes", "Notes", PageKind::Page).unwrap();
    assert_eq!(
        outl_actions::tree::children_of(&open_path, page2).len(),
        0,
        "the open path revives it empty, with no file to argue otherwise"
    );
}
