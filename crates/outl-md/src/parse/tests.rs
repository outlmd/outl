//! Unit tests for the outl markdown grammar.
//!
//! Split out of `parse.rs` to keep the grammar itself under the
//! file-size guard *without* cutting through it. `parse.rs` is invariant
//! 8 territory — issue #210 was a continuation bug, and the continuation
//! reader, the blank-line arm and the recovery arms only make sense read
//! together — so the seam is implementation / tests, never a seam inside
//! the parser.
//!
//! `super` is `crate::parse`, so every test keeps its exact path and
//! name. Several are cited by name in the root `CLAUDE.md` regression
//! net; renaming one breaks `cargo test <name>` for everybody.

use super::*;

/// A `remind::` the scheduler can't read never costs the user the
/// property or the block — only the scheduling. The recovery is
/// reported with the exact source line so a banner can point at it.
#[test]
fn invalid_remind_warns_but_keeps_the_property() {
    let md = "- TODO ship it\n  remind:: every 1h\n";
    let p = parse(md);
    assert_eq!(p.blocks.len(), 1);
    assert_eq!(
        p.blocks[0].properties,
        vec![("remind".to_string(), "every 1h".to_string())]
    );
    assert_eq!(p.warnings.len(), 1);
    assert_eq!(p.warnings[0].line, 2);
    assert_eq!(p.warnings[0].kind, ParseWarningKind::RemindMissingAnchor);
}

#[test]
fn valid_remind_produces_no_warning() {
    let p = parse("- TODO ship it\n  remind:: 3pm every 1h until DONE\n");
    assert!(p.warnings.is_empty());
}

#[test]
fn page_properties_only() {
    let md = "title:: foo\nstatus:: active\n";
    let p = parse(md);
    assert_eq!(
        p.properties,
        vec![
            ("title".into(), "foo".into()),
            ("status".into(), "active".into()),
        ]
    );
    assert!(p.blocks.is_empty());
}

/// A `.md` that starts with a markdown heading (the seeded
/// journal template was `# {{date}}\n\n- \n` before issue #55).
/// The parser must NOT drop content — every line becomes a
/// block — and the recovery is logged as a warning so a UI can
/// surface it.
#[test]
fn permissive_recovers_top_level_heading() {
    let md = "# 2026-06-08\n\n- real bullet\n";
    let p = parse(md);
    assert_eq!(p.blocks.len(), 2, "heading + bullet, neither dropped");
    assert_eq!(p.blocks[0].text, "# 2026-06-08");
    assert_eq!(p.blocks[1].text, "real bullet");
    assert_eq!(p.warnings.len(), 1);
    assert_eq!(p.warnings[0].line, 1);
    assert_eq!(p.warnings[0].raw, "# 2026-06-08");
    assert_eq!(
        p.warnings[0].kind,
        ParseWarningKind::UnrecognizedBlockMarker
    );
}

#[test]
fn permissive_recovers_paragraph_at_top_level() {
    // A paragraph between bullets is preserved as a block too.
    // (At depth 0 — deeper levels still belong to their owning
    // bullet via the continuation / property machinery.)
    let md = "- first\nfree paragraph\n- second\n";
    let p = parse(md);
    assert_eq!(p.blocks.len(), 3);
    assert_eq!(p.blocks[1].text, "free paragraph");
    assert_eq!(p.warnings.len(), 1);
    assert_eq!(p.warnings[0].line, 2);
}

/// Over-indented line at the top level (e.g. an imported snippet
/// pasted before its parent bullet was added). The parser used to
/// silently drop it because `line_indent > indent` triggered an
/// unconditional `continue`. Permissive contract says it now
/// surfaces as a warning + verbatim block.
#[test]
fn permissive_recovers_over_indented_top_level_line() {
    let md = "  indented orphan\n- real bullet\n";
    let p = parse(md);
    // The *content* is preserved; the leading indent is not, and that
    // is deliberate. It is the renderer's layout, so keeping it inside
    // the text means the renderer writes it after its own marker and
    // the next parse trims it back — the file would settle on the
    // second save instead of the first. Measured before the trim: 3
    // pages of 2,827 in the real workspace differed between
    // `render(parse(x))` and `render(parse(render(parse(x))))`, all of
    // them by exactly this whitespace.
    assert!(
        p.blocks.iter().any(|b| b.text == "indented orphan"),
        "indented orphan must be preserved as a block, got blocks: {:#?}",
        p.blocks,
    );
    assert!(
        p.warnings.iter().any(|w| w.line == 1),
        "warning for line 1 missing, got: {:#?}",
        p.warnings,
    );
}

