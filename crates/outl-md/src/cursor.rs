//! Cursor introspection over inline block content — "what token sits
//! under the caret?".
//!
//! UI-agnostic, like [`crate::inline`]: the TUI's `Enter` / `gx` chords,
//! a future GUI's click handler, and mobile's tap handler all ask the
//! same questions here instead of re-parsing block text per client.
//!
//! - [`ref_at_cursor`] resolves a navigable outl reference
//!   ([`RefTarget::Page`] / `Journal` / `Tag` / `Block`) under the caret.
//! - [`link_at_cursor`] resolves a standard markdown link `[text](url)`
//!   under the caret and returns its URL.
//! - [`byte_index_for_char`] converts a char index (what a cursor column
//!   is) into a byte offset (what `str` slicing needs).

use chrono::NaiveDate;

use crate::inline::{is_valid_block_handle, try_md_link, InlineTok, RefTarget};

/// If `char_index` falls inside a `[[ref]]`, `#tag`, or `[[date]]` token
/// in `text`, return the corresponding [`RefTarget`]. Otherwise `None`.
pub fn ref_at_cursor(text: &str, char_index: usize) -> Option<RefTarget> {
    let cursor_byte = byte_index_for_char(text, char_index);

    // Scan `[[...]]` ranges. A preceding `!` (the page-embed form
    // `![[name]]`) widens the match by one byte so a cursor sitting on
    // the `!` still resolves to the same page — the same courtesy the
    // block-embed scan below extends to `!((blk-…))`. Both forms are
    // page / block *references*, so "go to target" is the right verb
    // whichever half of the token the caret is on.
    let mut search = 0usize;
    while let Some(rel_open) = text[search..].find("[[") {
        let abs_open = search + rel_open;
        let inner_start = abs_open + 2;
        let Some(rel_close) = text[inner_start..].find("]]") else {
            break;
        };
        let inner_end = inner_start + rel_close;
        let abs_close_end = inner_end + 2;
        let match_start = if abs_open > 0 && text.as_bytes()[abs_open - 1] == b'!' {
            abs_open - 1
        } else {
            abs_open
        };
        if cursor_byte >= match_start && cursor_byte <= abs_close_end {
            let inner = &text[inner_start..inner_end];
            if let Ok(date) = NaiveDate::parse_from_str(inner, "%Y-%m-%d") {
                return Some(RefTarget::Journal(date));
            }
            return Some(RefTarget::Page(inner.to_string()));
        }
        search = abs_close_end;
    }

    // Scan `((blk-...))` ranges. A preceding `!` (embed form) widens
    // the match by one byte so a cursor sitting on `!` still resolves
    // to the same target.
    //
    // Bug fix: when the candidate handle fails validation we advance
    // by ONE byte (not past the closing `))`) so an overlapping valid
    // handle still gets a chance. Example: `((((blk-x))))` — the
    // outer `((` captures `((blk-x` (invalid). Skipping to the first
    // `))` would step past the real `((blk-x))` at offset 2.
    let mut search = 0usize;
    while let Some(rel_open) = text[search..].find("((") {
        let abs_open = search + rel_open;
        let inner_start = abs_open + 2;
        let Some(rel_close) = text[inner_start..].find("))") else {
            break;
        };
        let inner_end = inner_start + rel_close;
        let abs_close_end = inner_end + 2;
        let handle = &text[inner_start..inner_end];
        if !is_valid_block_handle(handle) {
            search = abs_open + 1;
            continue;
        }
        let starts_at = if abs_open > 0 && text.as_bytes()[abs_open - 1] == b'!' {
            abs_open - 1
        } else {
            abs_open
        };
        if cursor_byte >= starts_at && cursor_byte <= abs_close_end {
            return Some(RefTarget::Block(handle.to_string()));
        }
        search = abs_close_end;
    }

    // Scan `#tag` ranges.
    let mut idx = 0usize;
    while idx < text.len() {
        if text[idx..].starts_with('#') {
            let after = &text[idx + 1..];
            let mut tag_byte_end = 0usize;
            for (rel, ch) in after.char_indices() {
                if ch.is_alphanumeric() || ch == '-' || ch == '_' || ch == '/' {
                    tag_byte_end = rel + ch.len_utf8();
                } else {
                    break;
                }
            }
            if tag_byte_end > 0 {
                let abs_end = idx + 1 + tag_byte_end;
                if cursor_byte >= idx && cursor_byte <= abs_end {
                    let name = &text[idx + 1..abs_end];
                    return Some(RefTarget::Tag(name.to_string()));
                }
                idx = abs_end;
                continue;
            }
        }
        let ch = text[idx..].chars().next()?;
        idx += ch.len_utf8();
    }

    None
}

/// If `char_index` falls anywhere inside a standard markdown link
/// `[text](url)` in `text` — over the anchor text *or* the URL — return
/// the link's URL. Otherwise `None`.
///
/// Reuses the canonical `try_md_link` matcher so the notion of "what is
/// a link" stays owned in one place; `[[page]]` refs are skipped (they are
/// not links). Consumed by clients that follow a link under the cursor
/// (e.g. the TUI's `gx` chord) without reimplementing link parsing.
pub fn link_at_cursor(text: &str, char_index: usize) -> Option<&str> {
    let cursor_byte = byte_index_for_char(text, char_index);

    let mut search = 0usize;
    while let Some(rel_open) = text[search..].find('[') {
        let abs_open = search + rel_open;
        match try_md_link(&text[abs_open..]) {
            Some((InlineTok::Link { url, .. }, consumed)) => {
                // Cursor inside `[abs_open, abs_open + consumed]` (inclusive
                // of the trailing `)` so a cursor parked on it still opens).
                if cursor_byte >= abs_open && cursor_byte <= abs_open + consumed {
                    return Some(url);
                }
                search = abs_open + consumed;
            }
            // Not a link here (bare `[`, `[[ref]]`, unbalanced). Advance one
            // byte so an overlapping link right after still gets a chance.
            _ => search = abs_open + 1,
        }
    }

    None
}

/// Convert a char index (0-based) into the corresponding byte offset.
///
/// Returns `s.len()` when the char index is at or past the end. Always
/// safe to pass into `s.split_at(...)`.
pub fn byte_index_for_char(s: &str, char_index: usize) -> usize {
    s.char_indices()
        .nth(char_index)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}
