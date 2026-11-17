//! Inline tokenization and cursor introspection — agnostic of any UI.
//!
//! This module exists so the TUI, a future Tauri/desktop GUI, and the
//! mobile (uniffi-bridged) clients can all share the same understanding
//! of what's inside a block:
//!
//! - **TUI** maps each [`InlineTok`] to a `ratatui::Span` with style.
//! - **Tauri / web** maps tokens to HTML / React fragments.
//! - **iOS / Android** maps tokens to `AttributedString` /
//!   `AnnotatedString`.
//!
//! The recognized constructs:
//!
//! - `[[name]]` — outl page reference (lives in `pages/{slugify(name)}.md`).
//! - `[[YYYY-MM-DD]]` — journal date reference.
//! - `#tag` — tag (resolves to a page when opened).
//! - `**bold**` / `*italic*` / `_italic_` / `~~strike~~` / `==highlight==` / `` `code` ``.
//! - `[text](url)` — standard markdown link.
//! - Anything else: [`InlineTok::Plain`].
//!
//! Multi-byte UTF-8 (accents, emoji, CJK) is handled correctly — we
//! always advance by `ch.len_utf8()`, never by raw byte.
//!
//! What lives *here* is the scan itself: [`tokenize`] (plus its owned
//! twin [`tokenize_owned`]), the `match_one` precedence table, and
//! [`inline_to_source`], the inverse that re-emits tokens as the
//! markdown they came from. Sibling modules own one slice each:
//! `token` (the two token vocabularies), `emphasis` (`**`/`__`/`*`/`_`/
//! `~~`/`==`/`` ` `` delimiter pairs), `reference` (`[[name]]`,
//! `((blk-…))`, `!((blk-…))`, `#tag`), `link` (`[text](url)`,
//! `![alt](url)`), `shortcode` (`:emoji:`), `plain` (flattening tokens
//! to prose) and `cursor` (what sits under the caret). Their public
//! items are re-exported here, so every `outl_md::inline::*` path stays
//! stable.

use crate::emphasis::{
    try_bold, try_bold_under, try_code, try_highlight, try_italic_star, try_italic_under,
    try_strike,
};
use crate::link::try_image;
use crate::reference::{try_block_ref, try_embed, try_page_embed, try_page_ref, try_tag};
use crate::shortcode::try_emoji;

pub use crate::plain::plain_text;
pub use crate::reference::is_valid_block_handle;
pub use crate::token::{InlineTok, InlineToken, RefTarget};

// Cursor introspection (`ref_at_cursor`, `link_at_cursor`,
// `byte_index_for_char`) lives in the sibling [`crate::cursor`] module and
// is re-exported here so `outl_md::inline::{…}` paths keep resolving.
pub use crate::cursor::{byte_index_for_char, link_at_cursor, ref_at_cursor};

// `try_md_link` is the canonical "what is a link" matcher; `cursor`
// reuses it through this path.
pub(crate) use crate::link::try_md_link;

/// Tokenize `text` directly into the owned, serializable form. This
/// is the call backend DTOs use when they need to ship tokens to a
/// frontend — single source of truth for inline markdown parsing
/// across every client, no parallel TS / Swift / Kotlin tokenizer
/// to keep in sync.
pub fn tokenize_owned(text: &str) -> Vec<InlineToken> {
    tokenize(text)
        .iter()
        .map(InlineToken::from_borrowed)
        .collect()
}

/// Tokenize inline block content.
///
/// Greedy left-to-right scan. Plain text accumulates between recognized
/// constructs and emerges as a single [`InlineTok::Plain`] run.
pub fn tokenize(text: &str) -> Vec<InlineTok<'_>> {
    let mut out = Vec::new();
    let mut plain_start = 0usize;
    let mut idx = 0usize;

    while idx < text.len() {
        // The char just before `idx` decides whether a `_` here can open
        // emphasis (CommonMark forbids intra-word `_`). `None` at the
        // start of the run counts as "not alphanumeric" → can open.
        let prev = text[..idx].chars().next_back();
        if let Some((tok, consumed)) = match_one(&text[idx..], prev) {
            if idx > plain_start {
                out.push(InlineTok::Plain(&text[plain_start..idx]));
            }
            out.push(tok);
            idx += consumed;
            plain_start = idx;
        } else {
            let ch = text[idx..]
                .chars()
                .next()
                .expect("idx < text.len() implies a next char");
            idx += ch.len_utf8();
        }
    }
    if plain_start < text.len() {
        out.push(InlineTok::Plain(&text[plain_start..]));
    }
    out
}

// --- matcher precedence --------------------------------------------------
//
// One attempt per construct, in the order below. The matchers
// themselves live in the sibling modules (`reference`, `link`,
// `emphasis`, `shortcode`); what lives here is the *order*, which is
// the part that cannot be reasoned about one matcher at a time.

