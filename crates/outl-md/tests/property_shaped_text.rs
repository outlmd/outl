//! A block's text may hold a line shaped like a property
//! (`source:: x`). The renderer escapes it, so `render → parse` gives
//! back the same text, in the same order, and no property.

use outl_md::parse::parse;
use outl_md::render::render;
use outl_md::{OutlineNode, ParsedPage};

fn page_of(blocks: Vec<OutlineNode>) -> ParsedPage {
    ParsedPage {
        blocks,
        frontmatter: None,
        properties: Vec::new(),
        warnings: Vec::new(),
    }
}

fn node(text: &str, properties: &[(&str, &str)]) -> OutlineNode {
    OutlineNode {
        text: text.to_string(),
        properties: properties
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        children: Vec::new(),
    }
}

fn roundtrip(n: OutlineNode) -> (String, OutlineNode) {
    let rendered = render(&page_of(vec![n]));
    let mut back = parse(&rendered);
    assert_eq!(back.blocks.len(), 1, "rendered:\n{rendered}");
    (rendered, back.blocks.remove(0))
}

#[test]
fn a_property_shaped_line_in_the_text_stays_text_in_order() {
    let text = "intro\nsource:: https://example.com\nmiddle\nempty::\n  indented:: too\nend";
    let (rendered, back) = roundtrip(node(text, &[]));
    assert_eq!(back.text, text, "rendered:\n{rendered}");
    assert!(back.properties.is_empty(), "{:?}", back.properties);
    assert!(
        rendered.contains("\n  \\source:: https://example.com\n"),
        "rendered:\n{rendered}"
    );
}

#[test]
fn real_properties_still_render_and_parse_as_properties() {
    let (rendered, back) = roundtrip(node("intro\nkey:: in text", &[("owner", "ana")]));
    assert_eq!(back.text, "intro\nkey:: in text", "rendered:\n{rendered}");
    assert_eq!(back.properties, [("owner".to_string(), "ana".to_string())]);
}

#[test]
fn a_literal_backslash_before_a_property_shape_survives() {
    let text = "intro\n\\key:: value\n\\\\deeper:: x";
    let (rendered, back) = roundtrip(node(text, &[]));
    assert_eq!(back.text, text, "rendered:\n{rendered}");
    assert!(back.properties.is_empty(), "{:?}", back.properties);
}

#[test]
fn an_unescaped_property_line_on_disk_is_still_a_property() {
    let back = parse("- intro\n  key:: value\n  more\n");
    assert_eq!(back.blocks[0].text, "intro\nmore");
    assert_eq!(
        back.blocks[0].properties,
        [("key".to_string(), "value".to_string())]
    );
}

#[test]
fn an_escaped_property_line_is_logged_content() {
    let text = "intro\nsource:: x";
    let rendered = render(&page_of(vec![node(text, &[])]));
    let missing = outl_md::unlogged::content_lines_missing_from_texts(&rendered, [text]);
    assert!(missing.is_empty(), "{missing:?}");
}
