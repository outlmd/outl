//! Literal capture of fenced code blocks while parsing a page.
//!
//! Inside a bullet, ` ``` ` suspends the outline grammar: every line
//! down to the matching closing fence is content, even when it looks
//! like a `- bullet` or a `key:: value` property. This module owns
//! that suspension — where the fence ends, what gets stripped, and
//! what happens when the user never closed it.
//!
//! Reached only from [`crate::parse`]'s block-list reader (a fence is
//! not addressable on its own), so the behaviour is pinned through
//! `parse` in the tests below.

use crate::parse::{leading_indent, INDENT_WIDTH};

/// The fence marker that `stripped` opens a fenced code block with, if it
/// opens one at all.
///
/// CommonMark has **two** fence characters and this dialect advertises
/// "fenced code", not "backtick-fenced code", so `~~~` suspends the
/// outline grammar exactly as ``` ``` ``` does. Recognising only one of
/// them meant a bullet inside a tilde fence became a real block — content
/// promoted into the tree, with an `UnrecognizedBlockMarker` raised
/// against a line the user wrote correctly.
///
/// The marker travels with the opener because a fence closes only on its
/// **own** character: the other one appearing in the body is content, and
/// treating it as a closer ends the fence early and hands the rest of the
/// body back to the outline grammar.
pub(crate) fn fence_marker(stripped: &str) -> Option<&'static str> {
    const MARKERS: [&str; 2] = ["```", "~~~"];
    MARKERS.into_iter().find(|m| stripped.starts_with(m))
}

/// Consume a fenced code block from `lines[*i]` (the opening fence)
/// up to and including the matching closing fence. The full literal
/// content — including the fences themselves — gets appended to
/// `target` so a later [`crate::render::render`] can emit it back
/// untouched.
///
/// The closing fence is recognized as a line whose trimmed content is
/// **exactly** ` ``` `, ignoring any info string on the opener. Tabs
/// inside the fence are preserved (we always read the raw line, not
/// the indent-stripped form).
///
/// `i` is advanced past the closing fence on success, or to the end
/// of the input if the closing fence is missing (graceful close).
///
/// `separate` says whether the opener starts a new line of `target`.
/// The caller decides, because only it knows whether an empty `target`
/// means "this block has no text yet" or "this block's text starts with
/// an empty first line" — `- ` followed by a fence is the second, and
/// deciding it here from `target.is_empty()` dropped that empty line.
/// See `parse::needs_newline_separator`, the one owner of the answer.
pub(crate) fn consume_fence(
    lines: &[&str],
    i: &mut usize,
    fence_indent: usize,
    separate: bool,
    target: &mut String,
) {
    // The opening fence line itself.
    let opener = lines[*i];
    let opener_stripped = opener.trim();
    // The opener decides which character closes this fence. Callers reach
    // here only after `fence_marker` already said yes, so the fallback is
    // unreachable in practice and still has to be total.
    let marker = fence_marker(opener_stripped).unwrap_or("```");
    if separate {
        target.push('\n');
    }
    target.push_str(opener_stripped);
    *i += 1;
    consume_fence_until_close(lines, i, fence_indent, marker, target);
}

