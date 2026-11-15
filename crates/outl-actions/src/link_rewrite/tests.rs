//! The link rewriter's battery: every spelling it rewrites, every one it
//! must leave alone, and the shape of what it writes to the op log.

use super::*;
use crate::block::{append_block, delete};
use crate::page::{open_or_create, page_meta, set_property, PageKind};
use outl_core::id::ActorId;
use outl_core::property::PropValue;

fn names(slug: &str, title: &str) -> PageNames {
    PageNames {
        slug: slug.into(),
        title: title.into(),
        person: false,
    }
}

fn person(slug: &str, title: &str) -> PageNames {
    PageNames {
        person: true,
        ..names(slug, title)
    }
}

fn alpha() -> PageNames {
    names("alpha", "Alpha Project")
}

fn beta() -> PageNames {
    names("beta", "Beta Project")
}

fn rewrite(text: &str) -> Option<String> {
    rewrite_text(text, &alpha(), &beta())
}

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

fn names_of(w: &Workspace, id: NodeId) -> PageNames {
    PageNames::of(&page_meta(w, id).unwrap())
}

// --- pure text: what is rewritten -----------------------------------------

#[test]
fn a_ref_by_slug_is_rewritten_to_the_new_title() {
    assert_eq!(
        rewrite("see [[alpha]] now").as_deref(),
        Some("see [[Beta Project]] now")
    );
}

#[test]
fn a_ref_by_title_is_rewritten_ignoring_case() {
    assert_eq!(
        rewrite("see [[ALPHA project]]").as_deref(),
        Some("see [[Beta Project]]")
    );
}

#[test]
fn a_ref_by_slugified_title_is_rewritten() {
    // `slugify("Alpha Project")` is `alpha-project`: a page whose slug
    // is that is reached by that spelling, and so is a ref to it.
    let from = names("alpha-project", "Alpha Project");
    assert_eq!(
        rewrite_text("[[alpha project]]", &from, &beta()).as_deref(),
        Some("[[Beta Project]]")
    );
}

#[test]
fn a_tag_is_rewritten_to_the_new_slug() {
    assert_eq!(rewrite("ship #alpha").as_deref(), Some("ship #beta"));
    assert_eq!(rewrite("ship #Alpha").as_deref(), Some("ship #beta"));
}

#[test]
fn a_tag_followed_by_punctuation_keeps_the_punctuation() {
    assert_eq!(
        rewrite("(#alpha), then #alpha. Also #alpha!").as_deref(),
        Some("(#beta), then #beta. Also #beta!")
    );
}

#[test]
fn a_ref_inside_emphasis_is_rewritten() {
    let text = "**[[alpha]]** and *[[Alpha Project]]* and ~~#alpha~~ and ==[[alpha]]==";
    let want = "**[[Beta Project]]** and *[[Beta Project]]* and ~~#beta~~ and ==[[Beta Project]]==";
    assert_eq!(rewrite(text).as_deref(), Some(want));
}

#[test]
fn every_mention_in_one_block_is_rewritten_with_offsets_intact() {
    // Replacements change the length, so a wrong offset after the first
    // splice would corrupt the second.
    assert_eq!(
        rewrite("[[alpha]] é [[Alpha Project]] ü #alpha").as_deref(),
        Some("[[Beta Project]] é [[Beta Project]] ü #beta")
    );
}

#[test]
fn a_tag_the_slug_cannot_spell_becomes_a_ref() {
    // `#v1.2` is the tag `v1` followed by `.2`, so it cannot name this page.
    let to = names("v1.2", "Version One");
    assert_eq!(
        rewrite_text("#alpha", &alpha(), &to).as_deref(),
        Some("[[Version One]]")
    );
}

#[test]
fn a_title_that_cannot_sit_in_brackets_falls_back_to_the_slug() {
    let to = names("beta", "a]]b");
    assert_eq!(
        rewrite_text("[[alpha]]", &alpha(), &to).as_deref(),
        Some("[[beta]]")
    );
}

// --- pure text: person mentions -------------------------------------------

