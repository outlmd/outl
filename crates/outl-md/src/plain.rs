//! Flattening inline markup down to the prose a human reads.
//!
//! The counterpart to `inline::inline_to_source`: that one reconstructs
//! the markdown verbatim, this one throws the syntax away and keeps the
//! words. Built for notification bodies, and equally right for a11y
//! labels, plain-text export, or a search snippet.
//!
//! [`plain_text`] is re-exported by `crate::inline` and at the crate
//! root, so both `outl_md::plain_text` and
//! `outl_md::inline::plain_text` keep resolving.

use crate::inline::tokenize;
use crate::token::InlineTok;

/// Flatten inline markup into the plain text a human reads.
///
/// The single owner of "block text without the syntax". Built for
/// notification bodies — a lock screen showing
/// `ship it [[2026-12-12]] #fup` with the brackets and hash intact
/// reads like a bug — and equally right for any surface that needs the
/// prose without the markers (a11y labels, plain-text export, a search
/// snippet).
///
/// Rules, chosen so the result reads as the author meant it:
///
/// - `[[page]]` and `#tag` keep their name, drop their punctuation.
///   `[[@joão]]` becomes `@joão`: the `@` is part of how the person is
///   written, the brackets are not.
/// - Emphasis (`**bold**`, `_italic_`, `~~strike~~`, `==highlight==`)
///   keeps its contents and drops the markers, recursively.
/// - `[text](url)` keeps the anchor text; the URL is noise out loud.
///   `![alt](url)` keeps the alt for the same reason, and yields
///   nothing when the alt is empty.
/// - `:shortcode:` becomes its glyph, since that's what it renders as.
/// - `((blk-…))` / `!((blk-…))` resolve to nothing here. The handle is
///   an id, not prose, and this function has no index to look it up
///   in — a caller that wants the source text resolves the embed first.
pub fn plain_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    push_plain(&tokenize(raw), &mut out);
    // Collapsing runs keeps a dropped block-ref from leaving a double
    // space behind ("see ((blk-x)) later" -> "see later").
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn push_plain(tokens: &[InlineTok<'_>], out: &mut String) {
    for tok in tokens {
        match tok {
            InlineTok::Plain(s) => out.push_str(s),
            InlineTok::PageRef { name } | InlineTok::PageEmbed { name } => out.push_str(name),
            InlineTok::Tag { name } => out.push_str(name),
            InlineTok::Code { inner } => out.push_str(inner),
            InlineTok::Link { text, .. } => out.push_str(text),
            InlineTok::Image { alt, .. } => out.push_str(alt),
            InlineTok::Bold { inner }
            | InlineTok::Italic { inner, .. }
            | InlineTok::Strike { inner }
            | InlineTok::Highlight { inner } => push_plain(inner, out),
            InlineTok::Emoji { shortcode } => {
                match crate::emoji::shortcode_to_unicode(shortcode) {
                    Some(glyph) => out.push_str(glyph),
                    // Unreachable in practice (the matcher only emits
                    // a token the catalog knows), but falling back to
                    // the shortcode beats dropping the word.
                    None => out.push_str(shortcode),
                }
            }
            // An id, not prose, and there's no index in scope to
            // resolve it against.
            InlineTok::BlockRef { .. } | InlineTok::Embed { .. } => {}
        }
    }
}

#[cfg(test)]
mod plain_text_tests {
    use super::plain_text;

    #[test]
    fn refs_and_tags_keep_the_name_and_lose_the_punctuation() {
        assert_eq!(
            plain_text("ship it [[2026-12-12]] #fup"),
            "ship it 2026-12-12 fup"
        );
    }

    #[test]
    fn a_mention_keeps_its_at_sign() {
        // The `@` is how the person is written; the brackets are not.
        assert_eq!(plain_text("ping [[@joão]] today"), "ping @joão today");
    }

    #[test]
    fn emphasis_keeps_the_words_and_drops_the_markers() {
        assert_eq!(
            plain_text("**call** the _bank_ ~~today~~ ==now=="),
            "call the bank today now"
        );
    }

    #[test]
    fn nested_emphasis_still_flattens() {
        assert_eq!(plain_text("**see [[avelino]] now**"), "see avelino now");
    }

    #[test]
    fn a_link_keeps_its_anchor_not_its_url() {
        assert_eq!(
            plain_text("read [the RFC](https://example.com/very/long)"),
            "read the RFC"
        );
    }

    #[test]
    fn block_refs_drop_out_without_leaving_a_gap() {
        assert_eq!(plain_text("see ((blk-r6s4a1)) later"), "see later");
        assert_eq!(plain_text("see !((blk-r6s4a1)) later"), "see later");
    }

    #[test]
    fn code_keeps_its_contents() {
        assert_eq!(plain_text("run `cargo test` now"), "run cargo test now");
    }

    #[test]
    fn plain_prose_is_unchanged() {
        assert_eq!(plain_text("call the bank"), "call the bank");
    }

    #[test]
    fn an_empty_body_stays_empty() {
        assert_eq!(plain_text(""), "");
    }
}
