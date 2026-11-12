//! Render an outline AST back to clean `.md` (no IDs).
//!
//! Output must be CommonMark-compliant and roundtrip cleanly through
//! [`crate::parse::parse`]. Property test in `tests/roundtrip.rs`.

use crate::parse::{OutlineNode, ParsedPage};
use std::fmt::Write;

const INDENT_UNIT: &str = "  ";

/// Render a `ParsedPage` to a `.md` string.
pub fn render(page: &ParsedPage) -> String {
    let mut out = String::new();

    // A YAML frontmatter fence, when the page has one, goes first and
    // verbatim — it is only valid at the very top of the file, so its
    // position here is not a formatting choice. `properties` never
    // carries it (see `ParsedPage::frontmatter`), so there is nothing to
    // skip in the loop below.
    if let Some(yaml) = &page.frontmatter {
        out.push_str(&crate::frontmatter::render_frontmatter(yaml));
        // A blank line after the closing delimiter, but **only** when the
        // outline follows it directly. `read_page_header` ends the
        // page-property run at the first blank line, so writing one in
        // front of `title:: x` would push every page property out of the
        // header and into the outline as recovered blocks — the same
        // "content read in the wrong shape" this fix exists to stop, one
        // construct over.
        if page.properties.is_empty() && !page.blocks.is_empty() {
            out.push('\n');
        }
    }

    // Page properties at the top. An empty value is still a line, a
    // bare `key::`: the key was written by someone (a source graph, a
    // `prop set key=`) and dropping it loses that. Removing a key is
    // `SetProp` with no value, which takes it out of `properties`.
    for (k, v) in &page.properties {
        write_property(&mut out, 0, k, v);
    }
    // Separator line between header and outline.
    if !page.properties.is_empty() && !page.blocks.is_empty() {
        out.push('\n');
    }

    render_blocks(&page.blocks, &mut out, 0);

    out
}

fn render_blocks(blocks: &[OutlineNode], out: &mut String, indent: usize) {
    for block in blocks {
        write_block_text(out, indent, &block.text);
        for (k, v) in &block.properties {
            write_property(out, indent + 1, k, v);
        }
        render_blocks(&block.children, out, indent + 1);
    }
}

/// Render a block's `text` to `out`. The first line goes after the
/// `- ` bullet at `indent`; any subsequent lines (the user pressed
/// Shift+Enter while typing, or imported a code fence) are emitted
/// at `indent + 1` so they read as continuation when [`crate::parse`]
/// rounds back.
fn write_block_text(out: &mut String, indent: usize, text: &str) {
    write_indent(out, indent);
    if text.is_empty() {
        out.push('-');
        out.push('\n');
        return;
    }
    let mut lines = block_text_lines(text).into_iter();
    // First line keeps the bullet.
    let first = lines.next().unwrap_or_default();
    out.push_str("- ");
    out.push_str(&first);
    out.push('\n');
    // Remaining lines are continuation: indented one level deeper so
    // they align under the `-` and our own parser recognizes them.
    for line in lines {
        write_indent(out, indent + 1);
        out.push_str(&line);
        out.push('\n');
    }
}

/// A trimmed line the parser reads as a block marker (`- x` / `-`).
fn is_bullet_shaped(body: &str) -> bool {
    body == "-" || body.starts_with("- ")
}

/// A trimmed line that opens a fenced code block, or the escaped form
/// of one (`\```yaml`), which needs one more backslash so the unescape
/// on the way back is unambiguous.
fn is_fence_escapable(body: &str) -> bool {
    crate::fence::fence_marker(body).is_some()
        || body.strip_prefix('\\').is_some_and(is_fence_escapable)
}

/// A trimmed line whose leading backslash the parser drops on a
/// continuation line: a bullet, a fence opener, a `key:: value` line, or
/// the escaped form of any of them.
fn is_escapable(body: &str) -> bool {
    is_bullet_shaped(body)
        || crate::fence::fence_marker(body).is_some()
        || is_property_shaped(body)
        || body.strip_prefix('\\').is_some_and(is_escapable)
}

/// `\key:: value`: a continuation line the renderer escaped, which the
/// parser reads as text and not as a property.
pub(crate) fn is_escaped_text(line: &str) -> bool {
    line.strip_prefix('\\').is_some_and(is_escapable)
}

/// A line the parser would lift into a block property.
fn is_property_shaped(body: &str) -> bool {
    crate::property::parse_property_line(body.trim()).is_some()
}

/// Whether `line` closes a fence opened with `marker`, read the way
/// [`crate::fence`] reads it: the marker (or more of its character)
/// alone, at the continuation column (less than one indent level of
/// leading whitespace of its own).
fn closes_fence(line: &str, marker: &str) -> bool {
    let fence_char = marker.chars().next().unwrap_or('`');
    let stripped = line.trim();
    crate::parse::leading_indent(line) == 0
        && stripped.starts_with(marker)
        && stripped.chars().skip(marker.len()).all(|c| c == fence_char)
}