#[test]
fn a_person_mention_keeps_its_at_when_the_target_is_a_person() {
    let from = person("ana", "Ana Souza");
    let to = person("ana-lima", "Ana Lima");
    let text = "ping [[@Ana Souza]] and [[@ana]] and [[Ana Souza]]";
    assert_eq!(
        rewrite_text(text, &from, &to).as_deref(),
        Some("ping [[@Ana Lima]] and [[@Ana Lima]] and [[Ana Lima]]")
    );
}

#[test]
fn a_person_mention_loses_its_at_when_the_target_is_not_a_person() {
    let from = person("ana", "Ana Souza");
    let to = names("ana-project", "Ana Project");
    assert_eq!(
        rewrite_text("ping [[@Ana Souza]]", &from, &to).as_deref(),
        Some("ping [[Ana Project]]")
    );
}

#[test]
fn an_at_mention_is_not_a_mention_of_an_ordinary_page() {
    // Backlinks index the `@` spellings of a person page only.
    assert_eq!(rewrite("[[@alpha]]"), None);
}

// --- pure text: what is left alone ----------------------------------------

#[test]
fn a_code_span_is_untouched() {
    assert_eq!(rewrite("`[[alpha]]` and `#alpha`"), None);
    assert_eq!(
        rewrite("`[[alpha]]` but [[alpha]]").as_deref(),
        Some("`[[alpha]]` but [[Beta Project]]")
    );
}

#[test]
fn a_backtick_fence_is_untouched() {
    let code = "```rust\nlet x = \"[[alpha]]\"; // #alpha\n```";
    let text = format!("intro [[alpha]]\n{code}\noutro #alpha");
    let want = format!("intro [[Beta Project]]\n{code}\noutro #beta");
    assert_eq!(rewrite(&text), Some(want));
    assert_eq!(rewrite("```\n[[alpha]]\n```"), None);
}

#[test]
fn a_tilde_fence_and_an_unclosed_fence_are_untouched() {
    assert_eq!(rewrite("~~~\n[[alpha]]\n~~~"), None);
    // The other fence character inside is content, not a closer.
    assert_eq!(rewrite("~~~\n```\n[[alpha]]\n~~~"), None);
    // Unclosed: runs to the end, as the outline parser reads it.
    assert_eq!(rewrite("```\n[[alpha]]\nmore #alpha"), None);
}

#[test]
fn text_after_a_closed_fence_is_prose_again() {
    assert_eq!(
        rewrite("```\ncode\n```\n[[alpha]]").as_deref(),
        Some("```\ncode\n```\n[[Beta Project]]")
    );
}

#[test]
fn a_link_url_is_untouched() {
    assert_eq!(rewrite("[label]([[alpha]])"), None);
    assert_eq!(rewrite("[the page](https://example.test/#alpha)"), None);
    assert_eq!(rewrite("![img](#alpha)"), None);
}

#[test]
fn block_refs_and_embeds_are_untouched() {
    assert_eq!(rewrite("((blk-alpha1)) !((blk-alpha1))"), None);
}

#[test]
fn a_name_that_only_contains_the_slug_is_untouched() {
    assert_eq!(rewrite("[[alphabet]] and [[alpha beta]] and [[the alpha]]"), None);
    assert_eq!(rewrite("#alpha-two and #alphabet and #xalpha"), None);
}

#[test]
fn a_ref_already_naming_the_target_is_not_an_edit() {
    assert_eq!(rewrite("[[Beta Project]] and #beta"), None);
    // Same slug, different title: only the title spelling moves.
    let renamed = names("alpha", "Alpha Renamed");
    let text = "[[Alpha Project]] #alpha [[Alpha Renamed]]";
    assert_eq!(
        rewrite_text(text, &alpha(), &renamed).as_deref(),
        Some("[[Alpha Renamed]] #alpha [[Alpha Renamed]]")
    );
}

