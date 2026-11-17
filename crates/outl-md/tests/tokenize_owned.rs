//! Tests for the owned, serializable inline token form
//! (`InlineToken` / `tokenize_owned`).
//! Moved out of `src/inline.rs` to keep that module under the
//! file-size-guard. Every test here exercises only the public API.

use outl_md::inline::{tokenize_owned, InlineToken};

#[test]
fn round_trips_every_variant_into_serializable_form() {
    // Block-ref handles need `REF_HANDLE_TAIL_LEN` chars after
    // `blk-` (6 today) to pass `is_valid_block_handle`; shorter
    // handles correctly degrade to plain text.
    let toks = tokenize_owned(
        "**b** *i* ~~s~~ `c` [t](u) ![a](img.png) [[p]] #tag ((blk-aaaaaa)) !((blk-bbbbbb)) :tada: tail",
    );
    // Spot-check shape (kind discriminant) and the `tag` prefixing,
    // since that's the one place `from_borrowed` does more than a
    // string copy.
    let kinds: Vec<&str> = toks
        .iter()
        .map(|t| match t {
            InlineToken::Plain { .. } => "plain",
            InlineToken::Bold { .. } => "bold",
            InlineToken::Italic { .. } => "italic",
            InlineToken::Strike { .. } => "strike",
            InlineToken::Highlight { .. } => "highlight",
            InlineToken::Code { .. } => "code",
            InlineToken::Link { .. } => "link",
            InlineToken::Image { .. } => "image",
            InlineToken::Ref { .. } => "ref",
            InlineToken::Tag { .. } => "tag",
            InlineToken::BlockRef { .. } => "blockref",
            InlineToken::Embed { .. } => "embed",
            InlineToken::PageEmbed { .. } => "pageembed",
            InlineToken::Emoji { .. } => "emoji",
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            "bold", "plain", "italic", "plain", "strike", "plain", "code", "plain", "link",
            "plain", "image", "plain", "ref", "plain", "tag", "plain", "blockref", "plain",
            "embed", "plain", "emoji", "plain",
        ],
    );
    // Tag value carries the leading `#` so the mobile renderer
    // doesn't have to re-prefix.
    let tag = toks
        .iter()
        .find_map(|t| match t {
            InlineToken::Tag { value } => Some(value.clone()),
            _ => None,
        })
        .expect("tokenize_owned should emit one Tag");
    assert_eq!(tag, "#tag");
}

#[test]
fn image_carries_alt_and_href() {
    // The image token ships alt + href so the frontend renders an
    // `<img>` (asset path or remote URL) without re-parsing markdown.
    let toks = tokenize_owned("![cover](assets/ab12.png)");
    let img = toks
        .iter()
        .find_map(|t| match t {
            InlineToken::Image { alt, href } => Some((alt.clone(), href.clone())),
            _ => None,
        })
        .expect("tokenize_owned should emit one Image");
    assert_eq!(img.0, "cover");
    assert_eq!(img.1, "assets/ab12.png");
}

#[test]
fn empty_input_yields_no_tokens() {
    // Replaces coverage from the deleted mobile `markdown.test.ts`
    // — the old TS tokenizer used to push a phantom `plain` run on
    // empty input. Pin the Rust behaviour so a future refactor
    // doesn't reintroduce it.
    assert!(tokenize_owned("").is_empty());
}

#[test]
fn bare_text_is_one_plain_run() {
    let toks = tokenize_owned("tail");
    assert_eq!(toks.len(), 1);
    assert!(matches!(
        &toks[0],
        InlineToken::Plain { value } if value == "tail"
    ));
}

#[test]
fn plain_text_after_last_match_survives() {
    // The deleted TS test "preserves trailing text after the last
    // match" guarded a tokenizer bug where the tail run got
    // dropped. Pin the same invariant on the Rust side.
    let toks = tokenize_owned("**bold** tail");
    let trailing = toks.last().expect("at least one token");
    assert!(
        matches!(trailing, InlineToken::Plain { value } if value == " tail"),
        "expected trailing Plain(\" tail\"), got {trailing:?}",
    );
}

/// The `> ` blockquote marker is block-level, not inline. It must
/// not become a token: the inline tokenizer is called on a body
/// that already had the prefix stripped by
/// `outl_actions::quote::split_quote`. If the marker ever shows up
/// inside the body (the user typed `"> > foo"` — single split, the
/// inner `"> foo"` is the body), it stays Plain so the inline
/// surface doesn't accidentally double-style it.
#[test]
fn quote_prefix_is_not_tokenized_as_an_inline() {
    let toks = tokenize_owned("> still plain");
    assert_eq!(toks.len(), 1);
    assert!(
        matches!(&toks[0], InlineToken::Plain { value } if value == "> still plain"),
        "expected the whole string as Plain, got {toks:?}"
    );
}

#[test]
fn serde_json_kind_field_matches_mobile_dto() {
    // Mobile reads `kind` lowercase via Serde's
    // `rename_all = "lowercase"`. If we ever change the rename
    // policy, the iOS client silently goes to plain — this test
    // pins the wire shape.
    let toks = vec![
        InlineToken::Plain { value: "hi".into() },
        InlineToken::BlockRef {
            value: "blk-x1".into(),
        },
        InlineToken::Image {
            alt: "cover".into(),
            href: "assets/ab12.png".into(),
        },
    ];
    let json = serde_json::to_string(&toks).unwrap();
    assert!(json.contains(r#""kind":"plain""#));
    assert!(json.contains(r#""kind":"blockref""#));
    // The image token's wire shape is what the frontend switches on; pin
    // the discriminant and both field names so a rename can't silently
    // drop images to "renders as nothing" on a client.
    assert!(json.contains(r#""kind":"image""#));
    assert!(json.contains(r#""alt":"cover""#));
    assert!(json.contains(r#""href":"assets/ab12.png""#));
}
