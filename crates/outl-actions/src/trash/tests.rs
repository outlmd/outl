//! What restoring from the trash has to get right.

use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;

use super::*;

/// A workspace with one page root under the tree root.
///
/// Deliberately the same shape as `timeline::tests::Fixture` — both
/// modules fold the same parent trail, so both have to be exercised
/// against ops a real caller would actually write.
struct Fixture {
    ws: Workspace,
    hlc: HlcGenerator,
    page: NodeId,
}

impl Fixture {
    fn new() -> Self {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).expect("in-memory workspace");
        let page = NodeId::new();
        apply(
            &mut ws,
            &hlc,
            Op::Create {
                node: page,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        );
        Self { ws, hlc, page }
    }

    fn block(&mut self, parent: NodeId, text: &str) -> NodeId {
        let node = NodeId::new();
        apply(
            &mut self.ws,
            &self.hlc,
            Op::Create {
                node,
                parent,
                position: Fractional::first(),
            },
        );
        let text_op = self.ws.build_text_replace_update(node, text);
        apply(&mut self.ws, &self.hlc, Op::Edit { node, text_op });
        node
    }

    /// A second page root carrying a `page-slug`, which is what makes a
    /// node a page rather than a block.
    fn page_with_slug(&mut self, slug: &str) -> NodeId {
        let node = NodeId::new();
        apply(
            &mut self.ws,
            &self.hlc,
            Op::Create {
                node,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        );
        apply(
            &mut self.ws,
            &self.hlc,
            Op::SetProp {
                node,
                key: crate::page::SLUG_KEY.to_string(),
                value: Some(PropValue::Text(slug.to_string())),
                old_value: None,
            },
        );
        node
    }

    /// `old_parent` comes off the tree, the way `block::moves::move_to`
    /// does it. A fixture that hardcodes it writes an op no real caller
    /// produces.
    fn move_to(&mut self, node: NodeId, new_parent: NodeId) {
        let old_parent = self
            .ws
            .tree()
            .parent(node)
            .expect("the fixture only moves nodes that are in the tree");
        apply(
            &mut self.ws,
            &self.hlc,
            Op::Move {
                node,
                new_parent,
                position: Fractional::first(),
                old_parent,
                old_position: Fractional::first(),
            },
        );
    }

    fn delete(&mut self, node: NodeId) {
        self.move_to(node, NodeId::trash());
    }
}

fn apply(ws: &mut Workspace, hlc: &HlcGenerator, op: Op) {
    let ts = hlc.next();
    ws.apply(LogOp {
        ts,
        actor: ts.actor,
        op,
    })
    .expect("apply");
}

#[test]
fn a_restored_block_lands_as_the_last_child() {
    // Not the position it held before: `Move.old_position` carries the
    // same "local derivation, undo-only" caveat as `old_parent`, so the
    // original slot is not recoverable from the log as data. Last child
    // is the honest answer, and it is the one the doc promises.
    let mut f = Fixture::new();
    let first = f.block(f.page, "primeiro");
    let victim = f.block(f.page, "segundo");
    f.move_to(victim, f.page); // give it a position after `first`
    let last = f.block(f.page, "terceiro");
    f.delete(victim);

    restore(&mut f.ws, &f.hlc, victim).expect("restore");

    let children: Vec<NodeId> = crate::tree::children_of(&f.ws, f.page)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        children.last().copied(),
        Some(victim),
        "a restored block comes back at the end, not in its old slot"
    );
    assert!(children.contains(&first) && children.contains(&last));
}

#[test]
fn parent_at_deletion_ignores_a_re_emitted_create() {
    // A reconcile re-emits `Op::Create` for a block that already exists
    // — five times over for some blocks on the reference workspace. The
    // tree discards the stale parent it names, so folding it here would
    // attribute the deletion to a parent the block never had.
    let mut f = Fixture::new();
    let real_parent = f.block(f.page, "seção");
    let block = f.block(real_parent, "item");
    // A second `Create` naming a different parent, as a reconcile emits.
    apply(
        &mut f.ws,
        &f.hlc,
        Op::Create {
            node: block,
            parent: f.page,
            position: Fractional::first(),
        },
    );
    f.delete(block);

    let parent = parent_at_deletion(&f.ws, block).expect("fold");

    assert_eq!(
        parent,
        Some(real_parent),
        "the first Create wins; a re-emitted one must not move the parent"
    );
}