#[test]
fn an_ambiguous_name_is_left_when_the_workspace_says_it_is_elsewhere() {
    let elsewhere = |name: &str| name == "Alpha Project";
    let text = "[[Alpha Project]] [[alpha]]";
    assert_eq!(
        rewrite_text_with(text, &alpha(), &beta(), &elsewhere).as_deref(),
        Some("[[Alpha Project]] [[Beta Project]]")
    );
}

// --- workspace: rewrite_mentions ------------------------------------------

struct World {
    w: Workspace,
    hlc: HlcGenerator,
    from: PageNames,
    to: PageNames,
    notes: NodeId,
}

/// Pages `alpha` (the one being replaced), `beta` (its replacement) and a
/// bystander `notes` holding the mentions.
fn world() -> World {
    let (mut w, hlc) = ws();
    let a = page(&mut w, &hlc, "alpha", "Alpha Project");
    let b = page(&mut w, &hlc, "beta", "Beta Project");
    let notes = page(&mut w, &hlc, "notes", "Notes");
    let (from, to) = (names_of(&w, a), names_of(&w, b));
    World {
        w,
        hlc,
        from,
        to,
        notes,
    }
}

impl World {
    fn run(&mut self) -> LinkRewriteReport {
        rewrite_mentions(&mut self.w, &self.hlc, &self.from, &self.to).unwrap()
    }
}

#[test]
fn a_name_that_slug_resolves_to_another_page_is_not_a_mention() {
    let (mut w, hlc) = ws();
    let a = page(&mut w, &hlc, "alpha", "Shared Name");
    page(&mut w, &hlc, "shared-name", "Something Else");
    let b = page(&mut w, &hlc, "beta", "Beta Project");
    let notes = page(&mut w, &hlc, "notes", "Notes");
    // Navigation resolves this by slugify to `shared-name`, never reaching
    // the case-insensitive title rung where `alpha` would have matched.
    let elsewhere = block(&mut w, &hlc, notes, "[[Shared Name]]");
    let mine = block(&mut w, &hlc, notes, "[[alpha]]");
    let (from, to) = (names_of(&w, a), names_of(&w, b));

    let report = rewrite_mentions(&mut w, &hlc, &from, &to).unwrap();

    assert_eq!(text_of(&w, elsewhere), "[[Shared Name]]");
    assert_eq!(text_of(&w, mine), "[[Beta Project]]");
    assert_eq!(report.rewritten_blocks, vec![mine.to_string()]);
}

#[test]
fn a_ref_by_title_still_rewrites_when_nothing_else_claims_the_name() {
    let mut s = world();
    let b = block(&mut s.w, &s.hlc, s.notes, "[[alpha project]]");
    let report = s.run();
    assert_eq!(text_of(&s.w, b), "[[Beta Project]]");
    assert_eq!(report.rewritten_blocks, vec![b.to_string()]);
}

#[test]
fn rewrites_every_form_across_the_workspace() {
    let mut s = world();
    let r = block(&mut s.w, &s.hlc, s.notes, "[[alpha]]");
    let t = block(&mut s.w, &s.hlc, s.notes, "do it #alpha.");
    let nested = block(&mut s.w, &s.hlc, r, "**[[Alpha Project]]**");
    s.run();
    assert_eq!(text_of(&s.w, r), "[[Beta Project]]");
    assert_eq!(text_of(&s.w, t), "do it #beta.");
    assert_eq!(text_of(&s.w, nested), "**[[Beta Project]]**");
}

#[test]
fn a_person_page_is_rewritten_through_the_page_names() {
    let (mut w, hlc) = ws();
    let a = page(&mut w, &hlc, "ana", "Ana Souza");
    let b = page(&mut w, &hlc, "ana-lima", "Ana Lima");
    for id in [a, b] {
        let kind = Some(PropValue::Text("person".into()));
        set_property(&mut w, &hlc, id, "type", kind).unwrap();
    }
    let notes = page(&mut w, &hlc, "notes", "Notes");
    let m = block(&mut w, &hlc, notes, "ask [[@Ana Souza]] / [[ana]]");
    let (from, to) = (names_of(&w, a), names_of(&w, b));
    assert!(from.person && to.person);

    rewrite_mentions(&mut w, &hlc, &from, &to).unwrap();

    assert_eq!(text_of(&w, m), "ask [[@Ana Lima]] / [[Ana Lima]]");
}

