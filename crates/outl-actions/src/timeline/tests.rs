//! What a page's history has to get right.
//!
//! The load-bearing ones are [`a_deleted_block_stays_in_the_page_history`]
//! and [`the_text_a_deletion_took_is_in_the_event`]: a timeline that drops
//! deletions answers "what changed" with everything except the change
//! people open a history to find.

use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;

use super::*;

/// A workspace with one page root under the tree root, plus a handle to
/// mint further ops.
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
        let op = Op::Create {
            node: page,
            parent: NodeId::root(),
            position: Fractional::first(),
        };
        apply(&mut ws, &hlc, op);
        Self { ws, hlc, page }
    }

    /// A second page root, for the tests about what belongs to which
    /// page.
    fn page(&mut self) -> NodeId {
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
        node
    }

    /// Create a block under `parent` and give it `text`.
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
        self.edit(node, text);
        node
    }

    fn edit(&mut self, node: NodeId, text: &str) {
        let text_op = self.ws.build_text_replace_update(node, text);
        apply(&mut self.ws, &self.hlc, Op::Edit { node, text_op });
    }

    /// `old_parent` is read off the tree, the way `block::moves::move_to`
    /// does it — a fixture that hardcodes it writes an op no real caller
    /// produces, and the trash attribution reads exactly that field.
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

    fn set_prop(&mut self, node: NodeId, key: &str, value: Option<&str>) {
        apply(
            &mut self.ws,
            &self.hlc,
            Op::SetProp {
                node,
                key: key.to_string(),
                value: value.map(|v| PropValue::Text(v.to_string())),
                old_value: None,
            },
        );
    }

    fn timeline(&self) -> PageTimeline {
        page_timeline(&self.ws, self.page, "notes", usize::MAX).expect("timeline")
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

fn changes(timeline: &PageTimeline) -> Vec<&Change> {
    timeline.events.iter().map(|e| &e.change).collect()
}

#[test]
fn a_fresh_page_reports_only_its_own_creation() {
    let fixture = Fixture::new();
    let timeline = fixture.timeline();
    assert_eq!(changes(&timeline), vec![&Change::Created]);
}

#[test]
fn every_edit_is_an_event_carrying_both_sides() {
    let mut fixture = Fixture::new();
    let block = fixture.block(fixture.page, "draft");
    fixture.edit(block, "final");
    let timeline = fixture.timeline();

    let edits: Vec<_> = timeline
        .events
        .iter()
        .filter_map(|e| match &e.change {
            Change::Edited { from, to } => Some((from.clone(), to.clone())),
            _ => None,
        })
        .collect();
    // Newest first.
    assert_eq!(
        edits,
        vec![
            (Some("draft".to_string()), "final".to_string()),
            (None, "draft".to_string()),
        ]
    );
}

/// A reconcile re-emitting the same text is an op, not a change. It
/// used to land at the top of a real page's history as `- x` / `+ x`.
#[test]
fn an_edit_that_changed_nothing_is_not_an_event() {
    let mut fixture = Fixture::new();
    let block = fixture.block(fixture.page, "same");
    let before = fixture.timeline().total;
    fixture.edit(block, "same");
    assert_eq!(fixture.timeline().total, before);
}

/// …but the next real change still reports the text it replaced, so
/// skipping the no-op must not lose the running "previous" value.
#[test]
fn a_change_after_a_no_op_edit_still_knows_what_it_replaced() {
    let mut fixture = Fixture::new();
    let block = fixture.block(fixture.page, "first");
    fixture.edit(block, "first");
    fixture.edit(block, "second");
    let events = block_timeline(&fixture.ws, block).expect("timeline");
    assert!(events.iter().any(|e| e.change
        == Change::Edited {
            from: Some("first".to_string()),
            to: "second".to_string(),
        }));
}

#[test]
fn events_come_back_newest_first() {
    let mut fixture = Fixture::new();
    let block = fixture.block(fixture.page, "one");
    fixture.edit(block, "two");
    let timeline = fixture.timeline();
    assert!(timeline.events.windows(2).all(|w| w[0].ts > w[1].ts));
}

/// The reason this module exists. A block the user deleted is exactly
/// what they open a history to look for, and it is no longer in the
/// page's subtree — so the naive "walk the page" scan misses it.
#[test]
fn a_deleted_block_stays_in_the_page_history() {
    let mut fixture = Fixture::new();
    let block = fixture.block(fixture.page, "the paragraph I lost");
    fixture.delete(block);
    let timeline = fixture.timeline();

    assert!(timeline
        .events
        .iter()
        .any(|e| e.node == block && matches!(e.change, Change::Deleted { .. })));
    assert!(timeline
        .events
        .iter()
        .filter(|e| e.node == block)
        .all(|e| e.node_deleted));
}

/// Naming the deletion is not enough — the text it took has to come back
/// with it, or the history says "something was here" and stops.
#[test]
fn the_text_a_deletion_took_is_in_the_event() {
    let mut fixture = Fixture::new();
    let block = fixture.block(fixture.page, "the paragraph I lost");
    fixture.delete(block);

    let timeline = fixture.timeline();
    let deleted = timeline
        .events
        .iter()
        .find_map(|e| match &e.change {
            Change::Deleted { text } => Some(text.clone()),
            _ => None,
        })
        .expect("a Deleted event");
    assert_eq!(deleted, "the paragraph I lost");
}

/// Deleting a parent trashes the subtree in one `Move`, so the children
/// are only reachable by walking down from the trashed root.
#[test]
fn deleting_a_parent_keeps_its_children_in_the_history() {
    let mut fixture = Fixture::new();
    let parent = fixture.block(fixture.page, "parent");
    let child = fixture.block(parent, "child");
    fixture.delete(parent);

    let timeline = fixture.timeline();
    assert!(
        timeline.events.iter().any(|e| e.node == child),
        "the child's history went to the trash with its parent"
    );
}

/// The trash scan has to reach a fixpoint. Delete a child and *then* its
/// parent and the child becomes its own direct child of the trash, whose
/// parent-at-deletion is a block no longer in the live subtree — one pass
/// drops it along with the text it took.
#[test]
fn a_child_deleted_before_its_parent_stays_in_the_history() {
    let mut fixture = Fixture::new();
    let parent = fixture.block(fixture.page, "parent");
    let child = fixture.block(parent, "the child I deleted first");
    fixture.delete(child);
    fixture.delete(parent);

    let timeline = fixture.timeline();
    assert!(
        timeline.events.iter().any(|e| e.node == child
            && matches!(&e.change, Change::Deleted { text } if text == "the child I deleted first")),
        "the child's deletion vanished because the parent left the page after it"
    );
}

/// Deleting a subtree is one `Move` on its root, so every block inside it
/// keeps pointing at that root. A direct-parent test answers `false` for
/// all of them and the client renders a live block that isn't there.
#[test]
fn every_block_inside_a_deleted_subtree_is_flagged_deleted() {
    let mut fixture = Fixture::new();
    let parent = fixture.block(fixture.page, "parent");
    let child = fixture.block(parent, "child");
    let grandchild = fixture.block(child, "grandchild");
    fixture.delete(parent);

    let timeline = fixture.timeline();
    for node in [parent, child, grandchild] {
        let rows: Vec<_> = timeline.events.iter().filter(|e| e.node == node).collect();
        assert!(!rows.is_empty(), "no events for {node}");
        assert!(
            rows.iter().all(|e| e.node_deleted),
            "{node} not flagged deleted"
        );
    }
}

/// `SetProp.old_value` is the `Move.old_parent` defect in another field:
/// `do_op` fills it on the copy that reaches the in-memory log, and 17 of
/// the 18 callers hardcode `None`, so every stored op carries a null. The
/// previous value has to come from the node's own op stream.
#[test]
fn changing_a_property_reports_the_value_it_replaced() {
    let mut fixture = Fixture::new();
    fixture.set_prop(fixture.page, "icon", Some("📓"));
    fixture.set_prop(fixture.page, "icon", Some("📕"));

    let timeline = fixture.timeline();
    assert!(
        timeline.events.iter().any(|e| e.change
            == Change::PropertySet {
                key: "icon".to_string(),
                from: Some("📓".to_string()),
                to: Some("📕".to_string()),
            }),
        "the change reported no previous value"
    );
}

/// …and clearing reports what was cleared, not a bare `None -> None`.
#[test]
fn clearing_a_property_reports_the_value_it_removed() {
    let mut fixture = Fixture::new();
    fixture.set_prop(fixture.page, "icon", Some("📓"));
    fixture.set_prop(fixture.page, "icon", None);

    let timeline = fixture.timeline();
    assert!(timeline.events.iter().any(|e| e.change
        == Change::PropertySet {
            key: "icon".to_string(),
            from: Some("📓".to_string()),
            to: None,
        }));
}

/// Re-writing a property's current value is an op and not a change, the
/// same rule the no-op `Op::Edit` gets.
#[test]
fn rewriting_a_property_with_the_same_value_is_not_an_event() {
    let mut fixture = Fixture::new();
    fixture.set_prop(fixture.page, "icon", Some("📓"));
    let before = fixture.timeline().total;
    fixture.set_prop(fixture.page, "icon", Some("📓"));
    assert_eq!(fixture.timeline().total, before);
}

/// `do_op`'s `Create` is idempotent and keeps the node's current parent,
/// so a stale reconcile re-emitting `Create{parent: old_page}` must not
/// move the accumulator — otherwise the old page claims a deletion that
/// happened on the new one.
#[test]
fn a_stale_create_does_not_let_the_old_page_claim_the_deletion() {
    let mut fixture = Fixture::new();
    let other_page = fixture.page();
    let block = fixture.block(fixture.page, "moved away");
    fixture.move_to(block, other_page);
    // The stale device's reconcile, naming the page the block left.
    let stale_parent = fixture.page;
    apply(
        &mut fixture.ws,
        &fixture.hlc,
        Op::Create {
            node: block,
            parent: stale_parent,
            position: Fractional::first(),
        },
    );
    fixture.delete(block);

    assert!(
        fixture.timeline().events.iter().all(|e| e.node != block),
        "this page claimed a deletion that happened on another page"
    );
}

/// Another page's deletions are not this page's history. Over-including
/// is worse than a gap: a gap is visibly a gap.
#[test]
fn a_block_deleted_from_another_page_is_not_in_this_ones_history() {
    let mut fixture = Fixture::new();
    let other_page = fixture.page();
    let theirs = fixture.block(other_page, "not mine");
    fixture.delete(theirs);

    let timeline = fixture.timeline();
    assert!(timeline.events.iter().all(|e| e.node != theirs));
}

#[test]
fn coming_back_out_of_the_trash_is_its_own_event() {
    let mut fixture = Fixture::new();
    let block = fixture.block(fixture.page, "oops");
    fixture.delete(block);
    apply(
        &mut fixture.ws,
        &fixture.hlc,
        Op::Move {
            node: block,
            new_parent: fixture.page,
            position: Fractional::first(),
            old_parent: NodeId::trash(),
            old_position: Fractional::first(),
        },
    );

    let timeline = fixture.timeline();
    assert!(timeline.events.iter().any(|e| e.change == Change::Restored));
    // Back in the page, so no longer flagged gone.
    assert!(timeline
        .events
        .iter()
        .filter(|e| e.node == block)
        .all(|e| !e.node_deleted));
}

#[test]
fn a_property_change_reports_the_key_and_both_values() {
    let mut fixture = Fixture::new();
    fixture.set_prop(fixture.page, "icon", Some("📓"));
    let timeline = fixture.timeline();
    assert!(timeline.events.iter().any(|e| e.change
        == Change::PropertySet {
            key: "icon".to_string(),
            from: None,
            to: Some("📓".to_string()),
        }));
}

#[test]
fn clearing_a_property_is_an_event_with_no_new_value() {
    let mut fixture = Fixture::new();
    fixture.set_prop(fixture.page, "icon", Some("📓"));
    fixture.set_prop(fixture.page, "icon", None);
    let timeline = fixture.timeline();
    assert!(timeline
        .events
        .iter()
        .any(|e| matches!(&e.change, Change::PropertySet { key, to: None, .. } if key == "icon")));
}

/// `page-slug` / `page-kind` are written by the page model, not by the
/// user. Showing them means every page's history opens with two events
/// nobody caused.
#[test]
fn page_model_bookkeeping_is_not_an_event() {
    let mut fixture = Fixture::new();
    fixture.set_prop(fixture.page, crate::page::SLUG_KEY, Some("notes"));
    fixture.set_prop(fixture.page, crate::page::KIND_KEY, Some("page"));
    let timeline = fixture.timeline();
    assert!(timeline
        .events
        .iter()
        .all(|e| !matches!(e.change, Change::PropertySet { .. })));
}

/// Folding a block does not change what the page says, and it happens
/// often enough to bury what does.
#[test]
fn folding_a_block_is_not_an_event() {
    let mut fixture = Fixture::new();
    let block = fixture.block(fixture.page, "text");
    apply(
        &mut fixture.ws,
        &fixture.hlc,
        Op::SetCollapsed {
            node: block,
            value: true,
            old_value: false,
        },
    );
    let before = fixture.timeline().total;
    apply(
        &mut fixture.ws,
        &fixture.hlc,
        Op::SetCollapsed {
            node: block,
            value: false,
            old_value: true,
        },
    );
    assert_eq!(fixture.timeline().total, before);
}

/// A truncated listing that reports its own length as the total reads as
/// complete, which is the one thing a history must never do.
#[test]
fn the_limit_caps_the_listing_and_never_the_count() {
    let mut fixture = Fixture::new();
    let block = fixture.block(fixture.page, "one");
    for text in ["two", "three", "four", "five"] {
        fixture.edit(block, text);
    }
    let full = fixture.timeline();
    assert!(full.total > 2);
    assert!(!full.truncated());

    let capped = page_timeline(&fixture.ws, fixture.page, "notes", 2).expect("timeline");
    assert_eq!(capped.events.len(), 2);
    assert_eq!(capped.total, full.total);
    assert!(capped.truncated());
    // And what survived the cap is the newest, not an arbitrary two.
    assert_eq!(capped.events, full.events[..2].to_vec());
}

/// A block's history follows the block. `page_timeline` scopes to a page
/// deliberately; this is the read that does not.
#[test]
fn a_block_timeline_follows_the_block_across_pages() {
    let mut fixture = Fixture::new();
    let other_page = fixture.page();
    let block = fixture.block(fixture.page, "written here");
    fixture.move_to(block, other_page);
    fixture.edit(block, "edited there");

    let events = block_timeline(&fixture.ws, block).expect("timeline");
    assert!(events
        .iter()
        .any(|e| matches!(&e.change, Change::Edited { to, .. } if to == "written here")));
    assert!(events
        .iter()
        .any(|e| matches!(&e.change, Change::Edited { to, .. } if to == "edited there")));

    // …and the page it left no longer claims it.
    assert!(fixture.timeline().events.iter().all(|e| e.node != block));
}

/// From the reference workspace: a reconcile re-emitted `Create` +
/// `Move` for an already-existing block five times over, and the block's
/// whole history read as "created / moved" repeated — with the one real
/// edit pushed off the end of the listing.
#[test]
fn a_reconcile_reemitting_create_and_move_adds_no_events() {
    let mut fixture = Fixture::new();
    let block = fixture.block(fixture.page, "kasjdkajs");
    let before = fixture.timeline().total;

    for _ in 0..5 {
        let parent = fixture.page;
        apply(
            &mut fixture.ws,
            &fixture.hlc,
            Op::Create {
                node: block,
                parent,
                position: Fractional::first(),
            },
        );
        fixture.move_to(block, parent);
    }

    assert_eq!(
        fixture.timeline().total,
        before,
        "re-creating and re-parenting a block where it already is changes nothing"
    );
}

/// `Move.old_parent` is filled by `do_op` on the copy that reaches the
/// in-memory log, while `Workspace::apply` persists the caller's
/// original — 99% of the Move ops in the reference workspace say `root`
/// no matter where the block was. Attribution must survive that, so the
/// parent trail is folded from `new_parent` instead.
#[test]
fn a_deletion_is_attributed_even_when_old_parent_is_wrong() {
    let mut fixture = Fixture::new();
    let block = fixture.block(fixture.page, "deleted with a lying op");
    apply(
        &mut fixture.ws,
        &fixture.hlc,
        Op::Move {
            node: block,
            new_parent: NodeId::trash(),
            position: Fractional::first(),
            // What the reconcile path actually writes.
            old_parent: NodeId::root(),
            old_position: Fractional::first(),
        },
    );

    let timeline = fixture.timeline();
    assert!(
        timeline.events.iter().any(|e| e.node == block
            && matches!(&e.change, Change::Deleted { text } if text == "deleted with a lying op")),
        "the deletion was dropped because the op misreported where the block came from"
    );
}

/// `block_timeline` returns everything and the caller cuts, so the
/// caller is the one place that can report an honest total. It used to
/// take a `limit`, which left `outl block history` reporting the *cut*
/// length as the total with `truncated: false` — the same lie
/// `the_limit_caps_the_listing_and_never_the_count` forbids for pages.
#[test]
fn a_block_timeline_is_never_cut_by_the_callee() {
    let mut fixture = Fixture::new();
    let block = fixture.block(fixture.page, "one");
    for text in ["two", "three", "four"] {
        fixture.edit(block, text);
    }
    let events = block_timeline(&fixture.ws, block).expect("timeline");
    assert!(
        events.len() >= 5,
        "create + four edits, got {}",
        events.len()
    );
}

#[test]
fn an_unknown_block_has_an_empty_timeline() {
    let fixture = Fixture::new();
    let events = block_timeline(&fixture.ws, NodeId::new()).expect("timeline");
    assert!(events.is_empty());
}

/// The timeline must not become a second opinion about what a block
/// said — `block_revisions` is the owner and this joins onto it.
#[test]
fn edit_events_agree_with_the_core_revision_list() {
    let mut fixture = Fixture::new();
    let block = fixture.block(fixture.page, "one");
    fixture.edit(block, "one two");
    fixture.edit(block, "one two three");

    let revisions = fixture.ws.block_revisions(block).expect("revisions");
    let mut from_timeline: Vec<String> = block_timeline(&fixture.ws, block)
        .expect("timeline")
        .into_iter()
        .filter_map(|e| match e.change {
            Change::Edited { to, .. } => Some(to),
            _ => None,
        })
        .collect();
    from_timeline.reverse();
    assert_eq!(
        from_timeline,
        revisions.into_iter().map(|r| r.text).collect::<Vec<_>>()
    );
}