#[test]
fn list_reports_each_top_level_deletion_with_its_subtree_size() {
    let mut f = Fixture::new();
    let section = f.block(f.page, "seção");
    f.block(section, "filho a");
    f.block(section, "filho b");
    let loose = f.block(f.page, "solto");
    f.delete(section);
    f.delete(loose);

    let entries = list(&f.ws);

    assert_eq!(entries.len(), 2, "two top-level deletions");
    let section_entry = entries
        .iter()
        .find(|e| e.node == section)
        .expect("the deleted section should be listed");
    assert_eq!(
        section_entry.subtree_len, 3,
        "the count is what the user would lose: the section plus both children"
    );
    assert_eq!(section_entry.parent_at_deletion, Some(f.page));
    assert_eq!(section_entry.preview, "seção");
    assert!(section_entry.page.is_none(), "a block is not a page");
}

#[test]
fn list_marks_a_deleted_page_with_its_slug() {
    let mut f = Fixture::new();
    let page = f.page_with_slug("archive");
    f.delete(page);

    let entries = list(&f.ws);

    let entry = entries.iter().find(|e| e.node == page).expect("listed");
    assert_eq!(
        entry.page.as_deref(),
        Some("archive"),
        "a deleted page has to be tellable from a deleted block — restore refuses one and not the other"
    );
}

#[test]
fn a_listing_carries_the_same_refusal_restore_would_return() {
    // A listing that decides "restorable" on its own is a second owner
    // of the rule, and it drifts towards offering an action that fails.
    // One verdict function answers both.
    let mut f = Fixture::new();
    let parent = f.block(f.page, "seção");
    let child = f.block(parent, "item");
    f.delete(child);
    f.delete(parent);
    let page = f.page_with_slug("archive");
    f.delete(page);

    let entries = list(&f.ws);
    assert_eq!(entries.len(), 3);

    // Every refusable entry first: a refused restore mutates nothing, so
    // the listing stays valid as the loop runs.
    for node in [child, page] {
        let entry = entries.iter().find(|e| e.node == node).expect("listed");
        let refused = restore(&mut f.ws, &f.hlc, node).expect_err("should refuse");
        assert_eq!(
            entry.refusal.as_deref(),
            Some(refused.to_string().as_str()),
            "listing and restore must agree about {node}"
        );
    }

    // And the one that can be restored says so by saying nothing.
    let entry = entries.iter().find(|e| e.node == parent).expect("listed");
    assert_eq!(entry.refusal, None);
    restore(&mut f.ws, &f.hlc, parent).expect("the listing promised this one would work");
}

#[test]
fn list_leaves_out_a_block_that_was_already_restored() {
    let mut f = Fixture::new();
    let block = f.block(f.page, "voltou");
    f.delete(block);
    restore(&mut f.ws, &f.hlc, block).expect("restore");

    assert!(
        list(&f.ws).is_empty(),
        "the trash listing reads the tree, not the history of deletions"
    );
}

#[test]
fn restore_refuses_a_page_root_and_names_the_slug() {
    // Restoring a page is a `Move` *plus* re-projecting the `.md`, and
    // 16 of the 18 deleted pages on the reference workspace have their
    // slug taken by a live page today. Refusing with the slug named is
    // honest; succeeding structurally and leaving no `.md` is not.
    let mut f = Fixture::new();
    let page = f.page_with_slug("archive");
    f.delete(page);

    let err = restore(&mut f.ws, &f.hlc, page).expect_err("should refuse a page root");

    assert!(
        matches!(&err, ActionError::TrashPageRestoreUnsupported { slug, .. } if slug == "archive"),
        "expected TrashPageRestoreUnsupported naming the slug, got {err:?}"
    );
    assert_eq!(
        f.ws.tree().parent(page),
        Some(NodeId::trash()),
        "a refused restore must not move anything"
    );
}

#[test]
fn restore_refuses_a_block_that_was_never_deleted() {
    let mut f = Fixture::new();
    let block = f.block(f.page, "vivo");

    let err = restore(&mut f.ws, &f.hlc, block).expect_err("should refuse a live block");

    assert!(
        matches!(&err, ActionError::NotTrashed(id) if *id == block.to_string()),
        "expected NotTrashed, got {err:?}"
    );
}

