//! The rename battery: what a rename writes, what it refuses before writing
//! anything, and what it leaves for the caller to project.

use super::*;
use crate::block::append_block;
use crate::page::{open_journal, open_or_create};
use crate::person::{PERSON_TYPE, TYPE_KEY};
use crate::tree::children_of;
use chrono::NaiveDate;
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

fn text_of(w: &Workspace, id: NodeId) -> String {
    w.block_text(id).unwrap_or_default()
}

fn ops(w: &Workspace) -> usize {
    w.log().len()
}

fn slugs(w: &Workspace) -> Vec<String> {
    crate::page::list_all(w)
        .into_iter()
        .map(|m| m.slug)
        .collect()
}

/// A page `alpha` titled `Alpha Project`, the setup most tests rename.
fn alpha(w: &mut Workspace, hlc: &HlcGenerator) -> NodeId {
    page(w, hlc, "alpha", "Alpha Project")
}

#[test]
fn a_slug_rename_moves_the_page_and_keeps_its_id_and_blocks() {
    let (mut w, hlc) = ws();
    let id = alpha(&mut w, &hlc);
    let child = block(&mut w, &hlc, id, "first");

    let report = rename_page(&mut w, &hlc, "alpha", "beta", None).unwrap();

    assert_eq!(find_by_slug(&w, "alpha"), None);
    assert_eq!(find_by_slug(&w, "beta"), Some(id));
    assert_eq!(report.meta.id, id.to_string());
    assert_eq!(report.meta.slug, "beta");
    assert_eq!(report.old_slug, "alpha");
    assert_eq!(report.old_title, "Alpha Project");
    assert_eq!(report.meta.title, "Alpha Project", "title is left alone");
    let kids: Vec<NodeId> = children_of(&w, id).into_iter().map(|(n, _)| n).collect();
    assert_eq!(kids, vec![child]);
    assert_eq!(text_of(&w, child), "first");
    assert_eq!(slugs(&w), vec!["beta".to_string()]);
}

#[test]
fn a_title_only_rename_keeps_the_slug_and_the_file() {
    let (mut w, hlc) = ws();
    let id = alpha(&mut w, &hlc);

    let report = rename_page(&mut w, &hlc, "alpha", "alpha", Some("  Alpha Two  ")).unwrap();

    assert_eq!(find_by_slug(&w, "alpha"), Some(id));
    assert_eq!(report.meta.slug, "alpha");
    assert_eq!(report.meta.title, "Alpha Two", "the title is trimmed");
    assert_eq!(report.old_title, "Alpha Project");
    assert_eq!(report.touched_pages, vec!["alpha".to_string()]);
    assert!(report.stale_projection.is_none(), "the file does not move");
}

#[test]
fn a_slug_and_title_rename_sets_both() {
    let (mut w, hlc) = ws();
    let id = alpha(&mut w, &hlc);

    let report = rename_page(&mut w, &hlc, "alpha", "beta", Some("Beta Project")).unwrap();

    assert_eq!(find_by_slug(&w, "beta"), Some(id));
    assert_eq!(report.meta.title, "Beta Project");
    assert_eq!(page_meta(&w, id).unwrap().title, "Beta Project");
    assert_eq!(report.stale_projection.unwrap().slug, "alpha");
}

#[test]
fn a_title_derived_from_the_slug_follows_the_new_slug() {
    let (mut w, hlc) = ws();
    // `title == slug`, so `open_or_create` stores no `title::` property.
    let id = page(&mut w, &hlc, "alpha", "alpha");
    let other = page(&mut w, &hlc, "notes", "Notes");
    let mention = block(&mut w, &hlc, other, "see [[alpha]]");

    let report = rename_page(&mut w, &hlc, "alpha", "beta", None).unwrap();

    assert_eq!(report.old_title, "alpha");
    assert_eq!(report.meta.title, "beta");
    assert_eq!(page_meta(&w, id).unwrap().title, "beta");
    assert_eq!(text_of(&w, mention), "see [[beta]]");
}

