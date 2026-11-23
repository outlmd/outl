//! Reference matchers — the tokens that point at something inside the
//! workspace: `[[name]]`, `((blk-XXXXXX))`, `!((blk-XXXXXX))`, `#tag`.
//!
//! These are exactly the constructs `cursor::ref_at_cursor` resolves to
//! a `RefTarget`, which is why they share a module: a change to what
//! counts as a valid handle or tag character has to be visible next to
//! every form that can carry one.
//!
//! Each matcher takes the source starting at its candidate opener and
//! returns the token plus the bytes consumed, or `None` so the run
//! falls through to `Plain`. `inline::match_one` owns the order.
//!
//! Recognition here is deliberately **strict**: prose that happens to
//! contain `((…))` must not be silently rewritten into a broken
//! reference (see [`is_valid_block_handle`]).

use crate::token::InlineTok;

pub(crate) fn try_page_ref(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix("[[")?;
    let close = rest.find("]]")?;
    let name = &rest[..close];
    if name.is_empty() || name.contains('\n') {
        return None;
    }
    Some((InlineTok::PageRef { name }, 2 + close + 2))
}

/// `![[page]]` — page embed.
///
/// The page-level twin of [`try_embed`], assembled from the same two
/// halves: `!` means "expand", `[[name]]` names a page. Name validation
/// mirrors [`try_page_ref`] exactly, so a name that is a legal ref is a
/// legal embed target and vice versa.
///
/// **An image target is refused**, and that is not an oversight.
/// `![[photo.png]]` is Obsidian's image embed, and
/// [`crate::wikilink::convert_image_links`] already turns it into
/// `![photo.png](photo.png)` on the way in from a vault. Claiming it
/// here would give one spelling two owners — and the one that loses is
/// whichever runs second, silently. Refusing leaves the shape exactly
/// as it tokenizes today (`Plain("!")` + [`InlineTok::PageRef`]), so
/// this matcher only ever claims input that previously meant nothing.
pub(crate) fn try_page_embed(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix("![[")?;
    let close = rest.find("]]")?;
    let name = &rest[..close];
    if name.is_empty() || name.contains('\n') || crate::wikilink::is_image_target(name) {
        return None;
    }
    // Consumed: `!` (1) + `[[` (2) + name + `]]` (2).
    Some((InlineTok::PageEmbed { name }, 1 + 2 + close + 2))
}

/// `!((blk-XXXXXX))` — block embed.
///
/// Markdown-image-shaped (`!((handle))` mirrors `![alt](url)`).
/// Strict on the inner handle for the same reason
/// [`try_block_ref`] is: arbitrary `!((..))` in prose must not be
/// silently rewritten as an embed.
pub(crate) fn try_embed(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix("!((")?;
    let close = rest.find("))")?;
    let handle = &rest[..close];
    if !is_valid_block_handle(handle) {
        return None;
    }
    // Consumed: `!` (1) + `((` (2) + handle + `))` (2).
    Some((InlineTok::Embed { handle }, 1 + 2 + close + 2))
}

/// `((blk-XXXXXX))` — Roam-style block reference.
///
/// The handle must look like a valid one: starts with `blk-`, followed
/// by 1 or more ASCII-alphanumeric lowercase characters. Anything else
/// falls back to `Plain` so plain prose using `((..))` for parentheticals
/// is not silently rewritten.
pub(crate) fn try_block_ref(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix("((")?;
    let close = rest.find("))")?;
    let handle = &rest[..close];
    if !is_valid_block_handle(handle) {
        return None;
    }
    Some((InlineTok::BlockRef { handle }, 2 + close + 2))
}