#[test]
fn restore_refuses_when_the_parent_is_itself_trashed_and_names_it() {
    // 89 of the 393 top-level deletions on the reference workspace are
    // this shape: a block deleted on its own, whose parent was then
    // deleted too. Restoring it alone would put it back under a parent
    // that is itself in the trash — the block would stay invisible and
    // the user would be told it worked.
    let mut f = Fixture::new();
    let parent = f.block(f.page, "seção");
    let child = f.block(parent, "item");
    f.delete(child);
    f.delete(parent);

    let err = restore(&mut f.ws, &f.hlc, child).expect_err("should refuse");

    assert!(
        matches!(&err, ActionError::TrashParentTrashed { parent: p, .. } if *p == parent.to_string()),
        "expected TrashParentTrashed naming the parent, got {err:?}"
    );
    assert!(
        err.to_string().contains(&parent.to_string()),
        "the refusal has to name the ancestor to restore first: {err}"
    );
    assert_eq!(
        f.ws.tree().parent(child),
        Some(NodeId::trash()),
        "a refused restore must not move anything"
    );
}

#[test]
fn restore_puts_the_block_back_under_the_parent_it_was_deleted_from() {
    let mut f = Fixture::new();
    let block = f.block(f.page, "não era pra ter deletado");
    f.delete(block);
    assert_eq!(f.ws.tree().parent(block), Some(NodeId::trash()));

    restore(&mut f.ws, &f.hlc, block).expect("restore");

    assert_eq!(
        f.ws.tree().parent(block),
        Some(f.page),
        "the block should be back on the page it was deleted from"
    );
}

#[test]
fn a_move_the_tree_refused_as_a_cycle_never_becomes_the_origin() {
    // Root `CLAUDE.md` invariant 4: a `Move` that would create a cycle
    // is a no-op on the materialized tree **and the op still goes into
    // the log**. A fold that replays `new_parent` unconditionally
    // therefore disagrees with the tree on exactly those ops.
    //
    // Reachable today: `outl-plugins`' host calls `block::move_under`
    // without the cycle pre-check `outl block move` has.
    let mut f = Fixture::new();
    let parent = f.block(f.page, "seção");
    let block = f.block(parent, "item");

    // The tree refuses this; the log keeps it.
    apply(
        &mut f.ws,
        &f.hlc,
        Op::Move {
            node: block,
            new_parent: block,
            position: Fractional::first(),
            old_parent: parent,
            old_position: Fractional::first(),
        },
    );
    assert_eq!(
        f.ws.tree().parent(block),
        Some(parent),
        "precondition: the tree must have refused the cycle"
    );

    f.delete(block);

    assert_eq!(
        parent_at_deletion(&f.ws, block).expect("fold"),
        Some(parent),
        "the fold has to agree with the tree about which moves happened"
    );
}

#[test]
fn a_move_refused_into_a_descendant_stays_refused_after_the_descendant_leaves() {
    // `Move(section, item)` is a cycle while `item` sits under `section`,
    // so the tree refuses it. Then `item` moves back to the page, and
    // today's tree no longer shows any cycle: a check against it would
    // take `item` as where `section` lived and restore it there.
    let mut f = Fixture::new();
    let section = f.block(f.page, "seção");
    let item = f.block(section, "item");

    f.move_to(section, item);
    assert_eq!(
        f.ws.tree().parent(section),
        Some(f.page),
        "precondition: the tree must have refused the cycle"
    );

    f.move_to(item, f.page);
    f.delete(section);

    assert_eq!(
        parent_at_deletion(&f.ws, section).expect("fold"),
        Some(f.page),
        "the refused move must not become the origin once its target moves away"
    );
    restore(&mut f.ws, &f.hlc, section).expect("restore");
    assert_eq!(f.ws.tree().parent(section), Some(f.page));
}

#[test]
fn a_block_the_log_cannot_place_is_not_reported_as_untrashed() {
    // `refusal_for` proves `is_trashed(node)` before it folds. If the
    // fold then comes back empty, answering `NotTrashed` contradicts
    // what was just proven — and the listing shows the block while the
    // line under it says the block is not in the trash.
    let mut f = Fixture::new();
    let node = NodeId::new();
    apply(
        &mut f.ws,
        &f.hlc,
        Op::Create {
            node,
            parent: NodeId::trash(),
            position: Fractional::first(),
        },
    );

    let refusal = refusal_for(&f.ws, node).expect("should refuse");

    assert!(
        !matches!(refusal, ActionError::NotTrashed(_)),
        "the block IS in the trash; saying otherwise contradicts the listing: {refusal}"
    );
    assert!(
        refusal.to_string().contains("where it was"),
        "the refusal should say the log cannot place it: {refusal}"
    );
}