#[test]
fn an_explicit_title_equal_to_a_derived_one_is_pinned() {
    let (mut w, hlc) = ws();
    let id = page(&mut w, &hlc, "alpha", "alpha");

    let report = rename_page(&mut w, &hlc, "alpha", "beta", Some("alpha")).unwrap();

    assert_eq!(report.meta.slug, "beta");
    assert_eq!(report.meta.title, "alpha", "asked for, so it stays");
    assert_eq!(page_meta(&w, id).unwrap().title, "alpha");
}

#[test]
fn ref_and_tag_mentions_are_rewritten_on_other_pages_and_on_the_page_itself() {
    let (mut w, hlc) = ws();
    let id = alpha(&mut w, &hlc);
    let own = block(&mut w, &hlc, id, "self [[alpha]]");
    let other = page(&mut w, &hlc, "notes", "Notes");
    let by_ref = block(&mut w, &hlc, other, "see [[alpha]] now");
    let by_tag = block(&mut w, &hlc, other, "filed #alpha.");
    let by_title = block(&mut w, &hlc, other, "see [[Alpha Project]]");
    let unrelated = block(&mut w, &hlc, other, "nothing here");

    let report = rename_page(&mut w, &hlc, "alpha", "beta", Some("Beta Project")).unwrap();

    assert_eq!(text_of(&w, own), "self [[Beta Project]]");
    assert_eq!(text_of(&w, by_ref), "see [[Beta Project]] now");
    assert_eq!(text_of(&w, by_tag), "filed #beta.");
    assert_eq!(text_of(&w, by_title), "see [[Beta Project]]");
    assert_eq!(text_of(&w, unrelated), "nothing here");

    let mut rewritten = report.rewritten_blocks.clone();
    rewritten.sort();
    let mut expected: Vec<String> = [own, by_ref, by_tag, by_title]
        .iter()
        .map(|b| b.to_string())
        .collect();
    expected.sort();
    assert_eq!(rewritten, expected);
    assert_eq!(
        report.touched_pages,
        vec!["beta".to_string(), "notes".to_string()],
        "the renamed page's own block is reported under its new slug"
    );
}

#[test]
fn touched_pages_are_sorted_and_deduplicated() {
    let (mut w, hlc) = ws();
    alpha(&mut w, &hlc);
    let zeta = page(&mut w, &hlc, "zeta", "Zeta");
    let gamma = page(&mut w, &hlc, "gamma", "Gamma");
    block(&mut w, &hlc, zeta, "[[alpha]]");
    block(&mut w, &hlc, zeta, "#alpha");
    block(&mut w, &hlc, gamma, "[[alpha]]");

    let report = rename_page(&mut w, &hlc, "alpha", "beta", None).unwrap();

    assert_eq!(
        report.touched_pages,
        vec!["beta".to_string(), "gamma".to_string(), "zeta".to_string()]
    );
}

#[test]
fn person_mentions_follow_the_rename() {
    let (mut w, hlc) = ws();
    let ada = page(&mut w, &hlc, "ada", "Ada");
    set_property(
        &mut w,
        &hlc,
        ada,
        TYPE_KEY,
        Some(PropValue::Text(PERSON_TYPE.to_string())),
    )
    .unwrap();
    let other = page(&mut w, &hlc, "notes", "Notes");
    let ping = block(&mut w, &hlc, other, "ping [[@ada]] and [[Ada]]");

    let report = rename_page(&mut w, &hlc, "ada", "grace", Some("Grace")).unwrap();

    assert_eq!(text_of(&w, ping), "ping [[@Grace]] and [[Grace]]");
    assert_eq!(report.rewritten_blocks, vec![ping.to_string()]);
}

#[test]
fn a_code_span_is_not_rewritten() {
    let (mut w, hlc) = ws();
    alpha(&mut w, &hlc);
    let other = page(&mut w, &hlc, "notes", "Notes");
    let mixed = block(&mut w, &hlc, other, "`[[alpha]]` and [[alpha]]");

    rename_page(&mut w, &hlc, "alpha", "beta", Some("Beta Project")).unwrap();

    assert_eq!(text_of(&w, mixed), "`[[alpha]]` and [[Beta Project]]");
}