/// A block's text split into the lines the renderer writes: every line
/// that the parser would otherwise read as structure is escaped with a
/// leading backslash, so the block comes back as exactly one block with
/// exactly this text.
///
/// Two shapes need it:
///
/// - A **continuation line shaped like a bullet** (`- item`). Written
///   verbatim it parses back as a child block. A Roam/Logseq block like
///   `"head\n- item"` came back as two blocks, the import lost its 1:1
///   DFS mapping and every ref into the page degraded to a link.
/// - A **fence opener with no closer in the same text** (`"intro\n```yaml"`,
///   with the YAML in the child blocks). Written verbatim the open fence
///   swallows the children, so several blocks come back as one.
/// - A **continuation line shaped like a property** (`"intro\nsource:: x"`).
///   Written verbatim the parser lifts it out of the text into a block
///   property, so the line leaves the text and lands after it: the text
///   and its order change on the first round trip.
///
/// A line that already starts with a backslash in front of one of those
/// shapes gets one more, so the unescape is a strict inverse. Lines
/// inside a closed fence are never escaped: the parser suspends the
/// outline grammar there, so they already round-trip.
pub fn block_text_lines(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = Vec::with_capacity(lines.len());
    let mut fence: Option<&'static str> = None;
    for (n, line) in lines.iter().enumerate() {
        let body = line.trim_start();
        let escape = if let Some(m) = fence {
            if closes_fence(line, m) {
                fence = None;
            }
            false
        } else if n == 0 {
            // The first line sits after the bullet; only a fence opener
            // is structure there. A bullet-shaped first line is text.
            match crate::fence::fence_marker(body) {
                Some(m) if has_closer(&lines[1..], m) => {
                    fence = Some(m);
                    false
                }
                Some(_) => true,
                None => body.strip_prefix('\\').is_some_and(is_fence_escapable),
            }
        } else if is_bullet_shaped(body)
            || is_property_shaped(body)
            || body.strip_prefix('\\').is_some_and(is_escapable)
        {
            true
        } else if let Some(m) = crate::fence::fence_marker(body) {
            if crate::parse::leading_indent(line) > 0 {
                // Over-indented: the parser keeps it as plain
                // continuation, never as a fence.
                false
            } else if has_closer(&lines[n + 1..], m) {
                fence = Some(m);
                false
            } else {
                true
            }
        } else {
            false
        };
        if escape {
            let ws = &line[..line.len() - body.len()];
            out.push(format!("{ws}\\{body}"));
        } else {
            out.push((*line).to_string());
        }
    }
    out
}

/// Whether a fence opened with `marker` closes somewhere in `rest`.
fn has_closer(rest: &[&str], marker: &str) -> bool {
    rest.iter().any(|l| closes_fence(l, marker))
}

/// Inverse of the escape in [`block_text_lines`] for one continuation
/// line: drop exactly one backslash in front of an escapable body.
pub(crate) fn unescape_continuation_line(line: &str) -> String {
    let body = line.trim_start();
    match body.strip_prefix('\\') {
        Some(rest) if is_escapable(rest) => {
            let ws = &line[..line.len() - body.len()];
            format!("{ws}{rest}")
        }
        _ => line.to_string(),
    }
}

/// Inverse of the escape in [`block_text_lines`] for a block's first
/// line, where only a fence opener is ever escaped.
pub(crate) fn unescape_first_line(body: &str) -> Option<&str> {
    body.strip_prefix('\\')
        .filter(|rest| is_fence_escapable(rest))
}

/// The text a disk line stands for once its escape is undone: `first`
/// says the line carried the block's own `- ` marker.
pub(crate) fn unescaped_disk_body(body: &str, first: bool) -> Option<&str> {
    let rest = body.strip_prefix('\\')?;
    if first {
        is_fence_escapable(rest).then_some(rest)
    } else {
        is_escapable(rest).then_some(rest)
    }
}

fn write_indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str(INDENT_UNIT);
    }
}