#[test]
fn code_and_fences_in_the_workspace_are_untouched() {
    let mut s = world();
    let text = "`[[alpha]]`\n```\n[[alpha]]\n#alpha\n```";
    let b = block(&mut s.w, &s.hlc, s.notes, text);
    let ops = s.w.log().len();
    let report = s.run();
    assert_eq!(text_of(&s.w, b), text);
    assert!(report.rewritten_blocks.is_empty());
    assert_eq!(s.w.log().len(), ops);
}

#[test]
fn a_namespace_child_is_skipped_and_reported() {
    let mut s = world();
    let b = block(
        &mut s.w,
        &s.hlc,
        s.notes,
        "[[alpha/child]] #alpha/child/deep and [[alpha]]",
    );
    let other = block(&mut s.w, &s.hlc, s.notes, "[[alpha/child]] [[alpha/child]]");
    let report = s.run();

    assert_eq!(
        text_of(&s.w, b),
        "[[alpha/child]] #alpha/child/deep and [[Beta Project]]"
    );
    assert_eq!(text_of(&s.w, other), "[[alpha/child]] [[alpha/child]]");
    let skipped: Vec<(String, String)> = report
        .skipped_namespace_refs
        .iter()
        .map(|s| (s.block_id.clone(), s.name.clone()))
        .collect();
    assert_eq!(
        skipped,
        vec![
            (b.to_string(), "alpha/child".to_string()),
            (b.to_string(), "alpha/child/deep".to_string()),
            (other.to_string(), "alpha/child".to_string()),
        ]
    );
}

#[test]
fn a_second_run_rewrites_nothing() {
    let mut s = world();
    block(&mut s.w, &s.hlc, s.notes, "[[alpha]] #alpha **[[Alpha Project]]**");
    block(&mut s.w, &s.hlc, s.notes, "[[alpha/child]]");
    let first = s.run();
    assert_eq!(first.rewritten_blocks.len(), 1);

    let ops = s.w.log().len();
    let second = s.run();
    assert!(second.rewritten_blocks.is_empty());
    assert!(second.touched_pages.is_empty());
    assert_eq!(s.w.log().len(), ops, "nothing left to edit");
}

#[test]
fn a_case_only_rename_is_stable_on_a_second_run() {
    let (mut w, hlc) = ws();
    let a = page(&mut w, &hlc, "alpha", "Alpha Project");
    let notes = page(&mut w, &hlc, "notes", "Notes");
    let m = block(&mut w, &hlc, notes, "[[Alpha Project]]");
    let from = names_of(&w, a);
    let to = names("alpha", "ALPHA PROJECT");

    rewrite_mentions(&mut w, &hlc, &from, &to).unwrap();
    assert_eq!(text_of(&w, m), "[[ALPHA PROJECT]]");
    let ops = w.log().len();
    rewrite_mentions(&mut w, &hlc, &from, &to).unwrap();
    assert_eq!(w.log().len(), ops);
}

#[test]
fn identical_names_are_a_no_op() {
    let mut s = world();
    block(&mut s.w, &s.hlc, s.notes, "[[alpha]] #alpha [[alpha/child]]");
    let ops = s.w.log().len();
    s.to = s.from.clone();
    let report = s.run();
    assert!(report.rewritten_blocks.is_empty());
    assert!(report.touched_pages.is_empty());
    assert!(report.skipped_namespace_refs.is_empty());
    assert_eq!(s.w.log().len(), ops);
}

#[test]
fn one_edit_op_per_changed_block() {
    let mut s = world();
    block(&mut s.w, &s.hlc, s.notes, "[[alpha]] twice [[alpha]] and #alpha");
    block(&mut s.w, &s.hlc, s.notes, "untouched [[beta]]");
    block(&mut s.w, &s.hlc, s.notes, "[[alpha]]");
    let before = s.w.log().len();
    let report = s.run();
    assert_eq!(report.rewritten_blocks.len(), 2);
    assert_eq!(s.w.log().len() - before, 2);
}