#[test]
fn a_namespace_child_is_reported_not_rewritten() {
    let (mut w, hlc) = ws();
    alpha(&mut w, &hlc);
    let other = page(&mut w, &hlc, "notes", "Notes");
    let child = block(&mut w, &hlc, other, "see [[alpha/child]]");

    let report = rename_page(&mut w, &hlc, "alpha", "beta", Some("Beta Project")).unwrap();

    assert_eq!(text_of(&w, child), "see [[alpha/child]]");
    assert!(report.rewritten_blocks.is_empty());
    assert_eq!(
        report.skipped_namespace_refs,
        vec![SkippedRef {
            block_id: child.to_string(),
            name: "alpha/child".to_string(),
        }]
    );
}

#[test]
fn a_mention_that_resolves_to_another_page_is_left_alone() {
    let (mut w, hlc) = ws();
    alpha(&mut w, &hlc);
    // `[[Alpha Project]]` slug-resolves to `alpha-project`, so navigation
    // never lands on `alpha` for it, even though it is `alpha`'s title.
    let rival = page(&mut w, &hlc, "alpha-project", "Rival");
    let other = page(&mut w, &hlc, "notes", "Notes");
    let aimed_at_rival = block(&mut w, &hlc, other, "see [[Alpha Project]]");
    let aimed_at_alpha = block(&mut w, &hlc, other, "see [[alpha]]");

    rename_page(&mut w, &hlc, "alpha", "beta", Some("Beta Project")).unwrap();

    assert_eq!(text_of(&w, aimed_at_rival), "see [[Alpha Project]]");
    assert_eq!(text_of(&w, aimed_at_alpha), "see [[Beta Project]]");
    assert_eq!(find_by_slug(&w, "alpha-project"), Some(rival));
}

#[test]
fn a_rename_onto_an_existing_slug_is_refused_and_writes_no_op() {
    let (mut w, hlc) = ws();
    alpha(&mut w, &hlc);
    let taken = page(&mut w, &hlc, "beta", "Beta");
    let other = page(&mut w, &hlc, "notes", "Notes");
    let mention = block(&mut w, &hlc, other, "see [[alpha]]");
    let before = ops(&w);

    let err = rename_page(&mut w, &hlc, "alpha", "beta", Some("Other")).unwrap_err();

    match err {
        ActionError::PageExists { existing } => assert_eq!(existing, "beta"),
        other => panic!("expected PageExists, got {other:?}"),
    }
    assert_eq!(ops(&w), before, "nothing is partially applied");
    assert!(find_by_slug(&w, "alpha").is_some());
    assert_eq!(find_by_slug(&w, "beta"), Some(taken));
    assert_eq!(text_of(&w, mention), "see [[alpha]]");
    assert_eq!(page_meta(&w, taken).unwrap().title, "Beta");
}

#[test]
fn a_journal_cannot_be_renamed() {
    let (mut w, hlc) = ws();
    let date = NaiveDate::from_ymd_opt(2031, 3, 4).unwrap();
    open_journal(&mut w, &hlc, date).unwrap();
    let before = ops(&w);

    let err = rename_page(&mut w, &hlc, "2031-03-04", "diary", None).unwrap_err();

    assert!(matches!(err, ActionError::CannotRenameJournal(s) if s == "2031-03-04"));
    assert_eq!(ops(&w), before);
}

#[test]
fn an_invalid_new_slug_is_refused() {
    let (mut w, hlc) = ws();
    alpha(&mut w, &hlc);
    let before = ops(&w);

    for bad in ["", "a/b", "..", " padded ", "line\nbreak"] {
        let err = rename_page(&mut w, &hlc, "alpha", bad, None).unwrap_err();
        assert!(
            matches!(&err, ActionError::InvalidSlug(s) if s == bad),
            "{bad:?} gave {err:?}"
        );
    }
    assert_eq!(ops(&w), before);
}

#[test]
fn an_unknown_page_is_refused() {
    let (mut w, hlc) = ws();

    let err = rename_page(&mut w, &hlc, "ghost", "beta", None).unwrap_err();

    assert!(matches!(err, ActionError::PageNotFound(s) if s == "ghost"));
}

