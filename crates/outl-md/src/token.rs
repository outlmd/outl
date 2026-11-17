//! The inline token vocabulary — what a block's text can contain, in
//! both the borrowed and the owned form.
//!
//! Types only: no scanning happens here. The matchers that produce
//! these tokens live in [`crate::emphasis`], [`crate::reference`],
//! [`crate::link`] and [`crate::shortcode`]; the scanner that drives
//! them is [`crate::inline::tokenize`]. Everything public here is
//! re-exported by [`crate::inline`], so `outl_md::inline::InlineTok`
//! and friends keep resolving.
//!
//! The two forms are deliberate mirrors of each other:
//!
//! - [`InlineTok`] borrows from the source string — zero-copy, for use
//!   inside Rust where the source outlives the tokens.
//! - [`InlineToken`] owns its strings and is Serde-friendly — for
//!   anything crossing a serialization boundary (a Tauri command's
//!   return value, a `BlockNode` DTO, a `Backlink.block_tokens`
//!   payload).
//!
//! Keeping them side by side is the point: **adding a variant to
//! [`InlineTok`] requires adding the same variant to [`InlineToken`]
//! plus its arm in [`InlineToken::from_borrowed`], in the same
//! change.** Otherwise the new variant silently degrades to `Plain` on
//! the wire.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// A token recognized in inline block content.
///
/// Lifetimes reference the source string; clone with `to_owned()` if
/// the consumer needs to outlive the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineTok<'a> {
    /// Bare text with no formatting.
    Plain(&'a str),
    /// `[[name]]` — outl page reference.
    PageRef {
        /// Display name (preserved verbatim; the filename is the
        /// slugified form).
        name: &'a str,
    },
    /// `#tag`.
    Tag {
        /// Tag identifier without the leading `#`.
        name: &'a str,
    },
    /// `**bold**`. The inner span is re-tokenized so refs / tags /
    /// block-refs nested inside the markers render with their own
    /// styling instead of falling through as plain text. Same for
    /// `Italic` and `Strike` below.
    Bold {
        /// Recursively-tokenized contents between the markers.
        inner: Vec<InlineTok<'a>>,
    },
    /// `*italic*` or `_italic_`. `marker` is the literal delimiter used.
    Italic {
        /// Recursively-tokenized contents between the markers.
        inner: Vec<InlineTok<'a>>,
        /// Either `'*'` or `'_'`.
        marker: char,
    },
    /// `~~strike~~`.
    Strike {
        /// Recursively-tokenized contents between the markers.
        inner: Vec<InlineTok<'a>>,
    },
    /// `==highlight==` — the target of Roam's `^^highlight^^` on import.
    Highlight {
        /// Recursively-tokenized contents between the markers.
        inner: Vec<InlineTok<'a>>,
    },
    /// `` `code` ``.
    Code {
        /// Inner text between the backticks.
        inner: &'a str,
    },
    /// `[text](url)` — standard markdown link.
    Link {
        /// Anchor text shown to the user.
        text: &'a str,
        /// URL target.
        url: &'a str,
    },
    /// `![alt](url)` — markdown image / embedded asset.
    ///
    /// Mirrors [`InlineTok::Link`] with a leading `!`. `url` is either a
    /// workspace-relative `assets/<hash>.<ext>` path (imported / uploaded
    /// files, resolved against the workspace root by the client) or a
    /// remote `http(s)` URL. `alt` may be empty (`![](url)` is valid).
    /// The client decides how to render by inspecting `url` — inline
    /// `<img>` for image extensions, a viewer / chip for other files.
    Image {
        /// Alt text (may be empty).
        alt: &'a str,
        /// Image / asset target — relative `assets/…` path or remote URL.
        url: &'a str,
    },
    /// `((blk-XXXXXX))` — inline reference to another block.
    ///
    /// The `handle` is the short, stable id persisted in the sidecar
    /// (see [`crate::sidecar::derive_ref_handle`]). The token carries
    /// the full handle including the `blk-` prefix so UI consumers can
    /// trust it as the lookup key without re-parsing.
    BlockRef {
        /// Full handle, e.g. `"blk-r6s4a1"`.
        handle: &'a str,
    },
    /// `!((blk-XXXXXX))` — embed: render the referenced block expanded
    /// (its `text` plus subtree) inline instead of as a link.
    ///
    /// Mirrors markdown image syntax (`![alt](url)`) where `!` means
    /// "expand". UI consumers render an Embed by resolving `handle`
    /// through [`crate::index::WorkspaceIndex::resolve_block_ref`]
    /// and drawing the result's `text` + `children`.
    Embed {
        /// Full handle, e.g. `"blk-r6s4a1"`.
        handle: &'a str,
    },
    /// `![[page]]` — page embed: render the target page's outline
    /// expanded inline instead of as a link.
    ///
    /// The page-level twin of [`InlineTok::Embed`], and shaped from the
    /// same two halves: `!` means "expand", `[[name]]` names a page.
    /// It is also a page *reference* — it produces a backlink and it
    /// gets respelled by a rename, exactly like
    /// [`InlineTok::PageRef`] — so a consumer that treats it as inert
    /// drops a real edge (see `outl_actions::mentions::collect`).
    ///
    /// UI consumers resolve `name` to a page and draw its title plus
    /// its top-level blocks, read-only, capped at the same depth the
    /// block embed is capped at.
    PageEmbed {
        /// Page name, display form, kept verbatim (the file is
        /// `pages/<slugify(name)>.md`).
        name: &'a str,
    },
    /// `:shortcode:` — GitHub gemoji shortcode.
    ///
    /// The borrowed form carries only the shortcode (without the `:`s);
    /// the glyph is resolved at conversion time by
    /// [`crate::emoji::shortcode_to_unicode`]. The matcher only emits
    /// this token when the catalog recognizes the shortcode — unknown
    /// `:foo:` runs stay [`InlineTok::Plain`] so prose like
    /// `meeting at 14:00 : ok?` is not silently rewritten.
    Emoji {
        /// Shortcode without the surrounding `:`s (e.g. `"tada"`).
        shortcode: &'a str,
    },
}