fn write_property(out: &mut String, indent: usize, key: &str, value: &str) {
    write_indent(out, indent);
    if value.trim().is_empty() {
        // `key::` form (no trailing space, no value). The parser reads
        // it back as `""`.
        let _ = write!(out, "{key}::");
    } else {
        let _ = write!(out, "{key}:: {value}");
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn roundtrip(md: &str) -> String {
        render(&parse(md))
    }

    #[test]
    fn page_props_only() {
        let md = "title:: foo\nstatus:: active\n";
        let out = roundtrip(md);
        assert!(out.contains("title:: foo"));
        assert!(out.contains("status:: active"));
    }

    #[test]
    fn simple_outline_roundtrips() {
        let md = "- a\n- b\n";
        let out = roundtrip(md);
        assert_eq!(out, "- a\n- b\n");
    }

    #[test]
    fn nested_with_properties() {
        let md = "- objective\n  priority:: high\n  - sub1\n  - sub2\n";
        let out = roundtrip(md);
        let reparsed = parse(&out);
        let original = parse(md);
        assert_eq!(reparsed, original);
    }

    #[test]
    fn empty_block_renders_dash_only() {
        let md = "-\n";
        let out = roundtrip(md);
        assert_eq!(out, "-\n");
    }

    #[test]
    fn page_props_get_blank_separator_before_blocks() {
        let md = "title:: foo\n\n- one\n";
        let out = roundtrip(md);
        assert!(out.contains("title:: foo\n\n- one"));
    }

    /// An empty value is still a line, a bare `key::`, on a page and on
    /// a block. A source graph writes `published::` on purpose; the
    /// writer keeps it so the key survives a round trip.
    #[test]
    fn an_empty_property_is_written_as_a_bare_key() {
        let page = ParsedPage {
            properties: vec![
                ("foo".into(), String::new()),
                ("title".into(), "HB".into()),
                ("blank".into(), "   ".into()),
            ],
            blocks: vec![OutlineNode {
                text: "body".into(),
                properties: vec![("status".into(), String::new())],
                ..OutlineNode::default()
            }],
            ..ParsedPage::default()
        };
        let out = render(&page);
        assert_eq!(out, "foo::\ntitle:: HB\nblank::\n\n- body\n  status::\n");
        let back = crate::parse(&out);
        assert_eq!(back.properties[0], ("foo".into(), String::new()));
        assert_eq!(
            back.blocks[0].properties,
            vec![("status".into(), String::new())]
        );
    }

    /// A header made only of empty values is still a header, so the
    /// blank line that ends it is still there.
    #[test]
    fn a_header_of_only_empty_properties_keeps_its_separator() {
        let page = ParsedPage {
            properties: vec![("foo".into(), String::new())],
            blocks: vec![OutlineNode {
                text: "body".into(),
                ..OutlineNode::default()
            }],
            ..ParsedPage::default()
        };
        assert_eq!(render(&page), "foo::\n\n- body\n");
    }

    /// After a YAML fence the `key::` header follows directly, with no
    /// blank line in between, even when every value is empty.
    #[test]
    fn frontmatter_is_followed_by_an_empty_property_header() {
        let page = ParsedPage {
            frontmatter: Some("title: kept".into()),
            properties: vec![("foo".into(), String::new())],
            blocks: vec![OutlineNode {
                text: "body".into(),
                ..OutlineNode::default()
            }],
            ..ParsedPage::default()
        };
        assert_eq!(render(&page), "---\ntitle: kept\n---\nfoo::\n\n- body\n");
    }

    #[test]
    fn multiline_block_uses_continuation_indent() {
        let md = "- first line\n  second line\n  third line\n";
        let out = roundtrip(md);
        assert_eq!(out, "- first line\n  second line\n  third line\n");
    }

    #[test]
    fn fenced_code_inside_block_roundtrips() {
        let md = "- intro\n  ```lisp\n  (+ 1 2)\n  ```\n- next\n";
        let reparsed = parse(&roundtrip(md));
        let original = parse(md);
        assert_eq!(reparsed, original);
    }

    #[test]
    fn nested_block_with_multiline_parent() {
        let md = "- parent line one\n  parent line two\n  - child block\n";
        let out = roundtrip(md);
        let reparsed = parse(&out);
        let original = parse(md);
        assert_eq!(reparsed, original);
    }

    // ---- Blockquote (`> ` prefix) round-trip ----
    //
    // Quote is encoded as a per-block text prefix (see
    // `outl_actions::quote`). The parser already preserves the `> ` on
    // continuation lines (each continuation line lands in
    // `OutlineNode.text` separated by `\n`); the renderer emits each
    // continuation at indent+1 verbatim. These tests pin that
    // behaviour so a future refactor of the dialect doesn't silently
    // strip the marker.

    #[test]
    fn single_line_quote_roundtrips() {
        let md = "- > the only way to do great work\n";
        let out = roundtrip(md);
        assert_eq!(out, "- > the only way to do great work\n");
    }

    #[test]
    fn multi_line_quote_roundtrips_with_marker_on_each_line() {
        let md = "- > line one\n  > line two\n  > line three\n";
        let out = roundtrip(md);
        assert_eq!(out, "- > line one\n  > line two\n  > line three\n");
    }

    #[test]
    fn mixed_quoted_and_regular_siblings_roundtrip() {
        let md = "- regular\n- > quoted\n- another regular\n";
        let out = roundtrip(md);
        assert_eq!(out, "- regular\n- > quoted\n- another regular\n");
    }

    #[test]
    fn quoted_block_with_inline_tokens_preserves_them() {
        // The wrapper is transparent — bold / ref / tag inside the
        // body stay verbatim.
        let md = "- > **bold** [[ref]] #tag\n";
        let out = roundtrip(md);
        assert_eq!(out, "- > **bold** [[ref]] #tag\n");
    }

    #[test]
    fn quoted_block_with_child_roundtrips() {
        // Children of a quoted block are not implicitly quoted (the
        // marker lives on the block, not on the subtree).
        let md = "- > parent quote\n  - normal child\n";
        let out = roundtrip(md);
        let reparsed = parse(&out);
        let original = parse(md);
        assert_eq!(reparsed, original);
    }
}