/// The recovery path must preserve trailing whitespace and any
/// other significant bytes verbatim. Earlier the implementation
/// stored `stripped` instead of `raw`, so a line with trailing
/// spaces (significant in commonmark hard breaks) silently lost
/// data on the next save.
#[test]
fn permissive_recovery_preserves_trailing_whitespace() {
    // Two trailing spaces after "trailing": a CommonMark hard break.
    let md = "trailing  \n- bullet\n";
    let p = parse(md);
    assert_eq!(p.blocks[0].text, "trailing  ");
    assert_eq!(p.warnings.len(), 1);
    assert_eq!(p.warnings[0].raw, "trailing  ");
}

#[test]
fn clean_file_has_no_warnings() {
    let md = "title:: foo\n\n- a\n  - b\n- c\n";
    let p = parse(md);
    assert!(p.warnings.is_empty(), "clean dialect emits zero warnings");
}

#[test]
fn simple_outline() {
    let md = "- a\n- b\n- c\n";
    let p = parse(md);
    assert_eq!(p.blocks.len(), 3);
    assert_eq!(p.blocks[0].text, "a");
    assert_eq!(p.blocks[2].text, "c");
}

#[test]
fn nested_outline_two_levels() {
    let md = "- parent\n  - child1\n  - child2\n";
    let p = parse(md);
    assert_eq!(p.blocks.len(), 1);
    assert_eq!(p.blocks[0].text, "parent");
    assert_eq!(p.blocks[0].children.len(), 2);
    assert_eq!(p.blocks[0].children[0].text, "child1");
    assert_eq!(p.blocks[0].children[1].text, "child2");
}

#[test]
fn block_properties_then_children() {
    let md = "- objective\n  priority:: high\n  owner:: avelino\n  - subobjective\n";
    let p = parse(md);
    assert_eq!(p.blocks.len(), 1);
    let b = &p.blocks[0];
    assert_eq!(b.text, "objective");
    assert_eq!(
        b.properties,
        vec![
            ("priority".into(), "high".into()),
            ("owner".into(), "avelino".into()),
        ]
    );
    assert_eq!(b.children.len(), 1);
    assert_eq!(b.children[0].text, "subobjective");
}

/// Prose after a block property used to vanish, silently.
///
/// A `key:: value` line set `accepting_continuation = false` for the
/// rest of the block, so every following text line fell into the
/// "unrecognized — skip to avoid hang" arm and was dropped with no
/// AST entry and no warning. That contradicted this crate's stated
/// contract ("nothing is silently dropped") in the one place it
/// promises to hold, and it is how a page ends up hash-faithful
/// while its content exists in no op (issue #210).
///
/// The trigger is not exotic: outl writes `collapsed:: true` itself
/// when the user folds a block, so folding a multi-line block was
/// enough to put its body at risk on the next reconcile.
///
/// Properties are contiguous (see the grammar at the top of this
/// file), so the first non-property line resumes continuation.
#[test]
fn prose_after_a_block_property_stays_in_the_text() {
    let md = "- titulo\n  collapsed:: true\n  primeira linha\n  segunda linha\n";
    let p = parse(md);
    assert_eq!(p.blocks.len(), 1);
    let b = &p.blocks[0];
    assert_eq!(
        b.text, "titulo\nprimeira linha\nsegunda linha",
        "prose after a property belongs to the block, not the void"
    );
    assert_eq!(b.properties, vec![("collapsed".into(), "true".into())]);
    assert!(
        p.warnings.is_empty(),
        "recognized content must not warn: {:?}",
        p.warnings
    );
}

