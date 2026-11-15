//! Redirect aliases and backlinks of `merge_pages`: what the other
//! pages say about the source after it is gone.

use super::tests::{block, merge, page, prop, put, text_of, ws};
use super::*;
use crate::backlinks_index::build_backlink_index;
use crate::block::edit_text;
use std::path::Path;

#[test]
fn the_sources_slug_and_title_become_aliases_of_the_target() {
    let (mut w, hlc) = ws();
    page(&mut w, &hlc, "alpha", "Alpha Project");
    let beta = page(&mut w, &hlc, "beta", "Beta");

    let report = merge(&mut w, &hlc);

    assert_eq!(report.aliases_added, vec!["alpha", "Alpha Project"]);
    assert_eq!(prop(&w, beta, "alias").unwrap(), "alpha, Alpha Project");
    assert_eq!(report.target.aliases, vec!["alpha", "Alpha Project"]);
}

#[test]
fn a_redirect_name_equal_to_the_targets_own_is_not_added() {
    let (mut w, hlc) = ws();
    // The source's title is the target's slug in another case.
    page(&mut w, &hlc, "alpha", "BETA");
    let beta = page(&mut w, &hlc, "beta", "Beta");

    let report = merge(&mut w, &hlc);

    assert_eq!(report.aliases_added, vec!["alpha"]);
    assert_eq!(prop(&w, beta, "alias").unwrap(), "alpha");
}

#[test]
fn every_link_form_across_the_workspace_follows_the_merge() {
    let (mut w, hlc) = ws();
    page(&mut w, &hlc, "alpha", "Alpha");
    page(&mut w, &hlc, "beta", "Beta");
    let gamma = page(&mut w, &hlc, "gamma", "Gamma");
    let by_ref = block(&mut w, &hlc, gamma, "see [[Alpha]] now");
    let by_tag = block(&mut w, &hlc, gamma, "filed #alpha today");
    let by_slug = block(&mut w, &hlc, gamma, "also [[alpha]]");

    let report = merge(&mut w, &hlc);

    assert_eq!(text_of(&w, by_ref), "see [[Beta]] now");
    assert_eq!(text_of(&w, by_tag), "filed #beta today");
    assert_eq!(text_of(&w, by_slug), "also [[Beta]]");
    assert_eq!(report.rewritten_blocks.len(), 3);
    assert!(report.touched_pages.contains(&"gamma".to_string()));
}

#[test]
fn a_person_mention_follows_the_merge() {
    let (mut w, hlc) = ws();
    let ana = page(&mut w, &hlc, "ana", "Ana");
    let anna = page(&mut w, &hlc, "anna", "Anna");
    put(&mut w, &hlc, ana, "type", "person");
    put(&mut w, &hlc, anna, "type", "person");
    let gamma = page(&mut w, &hlc, "gamma", "Gamma");
    let mention = block(&mut w, &hlc, gamma, "ping [[@Ana]]");

    merge_pages(&mut w, &hlc, "ana", "anna").unwrap();

    assert_eq!(text_of(&w, mention), "ping [[@Anna]]");
}

#[test]
fn code_is_left_untouched() {
    let (mut w, hlc) = ws();
    page(&mut w, &hlc, "alpha", "Alpha");
    page(&mut w, &hlc, "beta", "Beta");
    let gamma = page(&mut w, &hlc, "gamma", "Gamma");
    let code = block(&mut w, &hlc, gamma, "run `[[Alpha]]` as is");

    let report = merge(&mut w, &hlc);

    assert_eq!(text_of(&w, code), "run `[[Alpha]]` as is");
    assert!(report.rewritten_blocks.is_empty());
}

#[test]
fn a_namespace_child_is_reported_not_rewritten() {
    let (mut w, hlc) = ws();
    page(&mut w, &hlc, "alpha", "Alpha");
    page(&mut w, &hlc, "beta", "Beta");
    let gamma = page(&mut w, &hlc, "gamma", "Gamma");
    let child = block(&mut w, &hlc, gamma, "see [[Alpha/notes]]");

    let report = merge(&mut w, &hlc);

    assert_eq!(text_of(&w, child), "see [[Alpha/notes]]");
    assert_eq!(report.skipped_namespace_refs.len(), 1);
    assert_eq!(report.skipped_namespace_refs[0].name, "Alpha/notes");
    assert_eq!(report.skipped_namespace_refs[0].block_id, child.to_string());
}

#[test]
fn a_mention_by_one_of_the_sources_aliases_is_not_rewritten() {
    let (mut w, hlc) = ws();
    let alpha = page(&mut w, &hlc, "alpha", "Alpha");
    let beta = page(&mut w, &hlc, "beta", "Beta");
    put(&mut w, &hlc, alpha, "alias", "Ay");
    let gamma = page(&mut w, &hlc, "gamma", "Gamma");
    let by_alias = block(&mut w, &hlc, gamma, "via [[Ay]]");

    merge(&mut w, &hlc);

    assert_eq!(text_of(&w, by_alias), "via [[Ay]]");
    assert!(prop(&w, beta, "alias").unwrap().contains("Ay"));
}

#[test]
fn an_old_name_written_after_the_merge_is_a_backlink_of_the_target() {
    let (mut w, hlc) = ws();
    page(&mut w, &hlc, "alpha", "Alpha Project");
    page(&mut w, &hlc, "beta", "Beta");
    let gamma = page(&mut w, &hlc, "gamma", "Gamma");
    let by_title = block(&mut w, &hlc, gamma, "draft one");
    let by_slug = block(&mut w, &hlc, gamma, "draft two");
    let report = merge(&mut w, &hlc);

    edit_text(&mut w, &hlc, by_title, "typed later [[Alpha Project]]").unwrap();
    edit_text(&mut w, &hlc, by_slug, "typed later [[alpha]]").unwrap();

    let index = build_backlink_index(&w, Path::new("/nowhere"));
    let found: Vec<String> = index
        .for_page(&w, &report.target)
        .into_iter()
        .map(|b| b.block_id)
        .collect();
    assert!(
        found.contains(&by_title.to_string()),
        "title alias: {found:?}"
    );
    assert!(
        found.contains(&by_slug.to_string()),
        "slug alias: {found:?}"
    );
}

#[test]
fn an_old_name_written_after_the_merge_resolves_to_the_target() {
    let (mut w, hlc) = ws();
    page(&mut w, &hlc, "alpha", "Alpha Project");
    let beta = page(&mut w, &hlc, "beta", "Beta");
    merge(&mut w, &hlc);

    let landed = crate::resolve::resolve_or_create_by_name(&mut w, &hlc, "alpha project").unwrap();

    assert_eq!(landed, beta, "no fresh page is minted for the old name");
}

#[test]
fn rerunning_rewrites_a_mention_a_peer_wrote_after_the_merge() {
    let (mut w, hlc) = ws();
    page(&mut w, &hlc, "alpha", "Alpha");
    page(&mut w, &hlc, "beta", "Beta");
    let gamma = page(&mut w, &hlc, "gamma", "Gamma");
    let later = block(&mut w, &hlc, gamma, "draft");
    merge(&mut w, &hlc);
    edit_text(&mut w, &hlc, later, "late [[Alpha]]").unwrap();

    let rerun = merge(&mut w, &hlc);

    assert_eq!(text_of(&w, later), "late [[Beta]]");
    assert!(rerun.already_merged);
    assert_eq!(rerun.touched_pages, vec!["beta", "gamma"]);
}
