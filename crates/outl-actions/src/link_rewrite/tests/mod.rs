//! The link rewriter's battery: every spelling it rewrites and every one
//! it must leave alone, as a pure text transform.
//!
//! The shape of what reaches the op log is [`workspace`]'s half. The
//! helpers below are shared with it, which is why they live here rather
//! than in either leaf.

use super::*;
use crate::block::{append_block, delete};
use crate::page::{open_or_create, page_meta, set_property, PageKind};
use outl_core::id::ActorId;
use outl_core::property::PropValue;

mod workspace;

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

// --- pure text: page embeds (`![[name]]`, issue #190) ---------------------

#[test]
fn a_page_embed_is_rewritten_and_keeps_its_bang() {
    assert_eq!(
        rewrite("see ![[alpha]] now").as_deref(),
        Some("see ![[Beta Project]] now")
    );
}

/// Every rung of the ladder `[[name]]` climbs, climbed by `![[name]]`:
/// the literal slug, the title ignoring case, and a name whose slugified
/// form is the page's slug.
#[test]
fn a_page_embed_is_rewritten_by_every_rung_of_the_ladder() {
    assert_eq!(rewrite("![[alpha]]").as_deref(), Some("![[Beta Project]]"));
    assert_eq!(
        rewrite("![[ALPHA project]]").as_deref(),
        Some("![[Beta Project]]")
    );
    let from = names("alpha-project", "Alpha Project");
    assert_eq!(
        rewrite_text("![[alpha project]]", &from, &beta()).as_deref(),
        Some("![[Beta Project]]")
    );
}

/// The embed's opener is three bytes (`![[`), not two. A splice computed
/// with the ref's width would start one byte late and eat the `!`, or
/// start one early and eat whatever the user wrote in front of it. Both
/// shapes are here because they fail in opposite directions.
#[test]
fn a_page_embed_splice_does_not_eat_the_byte_in_front_of_it() {
    assert_eq!(
        rewrite("x![[alpha]]y").as_deref(),
        Some("x![[Beta Project]]y")
    );
    assert_eq!(
        rewrite("(![[alpha]])").as_deref(),
        Some("(![[Beta Project]])")
    );
}

/// Multi-byte characters either side, and a second mention after the
/// first: a wrong length on the embed's splice corrupts the next one.
#[test]
fn page_embeds_and_refs_mix_in_one_block_with_offsets_intact() {
    assert_eq!(
        rewrite("é![[alpha]]ü [[alpha]] ü ![[Alpha Project]] #alpha").as_deref(),
        Some("é![[Beta Project]]ü [[Beta Project]] ü ![[Beta Project]] #beta")
    );
}

/// `![[name]]` is a page reference, so a rename that only changes the
/// title still respells it — same rule as `[[name]]`.
#[test]
fn a_title_only_rename_respells_a_page_embed() {
    let from = names("alpha", "Alpha Project");
    let to = names("alpha", "Alpha Renamed");
    assert_eq!(
        rewrite_text("![[alpha]]", &from, &to).as_deref(),
        Some("![[Alpha Renamed]]")
    );
}

#[test]
fn a_page_embed_inside_emphasis_is_rewritten() {
    assert_eq!(
        rewrite("**![[alpha]]** and ==![[alpha]]==").as_deref(),
        Some("**![[Beta Project]]** and ==![[Beta Project]]==")
    );
}

#[test]
fn a_page_embed_in_a_code_span_is_untouched() {
    assert_eq!(rewrite("`![[alpha]]`"), None);
}

/// A block embed names a block, not a page, so it stays — and it must
/// not be confused with the page embed it is one character away from.
#[test]
fn a_block_embed_is_still_left_alone() {
    assert_eq!(
        rewrite("!((blk-abc123)) and ![[alpha]]").as_deref(),
        Some("!((blk-abc123)) and ![[Beta Project]]")
    );
}

/// `![[photo.png]]` is the image-embed spelling, owned by
/// `wikilink::convert_image_links`, so the tokenizer refuses it as a page
/// embed: it reads as `Plain("!")` + `PageRef`. The ref half is still a
/// mention and is still rewritten; the `!` is not ours to move.
#[test]
fn an_image_shaped_embed_is_rewritten_as_the_plain_ref_it_tokenizes_as() {
    let from = names("photo.png", "photo.png");
    let to = names("beta", "Beta Project");
    assert_eq!(
        rewrite_text("![[photo.png]]", &from, &to).as_deref(),
        Some("![[Beta Project]]")
    );
}

/// The replacement is round-tripped through the tokenizer in the form it
/// will be written. A title the embed form cannot carry (an image target)
/// falls through to the slug, where `.` folds to `-` and `![[photo-png]]`
/// reads back as the embed it is. Writing the title would have produced
/// `![[photo.png]]`, which tokenizes as a `!` in front of a link —
/// silently turning the user's transclusion into something else.
#[test]
fn a_title_the_embed_form_cannot_carry_falls_back_to_the_slug() {
    let to = names("photo-png", "photo.png");
    assert_eq!(
        rewrite_text("![[alpha]]", &alpha(), &to).as_deref(),
        Some("![[photo-png]]")
    );
    // The plain ref has no such constraint, so it keeps the title.
    assert_eq!(
        rewrite_text("[[alpha]]", &alpha(), &to).as_deref(),
        Some("[[photo.png]]")
    );
}

/// A namespace child is a different page in either spelling.
#[test]
fn a_namespaced_page_embed_is_skipped_like_a_namespaced_ref() {
    let scan = scan("![[alpha/child]]", &alpha(), &beta(), &|_| false);
    assert_eq!(scan.rewritten, None);
    assert_eq!(scan.skipped, vec!["alpha/child".to_string()]);
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
    assert_eq!(
        rewrite("[[alphabet]] and [[alpha beta]] and [[the alpha]]"),
        None
    );
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