#[test]
fn blocks_are_edited_in_document_order() {
    let mut s = world();
    let first = block(&mut s.w, &s.hlc, s.notes, "[[alpha]]");
    let second = block(&mut s.w, &s.hlc, s.notes, "[[alpha]]");
    let child = block(&mut s.w, &s.hlc, first, "[[alpha]]");
    let report = s.run();
    assert_eq!(
        report.rewritten_blocks,
        vec![first.to_string(), child.to_string(), second.to_string()]
    );
}

#[test]
fn page_roots_are_never_rewritten() {
    let (mut w, hlc) = ws();
    let a = page(&mut w, &hlc, "alpha", "Alpha Project");
    let b = page(&mut w, &hlc, "beta", "Beta Project");
    // A page whose own title is prose that mentions `alpha`.
    let titled = page(&mut w, &hlc, "see-alpha", "see [[alpha]]");
    let (from, to) = (names_of(&w, a), names_of(&w, b));
    let title_before = page_meta(&w, titled).unwrap().title;
    let text_before = w.block_text(titled);
    let ops = w.log().len();

    let report = rewrite_mentions(&mut w, &hlc, &from, &to).unwrap();

    assert!(report.rewritten_blocks.is_empty());
    assert_eq!(page_meta(&w, titled).unwrap().title, title_before);
    assert_eq!(w.block_text(titled), text_before);
    assert_eq!(w.log().len(), ops);
}

#[test]
fn trashed_blocks_are_never_rewritten() {
    let mut s = world();
    let live = block(&mut s.w, &s.hlc, s.notes, "[[alpha]]");
    let doomed = block(&mut s.w, &s.hlc, s.notes, "[[alpha]]");
    let under_doomed = block(&mut s.w, &s.hlc, doomed, "#alpha");
    delete(&mut s.w, &s.hlc, doomed).unwrap();

    let report = s.run();

    assert_eq!(report.rewritten_blocks, vec![live.to_string()]);
    assert_eq!(text_of(&s.w, doomed), "[[alpha]]");
    assert_eq!(text_of(&s.w, under_doomed), "#alpha");
}

#[test]
fn rewritten_blocks_name_their_pages_once_and_sorted() {
    let (mut w, hlc) = ws();
    let a = page(&mut w, &hlc, "alpha", "Alpha Project");
    let b = page(&mut w, &hlc, "beta", "Beta Project");
    let zed = page(&mut w, &hlc, "zed", "Zed");
    let ann = page(&mut w, &hlc, "ann", "Ann");
    let quiet = page(&mut w, &hlc, "quiet", "Quiet");
    let z1 = block(&mut w, &hlc, zed, "[[alpha]]");
    let z2 = block(&mut w, &hlc, zed, "#alpha");
    let n1 = block(&mut w, &hlc, ann, "x");
    let n2 = block(&mut w, &hlc, n1, "[[alpha]]");
    block(&mut w, &hlc, quiet, "nothing here");
    let (from, to) = (names_of(&w, a), names_of(&w, b));

    let report = rewrite_mentions(&mut w, &hlc, &from, &to).unwrap();

    assert_eq!(report.touched_pages, vec!["ann".to_string(), "zed".to_string()]);
    let mut ids = report.rewritten_blocks.clone();
    ids.sort();
    let mut expected = vec![z1.to_string(), z2.to_string(), n2.to_string()];
    expected.sort();
    assert_eq!(ids, expected);
}

#[test]
fn a_title_only_rename_leaves_tags_alone() {
    let mut s = world();
    s.to = names("alpha", "Alpha Renamed");
    let b = block(&mut s.w, &s.hlc, s.notes, "#alpha and [[alpha project]]");
    s.run();
    assert_eq!(text_of(&s.w, b), "#alpha and [[Alpha Renamed]]");
}