/// Same as [`consume_fence`], but the opener line is assumed to be
/// already in `target` (e.g. it was part of the block's first line:
/// `- ```lisp`). Picks up at the body, scans until the matching
/// closing fence at `fence_indent`, and advances `*i` past it.
pub(crate) fn consume_fence_until_close(
    lines: &[&str],
    i: &mut usize,
    fence_indent: usize,
    marker: &str,
    target: &mut String,
) {
    // A fence closes only on the character that opened it.
    let fence_char = marker.chars().next().unwrap_or('`');
    while *i < lines.len() {
        let raw = lines[*i];
        let stripped = raw.trim();
        // A closing fence is three (or more) of that character, alone.
        let is_closing = stripped.starts_with(marker)
            && stripped.chars().skip(marker.len()).all(|c| c == fence_char);
        if is_closing && leading_indent(raw) == fence_indent {
            target.push('\n');
            target.push_str(stripped);
            *i += 1;
            return;
        }
        // A line outdented below the fence — e.g. a `- next block` at
        // indent 0 while the fence body lives at indent 1 — is *not*
        // part of this fence. Leave it for the outer parser to handle.
        // Without this guard a missing closer would swallow the rest
        // of the document.
        if leading_indent(raw) < fence_indent && !stripped.is_empty() {
            break;
        }
        // Inside the fence: append exactly what the user wrote,
        // minus the outer indent. Content indentation relative to
        // the fence is preserved.
        let preserved = strip_indent(raw, fence_indent);
        target.push('\n');
        target.push_str(preserved);
        *i += 1;
    }
    // Reached EOF (or an out-dented sibling) without a closing fence.
    // Leave a synthetic close so the rendered output stays well-formed
    // and the next parse round-trips. It has to be the *opener's* marker,
    // or the synthesized close would not close the fence it was written
    // for and the next parse would swallow the rest of the document.
    target.push('\n');
    target.push_str(marker);
}

/// Drop `level * INDENT_WIDTH` leading spaces from a line if present.
/// Used inside fenced code blocks so we don't bake outline indent
/// into the user's literal content.
fn strip_indent(line: &str, level: usize) -> &str {
    let want = level * INDENT_WIDTH;
    let mut count = 0usize;
    let mut byte = 0usize;
    for b in line.bytes() {
        if count >= want {
            break;
        }
        if b == b' ' {
            count += 1;
            byte += 1;
        } else if b == b'\t' {
            count += INDENT_WIDTH;
            byte += 1;
        } else {
            break;
        }
    }
    &line[byte..]
}

#[cfg(test)]
mod tests {
    use crate::parse::parse;

    #[test]
    fn fence_preserves_literal_content() {
        let md = "- intro\n  ```lisp\n  (+ 1 2)\n  ```\n- next\n";
        let p = parse(md);
        let expected = "intro\n```lisp\n(+ 1 2)\n```";
        assert_eq!(p.blocks[0].text, expected);
        assert_eq!(p.blocks[1].text, "next");
    }

    #[test]
    fn fence_keeps_outline_markers_literal() {
        // The `- not a block` line inside the fence must NOT become a
        // child block — that's the whole reason for fence mode.
        let md = "- header\n  ```\n  - not a block\n  - me neither\n  ```\n- next\n";
        let p = parse(md);
        assert_eq!(
            p.blocks[0].text,
            "header\n```\n- not a block\n- me neither\n```"
        );
        assert!(p.blocks[0].children.is_empty());
    }

    #[test]
    fn unclosed_fence_synthesizes_close() {
        let md = "- header\n  ```\n  oh no\n";
        let p = parse(md);
        assert!(p.blocks[0].text.ends_with("```"));
    }

    #[test]
    fn fence_opened_on_bullet_line_does_not_swallow_next_block() {
        // Regression: when the bullet line itself opens the fence
        // (`- ```lisp`), the parser used to keep `consume_fence` blind
        // to whether the next bullet at a lower indent was actually a
        // new block. The closing `` ``` `` got mistaken for an opener,
        // the `- **abc** __123__` line got absorbed as fence body, and
        // a synthetic close was appended at EOF — three corruptions
        // for the price of one.
        let md = "- ```lisp\n  (+ 1 2)\n  ```\n- **abc** __123__\n";
        let p = parse(md);
        assert_eq!(p.blocks.len(), 2, "must keep both blocks");
        assert_eq!(p.blocks[0].text, "```lisp\n(+ 1 2)\n```");
        assert_eq!(p.blocks[1].text, "**abc** __123__");
    }

    #[test]
    fn fence_opened_on_bullet_line_with_unclosed_body_stops_at_sibling() {
        // Even with a missing closer, a sibling block at outer indent
        // ends the fence — better to synthesize a close than swallow
        // every following block.
        let md = "- ```lisp\n  oops no close\n- next block\n";
        let p = parse(md);
        assert_eq!(p.blocks.len(), 2);
        assert!(p.blocks[0].text.ends_with("```"));
        assert_eq!(p.blocks[1].text, "next block");
    }
}