/// Validate a `((..))` payload as a block ref handle.
///
/// Conservative on purpose: the moment we accept arbitrary content
/// between `((` and `))`, prose like "look here ((really))" gets
/// rewritten as a broken reference. Loose validation is worse than no
/// recognition. Keep this aligned with [`crate::sidecar::derive_ref_handle`]
/// — handles are `blk-` plus at least [`crate::sidecar::REF_HANDLE_TAIL_LEN`]
/// lowercase ASCII alphanumerics; collision expansion only ever makes
/// them longer, never shorter.
pub fn is_valid_block_handle(handle: &str) -> bool {
    let Some(tail) = handle.strip_prefix(crate::sidecar::REF_HANDLE_PREFIX) else {
        return false;
    };
    if tail.chars().count() < crate::sidecar::REF_HANDLE_TAIL_LEN {
        return false;
    }
    tail.chars()
        .all(|c| c.is_ascii_alphanumeric() && !c.is_ascii_uppercase())
}

/// Spell `((handle))` — the token `try_block_ref` reads.
///
/// The writers live beside the readers because they are the same
/// decision seen from two sides, and they were not: the ref spelling
/// was written out by hand in the TUI's yank, in the Tauri command the
/// GUI clients call, and nowhere else was the embed spelling written
/// at all. Three `format!`s mean three places to forget the `!`.
///
/// Takes the handle the workspace index assigned — never one derived
/// on the spot, since collision expansion makes the stored handle
/// longer than [`crate::sidecar::derive_ref_handle`] would produce and
/// the short form would resolve to the *other* block.
pub fn block_ref_token(handle: &str) -> String {
    format!("(({handle}))")
}

/// Spell `!((handle))` — the token `try_embed` reads.
///
/// The embed is the ref with `!` in front: it points at the same block
/// and differs only in whether a reader expands it inline or links to
/// it. Writing it as its own function rather than as a `!` a caller
/// prepends is what keeps that `!` from being optional in practice.
pub fn block_embed_token(handle: &str) -> String {
    format!("!(({handle}))")
}

pub(crate) fn try_tag(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix('#')?;
    let mut tag_byte_end = 0usize;
    for (rel, ch) in rest.char_indices() {
        if ch.is_alphanumeric() || ch == '-' || ch == '_' || ch == '/' {
            tag_byte_end = rel + ch.len_utf8();
        } else {
            break;
        }
    }
    if tag_byte_end == 0 {
        return None;
    }
    Some((
        InlineTok::Tag {
            name: &rest[..tag_byte_end],
        },
        1 + tag_byte_end,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The writers are the readers' inverse, and that is the whole
    /// reason they live here: a token spelled in one module and parsed
    /// in another drifts on the first change to either.
    #[test]
    fn a_written_ref_parses_back_to_the_same_handle() {
        let token = block_ref_token("blk-r6s4a1");
        assert_eq!(token, "((blk-r6s4a1))");
        let (tok, used) = try_block_ref(&token).expect("writer must emit what the reader claims");
        assert_eq!(used, token.len());
        match tok {
            InlineTok::BlockRef { handle } => assert_eq!(handle, "blk-r6s4a1"),
            other => panic!("expected BlockRef, got {other:?}"),
        }
    }

    #[test]
    fn a_written_embed_parses_back_to_the_same_handle() {
        let token = block_embed_token("blk-r6s4a1");
        assert_eq!(token, "!((blk-r6s4a1))");
        let (tok, used) = try_embed(&token).expect("writer must emit what the reader claims");
        assert_eq!(used, token.len());
        match tok {
            InlineTok::Embed { handle } => assert_eq!(handle, "blk-r6s4a1"),
            other => panic!("expected Embed, got {other:?}"),
        }
    }

    /// Collision expansion only ever lengthens a handle
    /// (`block_index::assign_handle`), so the writers must not assume
    /// the 6-char tail `derive_ref_handle` starts from.
    #[test]
    fn an_expanded_handle_round_trips_in_both_forms() {
        let long = "blk-r6s4a1b2c3";
        assert!(is_valid_block_handle(long));
        assert!(try_block_ref(&block_ref_token(long)).is_some());
        assert!(try_embed(&block_embed_token(long)).is_some());
    }

    /// The embed is the ref with `!` in front — asserted rather than
    /// assumed, because the two are written by separate functions and
    /// a client picks between them by name.
    #[test]
    fn the_embed_is_the_ref_prefixed_with_a_bang() {
        let h = "blk-deadbe";
        assert_eq!(block_embed_token(h), format!("!{}", block_ref_token(h)));
    }
}
