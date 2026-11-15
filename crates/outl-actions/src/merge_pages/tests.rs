//! Blocks, resolution, refusals and idempotency of `merge_pages`.
//! Properties live in `tests_props.rs`, links and aliases in
//! `tests_links.rs`; the helpers are shared from here.

use super::*;
use crate::block::{append_block, delete as delete_block};
use crate::page::{open_journal, open_or_create, read_text_prop};
use crate::tree::children_of;
use outl_core::id::ActorId;
use outl_core::op::{LogOp, Op};
use outl_core::property::PropValue;

pub(super) fn ws() -> (Workspace, HlcGenerator) {
    let actor = ActorId::new();
    (
        Workspace::open_in_memory(actor).unwrap(),
        HlcGenerator::new(actor),
    )
}

pub(super) fn page(w: &mut Workspace, hlc: &HlcGenerator, slug: &str, title: &str) -> NodeId {
    open_or_create(w, hlc, slug, title, PageKind::Page).unwrap()
}

pub(super) fn block(w: &mut Workspace, hlc: &HlcGenerator, parent: NodeId, text: &str) -> NodeId {
    append_block(w, hlc, Some(parent), Some(text)).unwrap()
}

pub(super) fn text_of(w: &Workspace, id: NodeId) -> String {
    w.block_text(id).unwrap_or_default()
}

pub(super) fn kids(w: &Workspace, parent: NodeId) -> Vec<NodeId> {
    children_of(w, parent)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

pub(super) fn kid_texts(w: &Workspace, parent: NodeId) -> Vec<String> {
    kids(w, parent)
        .into_iter()
        .map(|id| text_of(w, id))
        .collect()
}

pub(super) fn prop(w: &Workspace, node: NodeId, key: &str) -> Option<String> {
    read_text_prop(w, node, key)
}

pub(super) fn put(w: &mut Workspace, hlc: &HlcGenerator, node: NodeId, key: &str, value: &str) {
    set_property(w, hlc, node, key, Some(PropValue::Text(value.to_string()))).unwrap();
}

pub(super) fn log_len(w: &Workspace) -> usize {
    w.log().iter().count()
}

pub(super) fn merge(w: &mut Workspace, hlc: &HlcGenerator) -> MergeReport {
    merge_pages(w, hlc, "alpha", "beta").unwrap()
}

#[test]
fn blocks_move_after_the_targets_own_in_order_with_ids_and_subtrees_intact() {
    let (mut w, hlc) = ws();
    let alpha = page(&mut w, &hlc, "alpha", "Alpha");
    let beta = page(&mut w, &hlc, "beta", "Beta");
    let b1 = block(&mut w, &hlc, beta, "b1");
    let a1 = block(&mut w, &hlc, alpha, "a1");
    let a1_child = block(&mut w, &hlc, a1, "a1.1");
    let a2 = block(&mut w, &hlc, alpha, "a2");
    let a3 = block(&mut w, &hlc, alpha, "a3");

    let report = merge(&mut w, &hlc);

    assert_eq!(kids(&w, beta), vec![b1, a1, a2, a3]);
    assert_eq!(kids(&w, a1), vec![a1_child], "the subtree travelled");
    assert_eq!(text_of(&w, a1_child), "a1.1");
    assert!(kids(&w, alpha).is_empty());
    assert_eq!(report.moved_blocks, 3);
    assert_eq!(
        report.moved_block_ids,
        vec![a1.to_string(), a2.to_string(), a3.to_string()]
    );
}

#[test]
fn a_merge_copies_nothing() {
    let (mut w, hlc) = ws();
    let alpha = page(&mut w, &hlc, "alpha", "Alpha");
    page(&mut w, &hlc, "beta", "Beta");
    block(&mut w, &hlc, alpha, "only once");
    let nodes_before = w.tree().iter_nodes().count();

    merge(&mut w, &hlc);

    assert_eq!(w.tree().iter_nodes().count(), nodes_before);
    let beta = find_by_slug(&w, "beta").unwrap();
    assert_eq!(kid_texts(&w, beta), vec!["only once"]);
}

#[test]
fn the_source_is_in_the_trash_afterwards_and_no_longer_found_by_slug() {
    let (mut w, hlc) = ws();
    let alpha = page(&mut w, &hlc, "alpha", "Alpha");
    page(&mut w, &hlc, "beta", "Beta");

    let report = merge(&mut w, &hlc);

    assert!(is_trashed(&w, alpha));
    assert_eq!(find_by_slug(&w, "alpha"), None);
    assert!(report.source_deleted);
    assert!(!report.already_merged);
    assert_eq!(report.source.slug, "alpha");
    assert_eq!(report.target.slug, "beta");
}

#[test]
fn touched_pages_name_the_target_and_never_the_source() {
    let (mut w, hlc) = ws();
    let alpha = page(&mut w, &hlc, "alpha", "Alpha");
    page(&mut w, &hlc, "beta", "Beta");
    let gamma = page(&mut w, &hlc, "gamma", "Gamma");
    block(&mut w, &hlc, gamma, "see [[Alpha]]");
    block(&mut w, &hlc, alpha, "mine");

    let report = merge(&mut w, &hlc);

    assert_eq!(report.touched_pages, vec!["beta", "gamma"]);
}

#[test]
fn source_and_target_resolve_by_page_root_id_too() {
    let (mut w, hlc) = ws();
    let alpha = page(&mut w, &hlc, "alpha", "Alpha");
    let beta = page(&mut w, &hlc, "beta", "Beta");
    let a1 = block(&mut w, &hlc, alpha, "a1");

    merge_pages(&mut w, &hlc, &alpha.to_string(), &beta.to_string()).unwrap();

    assert_eq!(kids(&w, beta), vec![a1]);
}

#[test]
fn a_journal_source_is_refused_and_nothing_is_written() {
    let (mut w, hlc) = ws();
    let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 9).unwrap();
    let day = open_journal(&mut w, &hlc, date).unwrap();
    page(&mut w, &hlc, "beta", "Beta");
    block(&mut w, &hlc, day, "diary");
    let before = log_len(&w);

    let err = merge_pages(&mut w, &hlc, "2026-07-09", "beta").unwrap_err();

    assert!(matches!(err, ActionError::CannotMergeJournal(_)));
    assert_eq!(log_len(&w), before);
    assert!(!is_trashed(&w, day));
}

