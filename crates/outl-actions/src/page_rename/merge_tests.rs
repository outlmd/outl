//! `rename_or_merge`: the collision policy on top of `rename_page`.

use super::*;
use crate::block::append_block;
use crate::page::open_or_create;
use crate::tree::children_of;
use outl_core::id::ActorId;

fn ws() -> (Workspace, HlcGenerator) {
    let actor = ActorId::new();
    (
        Workspace::open_in_memory(actor).unwrap(),
        HlcGenerator::new(actor),
    )
}

fn page(w: &mut Workspace, hlc: &HlcGenerator, slug: &str, title: &str) -> NodeId {
    open_or_create(w, hlc, slug, title, PageKind::Page).unwrap()
}

fn block(w: &mut Workspace, hlc: &HlcGenerator, parent: NodeId, text: &str) -> NodeId {
    append_block(w, hlc, Some(parent), Some(text)).unwrap()
}

fn kids(w: &Workspace, page: NodeId) -> Vec<NodeId> {
    children_of(w, page).into_iter().map(|(n, _)| n).collect()
}

#[test]
fn refuse_returns_page_exists_and_writes_nothing() {
    let (mut w, hlc) = ws();
    let src = page(&mut w, &hlc, "alpha", "Alpha");
    page(&mut w, &hlc, "beta", "Beta");
    block(&mut w, &hlc, src, "kept");
    let before = w.log().len();

    let err =
        rename_or_merge(&mut w, &hlc, "alpha", "beta", None, OnExisting::Refuse).unwrap_err();

    assert!(matches!(err, ActionError::PageExists { existing } if existing == "beta"));
    assert_eq!(w.log().len(), before);
    assert!(find_by_slug(&w, "alpha").is_some());
}

#[test]
fn merge_into_folds_the_old_page_into_the_existing_one() {
    let (mut w, hlc) = ws();
    let src = page(&mut w, &hlc, "alpha", "Alpha");
    let target = page(&mut w, &hlc, "beta", "Beta");
    let moved = block(&mut w, &hlc, src, "from alpha");
    let own = block(&mut w, &hlc, target, "from beta");
    let other = page(&mut w, &hlc, "notes", "Notes");
    let mention = block(&mut w, &hlc, other, "see [[alpha]]");

    let outcome = rename_or_merge(
        &mut w,
        &hlc,
        "alpha",
        "beta",
        Some("Ignored"),
        OnExisting::MergeInto,
    )
    .unwrap();

    let RenameOutcome::Merged(report) = outcome else {
        panic!("expected Merged, got {outcome:?}");
    };
    assert_eq!(report.target.slug, "beta");
    assert_eq!(find_by_slug(&w, "alpha"), None, "the old slug is gone");
    assert_eq!(find_by_slug(&w, "beta"), Some(target));
    let live = kids(&w, target);
    assert!(live.contains(&moved) && live.contains(&own));
    assert_eq!(
        page_meta(&w, target).unwrap().title,
        "Beta",
        "the existing page keeps its name, the new title is ignored"
    );
    assert_eq!(
        w.block_text(mention).unwrap_or_default(),
        "see [[Beta]]",
        "mentions of the old page follow the merge"
    );
}

#[test]
fn without_a_collision_it_renames_whatever_the_policy() {
    for policy in [OnExisting::Refuse, OnExisting::MergeInto] {
        let (mut w, hlc) = ws();
        let id = page(&mut w, &hlc, "alpha", "Alpha");

        let outcome = rename_or_merge(&mut w, &hlc, "alpha", "beta", None, policy).unwrap();

        let RenameOutcome::Renamed(report) = outcome else {
            panic!("expected Renamed under {policy:?}");
        };
        assert_eq!(report.meta.id, id.to_string());
        assert_eq!(find_by_slug(&w, "beta"), Some(id));
    }
}

#[test]
fn other_refusals_pass_through_under_merge_into() {
    let (mut w, hlc) = ws();
    page(&mut w, &hlc, "alpha", "Alpha");

    let unknown = rename_or_merge(&mut w, &hlc, "ghost", "beta", None, OnExisting::MergeInto)
        .unwrap_err();
    assert!(matches!(unknown, ActionError::PageNotFound(s) if s == "ghost"));

    let invalid = rename_or_merge(&mut w, &hlc, "alpha", "a/b", None, OnExisting::MergeInto)
        .unwrap_err();
    assert!(matches!(invalid, ActionError::InvalidSlug(_)));
}

#[test]
fn the_outcome_serialises_with_a_tag() {
    let (mut w, hlc) = ws();
    page(&mut w, &hlc, "alpha", "Alpha");

    let outcome =
        rename_or_merge(&mut w, &hlc, "alpha", "beta", None, OnExisting::Refuse).unwrap();
    let json = serde_json::to_value(&outcome).unwrap();

    assert_eq!(json["outcome"], "renamed");
    assert_eq!(json["meta"]["slug"], "beta");
}
