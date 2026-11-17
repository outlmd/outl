//! `![[page]]` page embed — the page-level twin of `!((blk-XXXXXX))`
//! (issue #190). Its own file, like `highlight.rs`, because a new token
//! has three things that go wrong independently: what the matcher
//! claims, whether the markdown survives a round trip (invariant 2),
//! and whether the one spelling it deliberately does *not* claim stays
//! unclaimed.
//!
//! That last group is the reason this file is worth reading before
//! changing the matcher. `![[photo.png]]` is refused on purpose:
//! `wikilink::convert_image_links` already owns it on the way in from a
//! vault, and one syntax with two owners resolves to whichever runs
//! second, silently. The refusal is what keeps this matcher claiming
//! only input that previously meant nothing.

use outl_md::inline::{inline_to_source, ref_at_cursor, tokenize, InlineTok, RefTarget};

#[test]
fn page_embed_is_recognized() {
    let toks = tokenize("context: ![[projects]] below");
    assert!(toks.contains(&InlineTok::PageEmbed { name: "projects" }));
}

#[test]
fn page_embed_consumes_the_bang_rather_than_stranding_it() {
    // The failure this guards against: the `!` falls through as plain
    // text and the `[[projects]]` behind it reads as an ordinary ref,
    // so the block renders a link where the user asked for an embed.
    let toks = tokenize("![[projects]]");
    assert_eq!(toks, vec![InlineTok::PageEmbed { name: "projects" }]);
    assert!(!toks.iter().any(|t| matches!(t, InlineTok::PageRef { .. })));
}

#[test]
fn page_embed_round_trips_through_inline_to_source() {
    // Invariant 2: what lands on disk is the literal `![[name]]`, and
    // re-tokenizing it yields the same token. A renderer that dropped
    // the `!` would silently demote every embed to a link on the next
    // save.
    let src = "see ![[weekly review]] and [[weekly review]]";
    let toks = tokenize(src);
    assert_eq!(inline_to_source(&toks), src);
    assert_eq!(tokenize(&inline_to_source(&toks)), toks);
}

#[test]
fn page_embed_and_page_ref_are_different_tokens_for_one_name() {
    let toks = tokenize("![[notes]] vs [[notes]]");
    assert!(toks.contains(&InlineTok::PageEmbed { name: "notes" }));
    assert!(toks.contains(&InlineTok::PageRef { name: "notes" }));
}

#[test]
fn page_embed_refuses_an_image_target() {
    // `![[photo.png]]` is Obsidian's image embed and
    // `wikilink::convert_image_links` already owns that spelling on the
    // way in from a vault. Claiming it here would give one syntax two
    // owners, so the matcher declines and the shape stays exactly what
    // it tokenizes to today.
    for img in ["![[photo.png]]", "![[a/b/cover.jpeg]]", "![[diagram.svg]]"] {
        let toks = tokenize(img);
        assert!(
            !toks
                .iter()
                .any(|t| matches!(t, InlineTok::PageEmbed { .. })),
            "{img} should not be a page embed"
        );
        assert!(
            toks.iter().any(|t| matches!(t, InlineTok::PageRef { .. })),
            "{img} should still tokenize as it did before"
        );
    }
}

#[test]
fn malformed_page_embed_falls_back_to_plain() {
    for bad in ["![[]]", "![[unclosed", "![["] {
        let toks = tokenize(bad);
        assert!(
            !toks
                .iter()
                .any(|t| matches!(t, InlineTok::PageEmbed { .. })),
            "{bad} should not be a page embed"
        );
    }
}

#[test]
fn page_embed_is_recognized_inside_emphasis() {
    // Emphasis re-tokenizes its inner span, so an embed nested in
    // `**…**` must survive. A flat scan drops it.
    let toks = tokenize("**![[projects]]**");
    let InlineTok::Bold { inner } = &toks[0] else {
        panic!("expected bold, got {toks:?}");
    };
    assert_eq!(inner, &vec![InlineTok::PageEmbed { name: "projects" }]);
}

#[test]
fn page_embed_is_not_confused_with_a_markdown_image() {
    let toks = tokenize("![[projects]]");
    assert!(!toks.iter().any(|t| matches!(t, InlineTok::Image { .. })));
    assert!(!toks.iter().any(|t| matches!(t, InlineTok::Link { .. })));
}

#[test]
fn page_embed_resolves_under_the_cursor_including_on_the_bang() {
    // A page embed is a page *reference*, so "go to target" is the
    // right verb wherever the caret sits in the token — the `!`
    // included, the same courtesy `!((blk-…))` already gets.
    let text = "![[projects]]";
    for col in 0..text.chars().count() {
        assert_eq!(
            ref_at_cursor(text, col),
            Some(RefTarget::Page("projects".into())),
            "caret at {col} should resolve"
        );
    }
}

#[test]
fn page_embed_of_a_date_resolves_as_a_journal() {
    assert_eq!(
        ref_at_cursor("![[2026-11-17]]", 0),
        Some(RefTarget::Journal(
            chrono::NaiveDate::from_ymd_opt(2026, 11, 17).expect("valid date")
        )),
    );
}

#[test]
fn page_embed_flattens_to_the_page_name_in_plain_text() {
    // Notification bodies and a11y labels read the prose, not the
    // syntax — same rule `[[page]]` follows.
    assert_eq!(
        outl_md::plain_text("review ![[weekly review]] today"),
        "review weekly review today"
    );
}

#[test]
fn a_namespaced_page_embed_keeps_its_slashes() {
    // The first shape this hit in a real workspace:
    // `![[buser/team/people/beneficios]]`. A page namespace lives in
    // the `title::`, and `slugify` folds `/` to `-` for the filename,
    // so the token has to carry the name **verbatim** — folding it here
    // would hand the resolver a slug and lose the title rung of the
    // ladder.
    let name = "buser/team/people/beneficios";
    let src = format!("![[{name}]]");
    assert_eq!(tokenize(&src), vec![InlineTok::PageEmbed { name }]);
    assert_eq!(inline_to_source(&tokenize(&src)), src);
    // And the fold is what the resolver's second rung looks up.
    assert_eq!(outl_md::slug::slugify(name), "buser-team-people-beneficios");
}