/// What `ref_at_cursor` resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefTarget {
    /// `[[name]]` — page reference (the disk path is `slugify(name)`).
    Page(String),
    /// `[[YYYY-MM-DD]]` — journal date reference.
    Journal(NaiveDate),
    /// `#name` — tag (resolves to a page with same name).
    Tag(String),
    /// `((blk-XXXXXX))` — block reference (lookup key into
    /// [`crate::index::WorkspaceIndex`]).
    Block(String),
}

/// Owned, serializable mirror of [`InlineTok`].
///
/// `InlineTok` borrows from the source string and is cheap to use
/// inside Rust. Anything that has to cross a serialization boundary
/// (a Tauri command's return value, a `BlockNode` DTO sent to a
/// frontend, a `Backlink.block_tokens` payload) needs owned strings
/// and a Serde-friendly shape. `InlineToken` is that shape.
///
/// The JSON form matches the schema mobile's TypeScript renderer
/// consumes one-for-one. Adding a variant in `InlineTok` requires
/// adding the same variant here plus the conversion in
/// [`InlineToken::from_borrowed`] in the same change — otherwise the
/// new variant silently degrades to `Plain` on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum InlineToken {
    /// Bare text with no formatting.
    Plain {
        /// Verbatim text.
        value: String,
    },
    /// `**bold**`. The inner span is re-tokenized so wiki-refs,
    /// tags, and block-refs nested inside the markers stay
    /// recognizable (e.g. `**[[avelino]]**` renders the bold `**` and
    /// the ref `[[avelino]]` as separate styled tokens, not a single
    /// flat string).
    Bold {
        /// Tokens of the inner span.
        inner: Vec<InlineToken>,
    },
    /// `*italic*` or `_italic_`. The TS renderer collapses both
    /// markers into one variant since the literal delimiter is
    /// purely cosmetic on the browser side; Rust consumers that need
    /// the marker keep using [`InlineTok`] directly.
    Italic {
        /// Tokens of the inner span.
        inner: Vec<InlineToken>,
    },
    /// `~~strike~~`.
    Strike {
        /// Tokens of the inner span.
        inner: Vec<InlineToken>,
    },
    /// `==highlight==`.
    Highlight {
        /// Tokens of the inner span.
        inner: Vec<InlineToken>,
    },
    /// `` `code` ``.
    Code {
        /// Inner text between the backticks.
        value: String,
    },
    /// `[text](url)` link.
    Link {
        /// Anchor text shown to the user.
        value: String,
        /// URL target.
        href: String,
    },
    /// `![alt](url)` image / embedded asset. `href` is a
    /// workspace-relative `assets/<hash>.<ext>` path or a remote URL;
    /// the client renders an `<img>` for image extensions and a viewer
    /// / file chip for other kinds.
    Image {
        /// Alt text (may be empty).
        alt: String,
        /// Image / asset target.
        href: String,
    },
    /// `[[name]]` page reference.
    Ref {
        /// Page name (display form, kept verbatim).
        value: String,
    },
    /// `#tag`. `value` includes the leading `#` so the frontend can
    /// render it as a single token without re-prefixing.
    Tag {
        /// Tag string including the `#` prefix (e.g. `"#project"`).
        value: String,
    },
    /// `((blk-XXXXXX))` block reference.
    #[serde(rename = "blockref")]
    BlockRef {
        /// Full handle including the `blk-` prefix.
        value: String,
    },
    /// `!((blk-XXXXXX))` block embed.
    Embed {
        /// Full handle including the `blk-` prefix.
        value: String,
    },
    /// `![[page]]` page embed — the page-level twin of `Embed`.
    ///
    /// Clients resolve `value` to a page and render its title plus its
    /// top-level blocks, read-only. It is also a page reference, so it
    /// carries a backlink.
    #[serde(rename = "pageembed")]
    PageEmbed {
        /// Page name, display form, verbatim.
        value: String,
    },
    /// `:shortcode:` GitHub gemoji shortcode.
    ///
    /// `shortcode` is the literal text between the `:`s
    /// (e.g. `"tada"`); `glyph` is the resolved unicode codepoint
    /// (e.g. `"🎉"`). Clients render `glyph` and surface `shortcode`
    /// for hover / `aria-label`. If the catalog ever misses (should
    /// not happen — the tokenizer pre-validates) the client should
    /// fall back to rendering `:${shortcode}:` literal.
    Emoji {
        /// Shortcode without the surrounding `:`s (e.g. `"tada"`).
        shortcode: String,
        /// Resolved unicode glyph (e.g. `"🎉"`).
        glyph: String,
    },
}

