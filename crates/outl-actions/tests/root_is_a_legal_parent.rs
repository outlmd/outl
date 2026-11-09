//! `NodeId::root()` is a legal parent, and three functions were asking
//! `contains` about it.
//!
//! `ROOT` and `TRASH_ROOT` are implicit: `Tree::new`'s doc says they "don't
//! appear in `nodes` but are valid parents for any other node". So
//! `Tree::contains(NodeId::root())` is **false**, and every guard written as
//! `contains(parent)` answers no for the one parent that always exists.
//!
//! Three places asked it, and none had a test:
//!
//! - `block::move_under`'s destination check, which is how this was found —
//!   `page::revive` is the first caller to move a node back under `root()`
//!   and got `NotInTree("000…000")`, naming the root itself.
//! - `trash::landing_for`'s `contains(parent)`, which made a block deleted
//!   from directly under root unrestorable. Every pre-page-model block lives
//!   there, and so does anything `migrate_legacy_into_today` has not swept.
//! - `block::siblings::append_in_page`'s `ensure_live(page)`, **added by the
//!   change that fixed the first two** and caught in review. That is the
//!   lesson worth keeping: this question is easy to reintroduce precisely
//!   because every caller today passes a real page root, so a third instance
//!   costs nothing until something reaches it.
//!
//! `NodeId::trash()` stays refused as a `move_under` destination, asymmetry
//! and all: landing there is `delete`, at `Fractional::first()`, and
//! accepting it here would be a second spelling of one op with a different
//! position. That half is pinned beside the function, in
//! `block/moves.rs`'s own tests.

use outl_actions::block::{append_block, create_after_or_append, delete as delete_block};
use outl_actions::page::{open_or_create, PageKind};
use outl_actions::tree::is_trashed;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;

fn replica() -> (Workspace, HlcGenerator) {
    let actor = ActorId::new();
    (
        Workspace::open_in_memory(actor).unwrap(),
        HlcGenerator::new(actor),
    )
}

/// What `move_under` accepting `NodeId::root()` made newly reachable: a
/// block parked as a page-less direct child of root, the pre-page-model
/// shape. Nothing lists it, and the startup sweep is the recovery — so the
/// relaxation does not strand content even when a caller aims there.
#[test]
fn a_block_moved_under_root_is_swept_into_today() {
    let (mut w, hlc) = replica();
    let page = open_or_create(&mut w, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    let block = append_block(&mut w, &hlc, Some(page), Some("content")).unwrap();

    outl_actions::block::move_under(&mut w, &hlc, block, NodeId::root()).unwrap();
    assert_eq!(w.tree().parent(block), Some(NodeId::root()));
    assert!(
        !outl_actions::page::list_all(&w)
            .iter()
            .any(|p| p.slug == block.to_string()),
        "a page-less root child is not a page"
    );

    outl_actions::page::migrate_legacy_into_today(&mut w, &hlc).unwrap();
    let today = outl_actions::page::open_today(&mut w, &hlc).unwrap();
    assert_eq!(
        w.tree().parent(block),
        Some(today),
        "the legacy sweep is what keeps this recoverable"
    );
}

/// `trash::restore` asked `contains(parent)` — the same wrong question
/// `move_under` was asking one function away, and `root()` is exactly the
/// parent it answers no for. So a block deleted from **directly under
/// root** could not be restored, and the refusal named the all-zero root id
/// as a missing node.
///
/// Every pre-page-model block lives there, and so does anything
/// `migrate_legacy_into_today` has not swept yet.
#[test]
fn a_block_deleted_from_directly_under_root_can_be_restored() {
    let (mut w, hlc) = replica();
    let block = append_block(&mut w, &hlc, None, Some("a legacy root block")).unwrap();
    assert_eq!(w.tree().parent(block), Some(NodeId::root()));
    delete_block(&mut w, &hlc, block).unwrap();

    assert!(
        outl_actions::trash::refusal_for(&w, block).is_none(),
        "the listing must not refuse what restore can do"
    );
    outl_actions::trash::restore(&mut w, &hlc, block).expect("root is a legal landing parent");

    assert_eq!(w.tree().parent(block), Some(NodeId::root()));
    assert!(!is_trashed(&w, block));
    assert_eq!(w.block_text(block).as_deref(), Some("a legacy root block"));
}

/// The third instance: `create_after_or_append`'s stale-anchor fallback
/// lands at the end of `page`, and `append_in_page` guards that page with
/// `ensure_live` — which reaches `contains` and so answers no for `root()`.
///
/// No caller passes `root()` as the page today, which is the whole reason
/// this needed catching in review rather than by a user: a wrong answer
/// here costs nothing until something reaches it.
#[test]
fn the_stale_anchor_fallback_accepts_root_as_the_page() {
    let (mut w, hlc) = replica();
    append_block(&mut w, &hlc, None, Some("a legacy root block")).unwrap();
    // A stale anchor the caller still holds: never created, so not in the
    // tree, which is exactly what routes this into the fallback.
    let ghost = NodeId::new();

    let made = create_after_or_append(&mut w, &hlc, NodeId::root(), ghost, Some("typed")).unwrap();

    assert_eq!(w.tree().parent(made), Some(NodeId::root()));
    assert_eq!(w.block_text(made).as_deref(), Some("typed"));
}