#[test]
fn a_blank_or_multiline_title_is_refused_before_any_op() {
    let (mut w, hlc) = ws();
    alpha(&mut w, &hlc);
    let before = ops(&w);

    for bad in ["", "   ", "two\nlines"] {
        let err = rename_page(&mut w, &hlc, "alpha", "beta", Some(bad)).unwrap_err();
        assert!(matches!(err, ActionError::InvalidSlug(_)), "{bad:?}");
    }
    assert_eq!(ops(&w), before);
    assert!(find_by_slug(&w, "alpha").is_some());
}

#[test]
fn renaming_to_the_same_name_is_a_no_op_with_an_empty_report() {
    let (mut w, hlc) = ws();
    let id = alpha(&mut w, &hlc);
    let other = page(&mut w, &hlc, "notes", "Notes");
    block(&mut w, &hlc, other, "see [[alpha]]");
    let before = ops(&w);

    let bare = rename_page(&mut w, &hlc, "alpha", "alpha", None).unwrap();
    let same_title = rename_page(&mut w, &hlc, "alpha", "alpha", Some("Alpha Project")).unwrap();

    assert_eq!(ops(&w), before, "no op for a rename that changes nothing");
    for report in [bare, same_title] {
        assert_eq!(report.meta.id, id.to_string());
        assert!(report.rewritten_blocks.is_empty());
        assert!(report.skipped_namespace_refs.is_empty());
        assert!(report.touched_pages.is_empty());
        assert!(report.stale_projection.is_none());
    }
}

#[test]
fn the_stale_projection_is_the_old_meta_and_only_when_the_slug_changed() {
    let (mut w, hlc) = ws();
    alpha(&mut w, &hlc);

    let retitled = rename_page(&mut w, &hlc, "alpha", "alpha", Some("Renamed")).unwrap();
    assert!(retitled.stale_projection.is_none());

    let moved = rename_page(&mut w, &hlc, "alpha", "beta", None).unwrap();
    let stale = moved.stale_projection.expect("slug changed");
    assert_eq!(stale.slug, "alpha");
    assert_eq!(stale.title, "Renamed");
    assert_eq!(moved.meta.slug, "beta");
}

#[test]
fn repeating_a_rename_finds_no_page_under_the_old_slug() {
    let (mut w, hlc) = ws();
    alpha(&mut w, &hlc);
    rename_page(&mut w, &hlc, "alpha", "beta", None).unwrap();
    let before = ops(&w);

    let again = rename_page(&mut w, &hlc, "alpha", "beta", None).unwrap_err();
    assert!(matches!(again, ActionError::PageNotFound(s) if s == "alpha"));

    let settled = rename_page(&mut w, &hlc, "beta", "beta", None).unwrap();
    assert!(settled.touched_pages.is_empty());
    assert_eq!(ops(&w), before, "neither call wrote anything");
}

#[test]
fn rewriting_after_a_rename_is_idempotent() {
    let (mut w, hlc) = ws();
    alpha(&mut w, &hlc);
    let other = page(&mut w, &hlc, "notes", "Notes");
    block(&mut w, &hlc, other, "see [[alpha]]");
    rename_page(&mut w, &hlc, "alpha", "beta", Some("Beta Project")).unwrap();
    let before = ops(&w);

    let from = PageNames {
        slug: "alpha".to_string(),
        title: "Alpha Project".to_string(),
        person: false,
    };
    let to = PageNames::of(&page_meta(&w, find_by_slug(&w, "beta").unwrap()).unwrap());
    let again = rewrite_mentions(&mut w, &hlc, &from, &to).unwrap();

    assert!(again.rewritten_blocks.is_empty());
    assert_eq!(ops(&w), before);
}

#[test]
fn known_limit_reopening_the_old_slug_renames_the_page_back() {
    let (mut w, hlc) = ws();
    let id = alpha(&mut w, &hlc);
    rename_page(&mut w, &hlc, "alpha", "beta", None).unwrap();

    // The deterministic id for `alpha` is the renamed page's own id, so a
    // reopen lands on that node and writes the old slug onto it.
    let reopened = page(&mut w, &hlc, "alpha", "Alpha Project");

    assert_eq!(reopened, id);
    assert_eq!(find_by_slug(&w, "alpha"), Some(id));
    assert_eq!(find_by_slug(&w, "beta"), None);
}
