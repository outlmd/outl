//! Unit tests for the frontmatter fence: the scan, the split, the line
//! count and their inverse.
//!
//! Split out of `frontmatter.rs` to keep the scan itself under the
//! file-size guard, on the same seam `parse/tests.rs` uses: implementation
//! / tests, never a seam inside the thing being tested. There is exactly
//! one scan here on purpose — `split_frontmatter`, `frontmatter_line_count`
//! and `crate::parse` all read it — so cutting it in two would be cutting
//! the invariant in two.
//!
//! `super` is `crate::frontmatter`, so every test keeps its exact path
//! and name.

use super::*;

// --- split_frontmatter -------------------------------------------------

#[test]
fn split_extracts_fenced_block() {
    let (yaml, body) = split_frontmatter("---\ntitle: X\n---\n- body\n");
    assert_eq!(yaml.as_deref(), Some("title: X"));
    assert_eq!(body, "- body\n");
}

#[test]
fn split_honours_document_end_marker() {
    let (yaml, body) = split_frontmatter("---\ntitle: X\n...\n- body\n");
    assert_eq!(yaml.as_deref(), Some("title: X"));
    assert_eq!(body, "- body\n");
}

#[test]
fn split_handles_crlf() {
    let (yaml, body) = split_frontmatter("---\r\ntitle: X\r\n---\r\n- body\r\n");
    assert_eq!(yaml.as_deref(), Some("title: X"));
    assert_eq!(body, "- body\n");
}

#[test]
fn split_without_fence_returns_original() {
    let (yaml, body) = split_frontmatter("- just bullets\n");
    assert!(yaml.is_none());
    assert_eq!(body, "- just bullets\n");
}

#[test]
fn split_without_closing_fence_keeps_whole_file_as_body() {
    // Malformed frontmatter must not eat the file.
    let (yaml, body) = split_frontmatter("---\ntitle: half\n- bullet\n");
    assert!(yaml.is_none());
    assert_eq!(body, "---\ntitle: half\n- bullet\n");
}

// --- the line count + the renderer ------------------------------------

#[test]
fn the_line_count_covers_both_delimiters() {
    assert_eq!(frontmatter_line_count("---\ntitle: X\n---\n- body\n"), 3);
    assert_eq!(frontmatter_line_count("---\n---\n- body\n"), 2);
    // A fence whose body is nothing but blank lines still occupies them.
    assert_eq!(frontmatter_line_count("---\n\n\n---\n- body\n"), 4);
    assert_eq!(frontmatter_line_count("- body\n"), 0);
    // Unterminated: not a fence, so its lines stay content.
    assert_eq!(frontmatter_line_count("---\ntitle: half\n- body\n"), 0);
}

#[test]
fn the_line_count_agrees_with_the_split() {
    for src in [
        "---\ntitle: X\n---\n- body\n",
        "---\n---\n",
        "---\n\n\n---\nrest\n",
        "---\r\ntitle: X\r\n---\r\nrest\r\n",
        "no fence here\n",
        "---\nunterminated\n",
        // A UTF-8 BOM sits before the opening delimiter in any `.md`
        // a Windows editor wrote.
        "\u{feff}---\ntitle: X\n---\nrest\n",
        "\u{feff}---\r\ntitle: X\r\n---\r\nrest\r\n",
        "\u{feff}---\nunterminated\n",
        "\u{feff}no fence here\n",
    ] {
        let (_, _, counted) = split_frontmatter_counted(src);
        assert_eq!(
            counted,
            frontmatter_line_count(src),
            "the two readings of one scan disagree on {src:?}"
        );
    }
}

/// A UTF-8 BOM sits **before** the opening delimiter in every `.md` a
/// Windows editor wrote, and [`crate::parse::parse`] drops it before
/// looking for the fence. This scan has to drop it too.
///
/// Not a cosmetic disagreement: `frontmatter_line_count` is what tells
/// `crate::unlogged` which leading lines are metadata, and that verdict
/// decides whether bytes may be overwritten.
#[test]
fn the_scan_sees_a_fence_behind_a_byte_order_mark() {
    let src = "\u{feff}---\ntitle: X\n---\n- hello\n";
    assert_eq!(frontmatter_line_count(src), 3);
    let (yaml, body, counted) = split_frontmatter_counted(src);
    assert_eq!(yaml.as_deref(), Some("title: X"));
    assert_eq!(
        body, "- hello\n",
        "the BOM must not ride into the body — the offsets and the text \
         they index have to come from one reading"
    );
    assert_eq!(counted, 3);
}

/// `\u{feff}---\r\n` is the shape a Windows editor actually writes, and
/// the CRLF branch normalizes the text *before* slicing it.
#[test]
fn the_scan_sees_a_crlf_fence_behind_a_byte_order_mark() {
    let (yaml, body) = split_frontmatter("\u{feff}---\r\ntitle: X\r\n---\r\n- hello\r\n");
    assert_eq!(yaml.as_deref(), Some("title: X"));
    assert_eq!(body, "- hello\n");
}

