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
        if !has_visible_property(&page.properties) && !page.blocks.is_empty() {
            out.push('\n');
        }
    }

    // Page properties at the top. An empty value is not a line: the
    // reader already treats `key::` as absent, and writing it back
    // leaves a key no surface can clear.
    for (k, v) in &page.properties {
        write_property(&mut out, 0, k, v);
    }
    // Separator line between header and outline. Empty values do not
    // count: a header that writes nothing must not insert a blank line.
    if has_visible_property(&page.properties) && !page.blocks.is_empty() {
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
    let mut lines = text.split('\n');
    // First line keeps the bullet.
    let first = lines.next().unwrap_or("");
    out.push_str("- ");
    out.push_str(first);
    out.push('\n');
    // Remaining lines are continuation: indented one level deeper so
    // they align under the `-` and our own parser recognizes them.
    for line in lines {
        write_indent(out, indent + 1);
        out.push_str(line);
        out.push('\n');
    }
}

fn write_indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str(INDENT_UNIT);
    }
}

/// A value that is empty or only whitespace has no `key:: value` line.
/// The parser stores a bare `key::` as `""`, and every reader already
/// treats that as absent, so writing the line back leaves a key no
/// surface can see.
fn has_visible_property(properties: &[(String, String)]) -> bool {
    properties.iter().any(|(_, value)| !value.trim().is_empty())
}

fn write_property(out: &mut String, indent: usize, key: &str, value: &str) {
    if value.trim().is_empty() {
        return;
    }
    write_indent(out, indent);
    let _ = write!(out, "{key}:: {value}");
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

    /// An empty value is not a line. The reader already drops `key::`
    /// with nothing after it; the writer has to do the same, or a
    /// cleared property stays on disk forever.
    #[test]
    fn an_empty_property_is_not_written() {
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
        assert!(
            !out.contains("foo::"),
            "an empty page property must not be a line: {out:?}"
        );
        assert!(
            !out.contains("blank::"),
            "a whitespace-only property must not be a line: {out:?}"
        );
        assert!(
            !out.contains("status::"),
            "an empty block property must not be a line: {out:?}"
        );
        assert_eq!(out, "title:: HB\n\n- body\n");
    }

    /// A header made only of empty values is the same as no header:
    /// no leading blank line in front of the outline.
    #[test]
    fn a_header_of_only_empty_properties_does_not_open_with_a_blank_line() {
        let page = ParsedPage {
            properties: vec![("foo".into(), String::new())],
            blocks: vec![OutlineNode {
                text: "body".into(),
                ..OutlineNode::default()
            }],
            ..ParsedPage::default()
        };
        assert_eq!(render(&page), "- body\n");
    }

    /// The blank line after a YAML fence is what keeps the outline out
    /// of the property header. It still belongs there when every
    /// `key::` value is empty and therefore unwritten.
    #[test]
    fn frontmatter_still_gets_a_blank_line_when_every_property_is_empty() {
        let page = ParsedPage {
            frontmatter: Some("title: kept".into()),
            properties: vec![("foo".into(), String::new())],
            blocks: vec![OutlineNode {
                text: "body".into(),
                ..OutlineNode::default()
            }],
            ..ParsedPage::default()
        };
        assert_eq!(render(&page), "---\ntitle: kept\n---\n\n- body\n");
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