#[test]
fn a_journal_target_is_allowed() {
    let (mut w, hlc) = ws();
    let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 9).unwrap();
    let day = open_journal(&mut w, &hlc, date).unwrap();
    let alpha = page(&mut w, &hlc, "alpha", "Alpha");
    let a1 = block(&mut w, &hlc, alpha, "a1");

    merge_pages(&mut w, &hlc, "alpha", "2026-07-09").unwrap();

    assert!(kids(&w, day).contains(&a1));
}

#[test]
fn merging_a_page_into_itself_is_refused_by_slug_and_by_id() {
    let (mut w, hlc) = ws();
    let alpha = page(&mut w, &hlc, "alpha", "Alpha");
    let before = log_len(&w);

    let by_slug = merge_pages(&mut w, &hlc, "alpha", "alpha").unwrap_err();
    let by_id = merge_pages(&mut w, &hlc, "alpha", &alpha.to_string()).unwrap_err();

    assert!(matches!(by_slug, ActionError::MergeIntoSelf(_)));
    assert!(matches!(by_id, ActionError::MergeIntoSelf(_)));
    assert_eq!(log_len(&w), before);
}

#[test]
fn an_unknown_source_or_target_is_page_not_found() {
    let (mut w, hlc) = ws();
    page(&mut w, &hlc, "alpha", "Alpha");

    let source = merge_pages(&mut w, &hlc, "nope", "alpha").unwrap_err();
    let target = merge_pages(&mut w, &hlc, "alpha", "nope").unwrap_err();

    assert!(matches!(source, ActionError::PageNotFound(name) if name == "nope"));
    assert!(matches!(target, ActionError::PageNotFound(name) if name == "nope"));
}

#[test]
fn a_trashed_target_is_page_not_found() {
    let (mut w, hlc) = ws();
    page(&mut w, &hlc, "alpha", "Alpha");
    let beta = page(&mut w, &hlc, "beta", "Beta");
    delete_block(&mut w, &hlc, beta).unwrap();

    let err = merge_pages(&mut w, &hlc, "alpha", "beta").unwrap_err();
    let by_id = merge_pages(&mut w, &hlc, "alpha", &beta.to_string()).unwrap_err();

    assert!(matches!(err, ActionError::PageNotFound(_)));
    assert!(matches!(by_id, ActionError::PageNotFound(_)));
}

#[test]
fn a_second_merge_is_a_no_op_that_says_so() {
    let (mut w, hlc) = ws();
    let alpha = page(&mut w, &hlc, "alpha", "Alpha");
    page(&mut w, &hlc, "beta", "Beta");
    block(&mut w, &hlc, alpha, "a1");
    put(&mut w, &hlc, alpha, "alias", "Ay");
    let first = merge(&mut w, &hlc);
    assert!(!first.nothing_changed());
    let before = log_len(&w);

    let second = merge(&mut w, &hlc);

    assert_eq!(log_len(&w), before, "a repeat writes no op");
    assert!(second.already_merged);
    assert!(!second.source_deleted);
    assert!(second.nothing_changed());
    assert_eq!(second.moved_blocks, 0);
    assert_eq!(second.source.slug, "alpha", "the trashed source is found");
}

#[test]
fn a_block_that_reached_the_trashed_source_late_is_moved_by_the_rerun() {
    let (mut w, hlc) = ws();
    let alpha = page(&mut w, &hlc, "alpha", "Alpha");
    let beta = page(&mut w, &hlc, "beta", "Beta");
    let a1 = block(&mut w, &hlc, alpha, "a1");
    merge(&mut w, &hlc);

    // A peer that had not seen the delete appended under the old root.
    // Actions refuse a trashed parent, so deliver the op the way sync does.
    let late = NodeId::new();
    let ts = hlc.next();
    let position = crate::tree::position_for_new_last_child(&w, alpha);
    w.apply(LogOp {
        ts,
        actor: ts.actor,
        op: Op::Create {
            node: late,
            parent: alpha,
            position,
        },
    })
    .unwrap();

    let rerun = merge(&mut w, &hlc);

    assert!(rerun.already_merged);
    assert_eq!(rerun.moved_block_ids, vec![late.to_string()]);
    assert_eq!(kids(&w, beta), vec![a1, late]);
    assert!(kids(&w, alpha).is_empty());
    let again = merge(&mut w, &hlc);
    assert!(again.nothing_changed());
}