/// The scan and the parser must agree on **which files have
/// frontmatter at all**, not merely on where a fence they both see
/// ends.
///
/// `parse` drops a BOM before splitting, so as far as every block in
/// the op log is concerned a file opening `\u{feff}---` has
/// frontmatter. A scan answering `0` for it hands
/// `crate::unlogged::content_lines_missing_from` a `skip` of zero, the
/// fence's own lines are reported as content the log never saw,
/// `reconcile_md` withholds `last_synced_hash`, and invariant 8 refuses
/// to re-project the page — on every boot, permanently, because the
/// projection that would rewrite the BOM never runs.
#[test]
fn the_scan_and_the_parser_agree_on_which_files_have_frontmatter() {
    for src in [
        "---\ntitle: X\n---\n- body\n",
        "\u{feff}---\ntitle: X\n---\n- body\n",
        "\u{feff}---\r\ntitle: X\r\n---\r\n- body\r\n",
        "\u{feff}---\ntitle: X\n...\n- body\n",
        "\u{feff}- body\n",
        "- body\n",
        "---\nunterminated\n",
        "\u{feff}---\nunterminated\n",
    ] {
        assert_eq!(
            frontmatter_line_count(src) > 0,
            crate::parse::parse(src).frontmatter.is_some(),
            "the scan and the parser disagree about {src:?}"
        );
    }
}

#[test]
fn the_renderer_inverts_the_split() {
    for src in [
        "---\ntitle: X\ntags: [a, b]\n---\n",
        "---\n---\n",
        "---\nnested:\n  key: 1\n---\n",
    ] {
        let (yaml, _) = split_frontmatter(src);
        let yaml = yaml.expect("fence");
        assert_eq!(render_frontmatter(&yaml), src, "not an inverse for {src:?}");
    }
}

/// The one shape the pair deliberately normalizes: a `...` closer comes
/// back as `---`. Both end the fence, so nothing is lost, and the file
/// settles on one spelling instead of carrying two.
#[test]
fn a_document_end_closer_is_normalized_to_a_fence() {
    let (yaml, _) = split_frontmatter("---\ntitle: X\n...\n");
    assert_eq!(
        render_frontmatter(&yaml.expect("fence")),
        "---\ntitle: X\n---\n"
    );
}

#[test]
fn split_with_empty_body_after_fence() {
    let (yaml, body) = split_frontmatter("---\ntitle: X\n---\n");
    assert_eq!(yaml.as_deref(), Some("title: X"));
    assert_eq!(body, "");
}

// --- parse_frontmatter ---------------------------------------------------

#[test]
fn title_and_tags_are_extracted() {
    let fm = parse_frontmatter("title: Real Title\ntags: [foo, bar]", &[]).unwrap();
    assert_eq!(fm.title.as_deref(), Some("Real Title"));
    assert_eq!(
        fm.props,
        vec![("tags".to_string(), "#foo #bar".to_string())]
    );
    assert_eq!(fm.dropped, 0);
}

#[test]
fn tags_block_list_form() {
    let fm = parse_frontmatter("tags:\n  - alpha\n  - beta", &[]).unwrap();
    assert_eq!(
        fm.props,
        vec![("tags".to_string(), "#alpha #beta".to_string())]
    );
}

#[test]
fn tags_scalar_comma_separated_and_hash_prefixed() {
    let fm = parse_frontmatter("tags: \"#foo, bar\"", &[]).unwrap();
    assert_eq!(
        fm.props,
        vec![("tags".to_string(), "#foo #bar".to_string())]
    );
}

#[test]
fn unknown_scalar_keys_pass_through_in_order() {
    let fm = parse_frontmatter("author: jane\nrating: 7\ndone: true", &[]).unwrap();
    assert_eq!(
        fm.props,
        vec![
            ("author".to_string(), "jane".to_string()),
            ("rating".to_string(), "7".to_string()),
            ("done".to_string(), "true".to_string()),
        ]
    );
}

#[test]
fn drop_keys_are_counted_not_emitted() {
    let fm = parse_frontmatter(
        "aliases: [foo, bar]\ncssclass: wide\npublish: false",
        &["aliases", "cssclass", "publish", "scroll"],
    )
    .unwrap();
    assert!(fm.props.is_empty());
    assert_eq!(fm.dropped, 3);
}

#[test]
fn non_scalar_values_are_dropped_and_counted() {
    let fm = parse_frontmatter("meta:\n  nested: 1\nok: yes", &[]).unwrap();
    assert_eq!(fm.dropped, 1);
    assert_eq!(fm.props.len(), 1);
}

#[test]
fn invalid_yaml_returns_none() {
    assert!(parse_frontmatter("title: [unclosed", &[]).is_none());
}

#[test]
fn non_mapping_yaml_yields_empty_frontmatter() {
    let fm = parse_frontmatter("- a\n- b", &[]).unwrap();
    assert_eq!(fm, Frontmatter::default());
}

// --- extract_leading_h1 ------------------------------------------------

#[test]
fn leading_h1_is_lifted_and_stripped() {
    let (title, rest) = extract_leading_h1("# Real Heading\n- under h1\n");
    assert_eq!(title.as_deref(), Some("Real Heading"));
    assert_eq!(rest, "- under h1");
}

#[test]
fn blank_lines_before_h1_are_skipped() {
    let (title, _rest) = extract_leading_h1("\n\n# Heading\n- x\n");
    assert_eq!(title.as_deref(), Some("Heading"));
}

#[test]
fn buried_heading_is_not_a_title() {
    let (title, rest) = extract_leading_h1("- first\n# Not Title\n");
    assert!(title.is_none());
    assert_eq!(rest, "- first\n# Not Title\n");
}

#[test]
fn empty_h1_is_ignored() {
    let (title, _) = extract_leading_h1("# \n- x\n");
    assert!(title.is_none());
}
