//! Backlink breadcrumb (`Backlink::ancestors`): a reference buried in a
//! nested outline carries the chain of ancestor blocks — root-first,
//! excluding the page root — so every client can render the branch a
//! reference belongs to as dimmed context above the citing block.
//!
//! Kept out of `src/backlinks.rs` (already near the module-size guard)
//! and driven purely through the public API, which is all the
//! breadcrumb touches.

use std::path::Path;

use chrono::NaiveDate;
use outl_actions::backlinks::backlinks_for_page;
use outl_actions::block::append_block;
use outl_actions::page::{open_journal, open_or_create, page_meta, PageKind, PageMeta};
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::workspace::Workspace;

fn ws() -> (Workspace, HlcGenerator) {
    let actor = ActorId::new();
    (
        Workspace::open_in_memory(actor).unwrap(),
        HlcGenerator::new(actor),
    )
}

fn root() -> &'static Path {
    Path::new("/tmp/outl-test")
}

/// The `metas` page every scenario references, plus its meta so we can
/// pull its backlinks.
fn target_metas(w: &mut Workspace, hlc: &HlcGenerator) -> PageMeta {
    let id = open_or_create(w, hlc, "metas", "metas", PageKind::Page).unwrap();
    page_meta(w, id).unwrap()
}

/// Convenience: the ancestor texts (root-first) of the backlink whose
/// block id matches `block_id`.
fn crumbs_of(w: &mut Workspace, hlc: &HlcGenerator, block_id: &str) -> Vec<String> {
    let meta = target_metas(w, hlc);
    let links = backlinks_for_page(w, root(), &meta);
    let bl = links
        .iter()
        .find(|l| l.block_id == block_id)
        .unwrap_or_else(|| panic!("no backlink for block {block_id}; got {links:#?}"));
    bl.ancestors.iter().map(|c| c.text.clone()).collect()
}

#[test]
fn root_level_reference_has_no_ancestors() {
    let (mut w, hlc) = ws();
    let _ = target_metas(&mut w, &hlc);
    let day = open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 7, 20).unwrap()).unwrap();
    let leaf = append_block(&mut w, &hlc, Some(day), Some("revisar [[metas]] sexta")).unwrap();

    assert!(
        crumbs_of(&mut w, &hlc, &leaf.to_string()).is_empty(),
        "a root-level reference needs no breadcrumb"
    );
}

#[test]
fn one_level_deep_carries_the_immediate_parent() {
    let (mut w, hlc) = ws();
    let _ = target_metas(&mut w, &hlc);
    let day = open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 7, 20).unwrap()).unwrap();
    let parent = append_block(&mut w, &hlc, Some(day), Some("Retro da sprint")).unwrap();
    let leaf = append_block(
        &mut w,
        &hlc,
        Some(parent),
        Some("fechamos abaixo das [[metas]]"),
    )
    .unwrap();

    assert_eq!(
        crumbs_of(&mut w, &hlc, &leaf.to_string()),
        vec!["Retro da sprint".to_string()]
    );
}

#[test]
fn deeper_reference_carries_the_full_chain_root_first() {
    let (mut w, hlc) = ws();
    let _ = target_metas(&mut w, &hlc);
    let day = open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 7, 20).unwrap()).unwrap();
    let p1 = append_block(&mut w, &hlc, Some(day), Some("Planejamento Q3")).unwrap();
    let p2 = append_block(&mut w, &hlc, Some(p1), Some("Objetivos")).unwrap();
    let leaf = append_block(
        &mut w,
        &hlc,
        Some(p2),
        Some("bater as [[metas]] de receita"),
    )
    .unwrap();

    // Root-first: direct child of the page root comes first, the
    // immediate parent last.
    assert_eq!(
        crumbs_of(&mut w, &hlc, &leaf.to_string()),
        vec!["Planejamento Q3".to_string(), "Objetivos".to_string()]
    );
}

#[test]
fn ancestor_todo_prefix_is_stripped_from_the_crumb() {
    let (mut w, hlc) = ws();
    let _ = target_metas(&mut w, &hlc);
    let day = open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 7, 20).unwrap()).unwrap();
    let parent = append_block(&mut w, &hlc, Some(day), Some("TODO Planejamento Q3")).unwrap();
    let leaf = append_block(&mut w, &hlc, Some(parent), Some("mover as [[metas]]")).unwrap();

    assert_eq!(
        crumbs_of(&mut w, &hlc, &leaf.to_string()),
        vec!["Planejamento Q3".to_string()],
        "the breadcrumb shows the plain text, not the TODO marker"
    );
}

#[test]
fn siblings_share_the_same_ancestor_prefix() {
    // Two references in the same branch must expose the same breadcrumb
    // prefix — that shared prefix is exactly what the clients collapse
    // so they don't repeat the branch header per reference.
    let (mut w, hlc) = ws();
    let _ = target_metas(&mut w, &hlc);
    let day = open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 7, 20).unwrap()).unwrap();
    let p1 = append_block(&mut w, &hlc, Some(day), Some("Planejamento Q3")).unwrap();
    let a = append_block(&mut w, &hlc, Some(p1), Some("meta A das [[metas]]")).unwrap();
    let b = append_block(&mut w, &hlc, Some(p1), Some("meta B das [[metas]]")).unwrap();

    let branch = vec!["Planejamento Q3".to_string()];
    assert_eq!(crumbs_of(&mut w, &hlc, &a.to_string()), branch);
    assert_eq!(crumbs_of(&mut w, &hlc, &b.to_string()), branch);
}

#[test]
fn alias_mention_lands_in_the_pages_backlinks() {
    // `alias:: Big Apple, NYC` on the New York page: a block holding
    // `[[NYC]]` must surface under that page's backlinks, not only
    // under the title spelling (issue #302). `keys_for_page` emits the
    // alias as a Ref and a slug Tag exactly like the title.
    let (mut w, hlc) = ws();
    let page = open_or_create(&mut w, &hlc, "new-york", "New York", PageKind::Page).unwrap();
    outl_actions::page::set_property(
        &mut w,
        &hlc,
        page,
        outl_actions::page::ALIAS_KEY,
        Some(outl_core::property::PropValue::Text(
            "Big Apple, NYC".into(),
        )),
    )
    .unwrap();
    let other = open_or_create(&mut w, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    let _ = append_block(
        &mut w,
        &hlc,
        Some(other),
        Some("see [[NYC]] and [[Big Apple]]"),
    );

    let meta = page_meta(&w, page).unwrap();
    let bl = backlinks_for_page(&w, root(), &meta);
    assert_eq!(bl.len(), 1, "one block mentions both aliases");
    assert!(bl[0].source_block.text.contains("[[NYC]]"));
}