impl InlineToken {
    /// Convert a borrowed [`InlineTok`] into the owned, serializable
    /// form. The conversion is total — every variant maps 1:1.
    pub fn from_borrowed(tok: &InlineTok<'_>) -> Self {
        match tok {
            InlineTok::Plain(s) => InlineToken::Plain {
                value: (*s).to_owned(),
            },
            InlineTok::Bold { inner } => InlineToken::Bold {
                inner: inner.iter().map(InlineToken::from_borrowed).collect(),
            },
            InlineTok::Italic { inner, .. } => InlineToken::Italic {
                inner: inner.iter().map(InlineToken::from_borrowed).collect(),
            },
            InlineTok::Strike { inner } => InlineToken::Strike {
                inner: inner.iter().map(InlineToken::from_borrowed).collect(),
            },
            InlineTok::Highlight { inner } => InlineToken::Highlight {
                inner: inner.iter().map(InlineToken::from_borrowed).collect(),
            },
            InlineTok::Code { inner } => InlineToken::Code {
                value: (*inner).to_owned(),
            },
            InlineTok::Link { text, url } => InlineToken::Link {
                value: (*text).to_owned(),
                href: (*url).to_owned(),
            },
            InlineTok::Image { alt, url } => InlineToken::Image {
                alt: (*alt).to_owned(),
                href: (*url).to_owned(),
            },
            InlineTok::PageRef { name } => InlineToken::Ref {
                value: (*name).to_owned(),
            },
            InlineTok::PageEmbed { name } => InlineToken::PageEmbed {
                value: (*name).to_owned(),
            },
            InlineTok::Tag { name } => InlineToken::Tag {
                value: format!("#{name}"),
            },
            InlineTok::BlockRef { handle } => InlineToken::BlockRef {
                value: (*handle).to_owned(),
            },
            InlineTok::Embed { handle } => InlineToken::Embed {
                value: (*handle).to_owned(),
            },
            InlineTok::Emoji { shortcode } => InlineToken::Emoji {
                shortcode: (*shortcode).to_owned(),
                // The tokenizer only emits `Emoji` when the catalog
                // resolves — so this `unwrap_or("")` is a defensive
                // landing pad, not a code path we expect to hit.
                // Empty `glyph` lets the frontend fall back to the
                // literal `:shortcode:` form without crashing.
                glyph: crate::emoji::shortcode_to_unicode(shortcode)
                    .unwrap_or("")
                    .to_owned(),
            },
        }
    }
}