fn match_one(s: &str, prev: Option<char>) -> Option<(InlineTok<'_>, usize)> {
    if let Some(out) = try_page_ref(s) {
        return Some(out);
    }
    // `try_page_embed` claims `![[name]]`. It sits here, ahead of the
    // other two `!` openers, because it is the most specific of the
    // three: `![[` cannot be a block embed (`!((`) and `try_image`
    // already refuses it (`![` with no `](`). Ahead of `try_md_link`
    // for the same reason `try_image` is — otherwise the `!` is
    // stranded and the bare `[[name]]` behind it reads as a plain ref.
    if let Some(out) = try_page_embed(s) {
        return Some(out);
    }
    // `try_embed` MUST be checked before `try_block_ref`: the embed
    // form starts with `!` and contains a `((handle))` inside, and we
    // want the whole `!((handle))` consumed as one token instead of
    // a stray `Plain("!")` followed by a `BlockRef`.
    if let Some(out) = try_embed(s) {
        return Some(out);
    }
    // `try_image` shares the `!` opener with `try_embed` (`![` vs `!((`),
    // so it sits next to it and, crucially, before `try_md_link` — the
    // bare `[` link — so `![alt](url)` isn't split into `!` + a link.
    if let Some(out) = try_image(s) {
        return Some(out);
    }
    if let Some(out) = try_block_ref(s) {
        return Some(out);
    }
    if let Some(out) = try_bold(s) {
        return Some(out);
    }
    if let Some(out) = try_bold_under(s, prev) {
        return Some(out);
    }
    if let Some(out) = try_strike(s) {
        return Some(out);
    }
    if let Some(out) = try_highlight(s) {
        return Some(out);
    }
    if let Some(out) = try_italic_star(s) {
        return Some(out);
    }
    if let Some(out) = try_italic_under(s, prev) {
        return Some(out);
    }
    if let Some(out) = try_code(s) {
        return Some(out);
    }
    if let Some(out) = try_md_link(s) {
        return Some(out);
    }
    // `try_emoji` sits between `try_md_link` and `try_tag` — `:` does
    // not overlap with any other matcher's opener, so the slot is
    // chosen for readability, not precedence.
    if let Some(out) = try_emoji(s) {
        return Some(out);
    }
    if let Some(out) = try_tag(s) {
        return Some(out);
    }
    None
}

/// Re-emit a tokenized inline span back as the markdown source it
/// came from.
///
/// Bold / italic / strike now carry recursively-tokenized inners
/// (`Vec<InlineTok>`), so consumers that used to call
/// `inner.to_string()` on a `&str` need a small helper to reconstruct
/// the literal source. Renderers that already iterate
/// `Vec<InlineTok>` to dispatch per-variant styling don't need this —
/// it's specifically for surfaces that want the whole inner span as
/// one styled string.
pub fn inline_to_source(toks: &[InlineTok<'_>]) -> String {
    let mut out = String::new();
    for tok in toks {
        match tok {
            InlineTok::Plain(s) => out.push_str(s),
            InlineTok::PageRef { name } => {
                out.push_str("[[");
                out.push_str(name);
                out.push_str("]]");
            }
            InlineTok::Tag { name } => {
                out.push('#');
                out.push_str(name);
            }
            InlineTok::Bold { inner } => {
                out.push_str("**");
                out.push_str(&inline_to_source(inner));
                out.push_str("**");
            }
            InlineTok::Italic { inner, marker } => {
                out.push(*marker);
                out.push_str(&inline_to_source(inner));
                out.push(*marker);
            }
            InlineTok::Strike { inner } => {
                out.push_str("~~");
                out.push_str(&inline_to_source(inner));
                out.push_str("~~");
            }
            InlineTok::Highlight { inner } => {
                out.push_str("==");
                out.push_str(&inline_to_source(inner));
                out.push_str("==");
            }
            InlineTok::Code { inner } => {
                out.push('`');
                out.push_str(inner);
                out.push('`');
            }
            InlineTok::Link { text, url } => {
                out.push('[');
                out.push_str(text);
                out.push_str("](");
                out.push_str(url);
                out.push(')');
            }
            InlineTok::Image { alt, url } => {
                out.push_str("![");
                out.push_str(alt);
                out.push_str("](");
                out.push_str(url);
                out.push(')');
            }
            InlineTok::BlockRef { handle } => {
                out.push_str("((");
                out.push_str(handle);
                out.push_str("))");
            }
            InlineTok::Embed { handle } => {
                out.push_str("!((");
                out.push_str(handle);
                out.push_str("))");
            }
            InlineTok::PageEmbed { name } => {
                out.push_str("![[");
                out.push_str(name);
                out.push_str("]]");
            }
            InlineTok::Emoji { shortcode } => {
                out.push(':');
                out.push_str(shortcode);
                out.push(':');
            }
        }
    }
    out
}

// Tests for the owned `InlineToken` / `tokenize_owned` wire form live
// in `tests/tokenize_owned.rs` (moved out to keep this module under the
// file-size-guard).