/// The interleaved form: property, prose, property, prose. Both
/// properties are collected and neither prose run is lost.
#[test]
fn prose_between_two_block_properties_survives() {
    let md = "- titulo\n  a:: 1\n  meio\n  b:: 2\n  fim\n";
    let p = parse(md);
    let b = &p.blocks[0];
    assert_eq!(b.text, "titulo\nmeio\nfim");
    assert_eq!(
        b.properties,
        vec![("a".into(), "1".into()), ("b".into(), "2".into())]
    );
}

/// The safety net behind the fix above: whatever the grammar cannot
/// place, the parser must still account for. No line may be consumed
/// without either landing in the AST or raising a warning — a line
/// that is dropped with neither is invisible to the user, to
/// `doctor`, and to the op log.
#[test]
fn a_line_the_grammar_cannot_place_is_never_dropped_in_silence() {
    // Prose after a child block already claimed the slot: continuation
    // is closed, so the line has nowhere to go. It must still be
    // reported. Same for prose after a blank line.
    for md in [
        "- titulo\n  - child\n  prose depois\n",
        "- titulo\n\n  prose apos linha vazia\n",
    ] {
        let p = parse(md);
        let text: String = p
            .blocks
            .iter()
            .flat_map(|b| {
                std::iter::once(b.text.clone()).chain(b.children.iter().map(|c| c.text.clone()))
            })
            .collect();
        let captured = text.contains("prose");
        assert!(
            captured || !p.warnings.is_empty(),
            "a consumed line must be in the AST or in warnings, never neither: {md:?}"
        );
    }
}

#[test]
fn page_props_then_blocks_with_blank() {
    let md = "title:: doc\n\n- one\n- two\n";
    let p = parse(md);
    assert_eq!(p.properties, vec![("title".into(), "doc".into())]);
    assert_eq!(p.blocks.len(), 2);
}

#[test]
fn deep_nesting() {
    let md = "- a\n  - b\n    - c\n      - d\n";
    let p = parse(md);
    assert_eq!(p.blocks[0].text, "a");
    assert_eq!(p.blocks[0].children[0].text, "b");
    assert_eq!(p.blocks[0].children[0].children[0].text, "c");
    assert_eq!(p.blocks[0].children[0].children[0].children[0].text, "d");
}

#[test]
fn empty_block_marker() {
    let md = "-\n- next\n";
    let p = parse(md);
    assert_eq!(p.blocks.len(), 2);
    assert_eq!(p.blocks[0].text, "");
    assert_eq!(p.blocks[1].text, "next");
}

#[test]
fn empty_md_yields_empty_page() {
    let p = parse("");
    assert!(p.properties.is_empty());
    assert!(p.blocks.is_empty());
}

#[test]
fn continuation_lines_join_into_block_text() {
    let md = "- first line\n  second line\n  third line\n- next block\n";
    let p = parse(md);
    assert_eq!(p.blocks.len(), 2);
    assert_eq!(p.blocks[0].text, "first line\nsecond line\nthird line");
    assert_eq!(p.blocks[1].text, "next block");
}

#[test]
fn continuation_stops_at_child_block() {
    // `  - child` is a child, not continuation.
    let md = "- header\n  continuation line\n  - child block\n";
    let p = parse(md);
    assert_eq!(p.blocks[0].text, "header\ncontinuation line");
    assert_eq!(p.blocks[0].children.len(), 1);
    assert_eq!(p.blocks[0].children[0].text, "child block");
}

#[test]
fn continuation_stops_at_property() {
    let md = "- header\n  continuation\n  priority:: high\n";
    let p = parse(md);
    assert_eq!(p.blocks[0].text, "header\ncontinuation");
    assert_eq!(
        p.blocks[0].properties,
        vec![("priority".to_string(), "high".to_string())]
    );
}

#[test]
fn blank_line_terminates_continuation() {
    // After the blank line, `still text` is unrecognized (not
    // continuation, not a child block) and gets skipped.
    let md = "- header\n  continuation\n\n  still text\n- next\n";
    let p = parse(md);
    assert_eq!(p.blocks[0].text, "header\ncontinuation");
    assert_eq!(p.blocks.len(), 2);
    assert_eq!(p.blocks[1].text, "next");
}
