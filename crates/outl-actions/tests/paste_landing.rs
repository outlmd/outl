//! Where a paste lands, and what a paste that has nothing to say writes.
//!
//! `PasteOutcome::last_root` is the block a client selects after a paste
//! and anchors the **next** paste on. It has to be the last root-level
//! block, not the last id minted: those are DFS, so a payload ending in
//! a block with children would put the cursor on a child, and `p p`
//! would nest the second copy inside the first.

use outl_actions::{append_block, children_of, paste_markdown, paste_plain, PasteAnchor};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;

fn ws() -> (Workspace, HlcGenerator) {
    let actor = ActorId::new();
    (
        Workspace::open_in_memory(actor).expect("in-memory workspace"),
        HlcGenerator::new(actor),
    )
}

fn texts(ws: &Workspace, parent: NodeId) -> Vec<String> {
    children_of(ws, parent)
        .into_iter()
        .map(|(id, _)| ws.block_text(id).unwrap_or_default())
        .collect()
}

const NESTED: &str = "- one\n- two\n  - two.a\n    - two.a.i\n";

#[test]
fn pasting_a_nested_payload_twice_lands_the_second_copy_as_a_sibling() {
    let (mut ws, hlc) = ws();
    let host = append_block(&mut ws, &hlc, None, Some("host")).expect("host");
    let first = append_block(&mut ws, &hlc, Some(host), Some("first")).expect("first");

    let out = paste_markdown(&mut ws, &hlc, PasteAnchor::AfterBlock(first), NESTED).expect("paste");
    let landed = out
        .last_root
        .expect("a paste that created blocks has a landing");
    assert_eq!(ws.block_text(landed).as_deref(), Some("two"));
    assert_ne!(
        out.new_blocks.last().copied(),
        Some(landed),
        "the fixture must end on a nested block, or this pins nothing"
    );

    paste_markdown(&mut ws, &hlc, PasteAnchor::AfterBlock(landed), NESTED).expect("again");
    assert_eq!(texts(&ws, host), ["first", "one", "two", "one", "two"]);
}

#[test]
fn as_last_child_lands_on_the_last_root() {
    let (mut ws, hlc) = ws();
    let host = append_block(&mut ws, &hlc, None, Some("host")).expect("host");
    let out =
        paste_markdown(&mut ws, &hlc, PasteAnchor::AsLastChildOf(host), NESTED).expect("paste");
    let landed = out.last_root.expect("landing");
    assert_eq!(ws.block_text(landed).as_deref(), Some("two"));
    assert_eq!(ws.tree().parent(landed), Some(host));
}

#[test]
fn plain_text_lands_on_the_one_block_it_created() {
    let (mut ws, hlc) = ws();
    let host = append_block(&mut ws, &hlc, None, Some("host")).expect("host");
    let out = paste_markdown(&mut ws, &hlc, PasteAnchor::AsLastChildOf(host), "just text")
        .expect("paste");
    assert_eq!(out.new_blocks.len(), 1);
    assert_eq!(out.last_root, out.new_blocks.first().copied());
}

#[test]
fn a_caret_splice_creates_nothing_to_land_on() {
    let (mut ws, hlc) = ws();
    let host = append_block(&mut ws, &hlc, None, Some("host")).expect("host");
    let out = paste_markdown(
        &mut ws,
        &hlc,
        PasteAnchor::AtCaret {
            block: host,
            caret: 4,
        },
        " text",
    )
    .expect("paste");
    assert_eq!(out.last_root, None);
}

/// A caret paste that splits off siblings lands on the last of them,
/// which is the host's own tail when there is one.
#[test]
fn a_caret_outline_paste_lands_on_the_tail() {
    let (mut ws, hlc) = ws();
    let host = append_block(&mut ws, &hlc, None, Some("left right")).expect("host");
    let out = paste_markdown(
        &mut ws,
        &hlc,
        PasteAnchor::AtCaret {
            block: host,
            caret: 5,
        },
        "- a\n- b",
    )
    .expect("paste");
    let landed = out.last_root.expect("landing");
    assert_eq!(ws.block_text(landed).as_deref(), Some("right"));
}

/// `foo\n` from a terminal, or a Windows clipboard's `\r\n`, used to
/// become a block whose text ended in a newline: something the `.md`
/// cannot hold, so the log and the file disagreed from the first write.
#[test]
fn a_new_plain_block_has_no_trailing_newline_or_carriage_return() {
    // `foo\n  ` is a terminal copy padding its last line.
    for payload in [
        "foo\n",
        "foo\r\n",
        "foo\n\n\n",
        "foo\r",
        "foo\n  ",
        "foo\n \n\t",
    ] {
        let (mut ws, hlc) = ws();
        let host = append_block(&mut ws, &hlc, None, Some("host")).expect("host");
        paste_plain(&mut ws, &hlc, PasteAnchor::AsLastChildOf(host), payload).expect("plain");
        paste_markdown(&mut ws, &hlc, PasteAnchor::AsLastChildOf(host), payload).expect("md");
        assert_eq!(texts(&ws, host), ["foo", "foo"], "payload {payload:?}");
    }
}

#[test]
fn a_whitespace_only_paste_writes_no_op() {
    for payload in ["", "   ", "\n\n", " \r\n\t\n"] {
        let (mut ws, hlc) = ws();
        let host = append_block(&mut ws, &hlc, None, Some("host")).expect("host");
        let before = ws.log().len();
        for anchor in [
            PasteAnchor::AsLastChildOf(host),
            PasteAnchor::AfterBlock(host),
        ] {
            let out = paste_plain(&mut ws, &hlc, anchor.clone(), payload).expect("plain");
            assert_eq!(out.last_root, None, "payload {payload:?}");
            paste_markdown(&mut ws, &hlc, anchor, payload).expect("md");
        }
        assert_eq!(ws.log().len(), before, "payload {payload:?} wrote an op");
    }
}

/// Two outl blocks whose text holds a tab are an outline: the tabular
/// conversion would otherwise see "two lines, same field count" and turn
/// the user's own copy into a table.
#[test]
fn bullets_carrying_tabs_stay_an_outline() {
    let (mut ws, hlc) = ws();
    let host = append_block(&mut ws, &hlc, None, Some("host")).expect("host");
    let out = paste_markdown(
        &mut ws,
        &hlc,
        PasteAnchor::AsLastChildOf(host),
        "- a\tb\n- c\td\n",
    )
    .expect("paste");
    assert_eq!(out.root_count, 2);
    assert_eq!(texts(&ws, host), ["a\tb", "c\td"]);
}

/// The round trip the fence pass exists for: outl's own copy of a code
/// block comes back byte-identical.
#[test]
fn a_copied_code_fence_pastes_back_unchanged() {
    let (mut ws, hlc) = ws();
    let host = append_block(&mut ws, &hlc, None, Some("host")).expect("host");
    let code = "```jinja\n{{ x }}\n- [ ] not a task\n```";
    let src = append_block(&mut ws, &hlc, Some(host), Some(code)).expect("code block");
    let copied = outl_actions::copy_markdown(&ws, &[src]);

    let out = paste_markdown(&mut ws, &hlc, PasteAnchor::AfterBlock(src), &copied).expect("paste");
    let landed = out.last_root.expect("landing");
    assert_eq!(ws.block_text(landed).as_deref(), Some(code));
}
