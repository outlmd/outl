//! The paste pipeline, end to end.
//!
//! Every case drives [`super::paste_markdown`] or [`super::paste_plain`]
//! against a real workspace, because the thing worth pinning is what
//! lands in the tree — the split between detection, normalisation and
//! grafting is an implementation detail these tests deliberately do not
//! know about.

use super::*;
use crate::block::append_block;
use outl_core::id::ActorId;

fn ws() -> (Workspace, HlcGenerator) {
    let actor = ActorId::new();
    (
        Workspace::open_in_memory(actor).unwrap(),
        HlcGenerator::new(actor),
    )
}

#[test]
fn paste_as_last_child_of_root() {
    let (mut workspace, hlc) = ws();
    let parent = append_block(&mut workspace, &hlc, None, Some("host")).unwrap();
    let out = paste_markdown(
        &mut workspace,
        &hlc,
        PasteAnchor::AsLastChildOf(parent),
        "- one\n- two\n- three",
    )
    .unwrap();
    assert_eq!(out.root_count, 3);
    let kids: Vec<String> = crate::tree::children_of(&workspace, parent)
        .into_iter()
        .map(|(id, _)| workspace.block_text(id).unwrap_or_default())
        .collect();
    assert_eq!(kids, vec!["one", "two", "three"]);
}

#[test]
fn paste_after_root_block() {
    let (mut workspace, hlc) = ws();
    let a = append_block(&mut workspace, &hlc, None, Some("a")).unwrap();
    let _z = append_block(&mut workspace, &hlc, None, Some("z")).unwrap();
    let _ = paste_markdown(
        &mut workspace,
        &hlc,
        PasteAnchor::AfterBlock(a),
        "- one\n- two",
    )
    .unwrap();
    let order: Vec<String> = crate::tree::children_of(&workspace, NodeId::root())
        .into_iter()
        .map(|(id, _)| workspace.block_text(id).unwrap_or_default())
        .collect();
    assert_eq!(order, vec!["a", "one", "two", "z"]);
}

#[test]
fn paste_at_caret_splits_and_appends_tail() {
    let (mut workspace, hlc) = ws();
    let host = append_block(&mut workspace, &hlc, None, Some("olá mundo")).unwrap();
    // caret = 4 → after "olá " (4 chars including the space).
    let out = paste_markdown(
        &mut workspace,
        &hlc,
        PasteAnchor::AtCaret {
            block: host,
            caret: 4,
        },
        "- um\n- dois",
    )
    .unwrap();
    assert_eq!(workspace.block_text(host).as_deref(), Some("olá um"));
    let order: Vec<String> = crate::tree::children_of(&workspace, NodeId::root())
        .into_iter()
        .map(|(id, _)| workspace.block_text(id).unwrap_or_default())
        .collect();
    assert_eq!(order, vec!["olá um", "dois", "mundo"]);
    assert_eq!(out.host_text.as_deref(), Some("olá um"));
    // 2 new sibling blocks created ("dois", "mundo"). "um" was
    // merged into the host so it isn't in new_blocks.
    assert_eq!(out.new_blocks.len(), 2);
}

#[test]
fn paste_plain_text_preserves_unknown_tokens() {
    // Pasting "{{video: ...}}" into a block as plain text must
    // NOT strip the token — only outline-shaped pastes go through
    // the normaliser. The user copied that string for a reason
    // and rewriting it silently is data loss.
    let (mut workspace, hlc) = ws();
    // `append_block` trims the seed text, so the host lands as
    // "watch" (5 chars). Paste at the very end with a leading
    // space inside the clipboard payload to verify the literal
    // splice path keeps every byte of the user's text.
    let host = append_block(&mut workspace, &hlc, None, Some("watch")).unwrap();
    let out = paste_markdown(
        &mut workspace,
        &hlc,
        PasteAnchor::AtCaret {
            block: host,
            caret: 5,
        },
        " {{video: https://x.test}} now",
    )
    .unwrap();
    assert_eq!(
        workspace.block_text(host).as_deref(),
        Some("watch {{video: https://x.test}} now"),
    );
    assert!(out.new_blocks.is_empty());
}

#[test]
fn paste_at_caret_with_plain_text_is_a_splice() {
    let (mut workspace, hlc) = ws();
    let host = append_block(&mut workspace, &hlc, None, Some("hello world")).unwrap();
    let out = paste_markdown(
        &mut workspace,
        &hlc,
        PasteAnchor::AtCaret {
            block: host,
            caret: 6,
        },
        "BRAVE ",
    )
    .unwrap();
    assert_eq!(
        workspace.block_text(host).as_deref(),
        Some("hello BRAVE world")
    );
    assert!(out.new_blocks.is_empty());
}

#[test]
fn paste_empty_input_is_noop() {
    let (mut workspace, hlc) = ws();
    let host = append_block(&mut workspace, &hlc, None, Some("hi")).unwrap();
    let out = paste_markdown(
        &mut workspace,
        &hlc,
        PasteAnchor::AtCaret {
            block: host,
            caret: 2,
        },
        "",
    )
    .unwrap();
    assert!(out.new_blocks.is_empty());
    // Splice of empty into "hi" is still "hi".
    assert_eq!(workspace.block_text(host).as_deref(), Some("hi"));
}

#[test]
fn paste_preserves_nested_children() {
    let (mut workspace, hlc) = ws();
    let parent = append_block(&mut workspace, &hlc, None, Some("p")).unwrap();
    let _ = paste_markdown(
        &mut workspace,
        &hlc,
        PasteAnchor::AsLastChildOf(parent),
        "- a\n  - a1\n  - a2\n- b",
    )
    .unwrap();
    let kids: Vec<(String, Vec<String>)> = crate::tree::children_of(&workspace, parent)
        .into_iter()
        .map(|(id, _)| {
            let grand: Vec<String> = crate::tree::children_of(&workspace, id)
                .into_iter()
                .map(|(gid, _)| workspace.block_text(gid).unwrap_or_default())
                .collect();
            (workspace.block_text(id).unwrap_or_default(), grand)
        })
        .collect();
    assert_eq!(
        kids,
        vec![
            ("a".to_string(), vec!["a1".to_string(), "a2".to_string()]),
            ("b".to_string(), Vec::new()),
        ]
    );
}

#[test]
fn paste_applies_block_properties() {
    use outl_core::property::PropValue;
    let (mut workspace, hlc) = ws();
    let parent = append_block(&mut workspace, &hlc, None, Some("p")).unwrap();
    let _ = paste_markdown(
        &mut workspace,
        &hlc,
        PasteAnchor::AsLastChildOf(parent),
        "- header\n  priority:: high\n  - child",
    )
    .unwrap();
    let kids = crate::tree::children_of(&workspace, parent);
    assert_eq!(kids.len(), 1);
    let header_id = kids[0].0;
    let prop = workspace.tree().property(header_id, "priority");
    match prop {
        Some(PropValue::Text(v)) => assert_eq!(v, "high"),
        other => panic!("expected Text(\"high\"), got {other:?}"),
    }
}

#[test]
fn paste_user_prompt_fixture() {
    // The literal markdown the user pasted in the prompt that
    // motivated this feature. The exact same string must produce
    // the expected tree on both clients.
    let raw = "- #LinkedIn #hot-take. Draft\n    - **Tema:** Can the stockmarket swallow Anthropic, SpaceX and OpenAI? ([hackernews](https://www.economist.com/finance-and-economics/2026/06/01/can-the-stockmarket-swallow-anthropic-spacex-and-openai))\n    - **Score:** 368 pontos, 641 comentários (hackernews)\n    - No mesmo dia, The Economist pergunta se o mercado público consegue engolir Anthropic, OpenAI e SpaceX juntas, Alphabet anuncia equity raise de $80 bi pra AI infra, e Groq abre rodada nova antes da última fechar.\n    - As três privadas somam mais de $1 trilhão em valuation. Capex de AI cresce mais rápido que revenue de qualquer um dos players.\n    - {{[[TODO]]}} revisar antes de postar\n";

    let (mut workspace, hlc) = ws();
    let _ = paste_markdown(
        &mut workspace,
        &hlc,
        PasteAnchor::AsLastChildOf(NodeId::root()),
        raw,
    )
    .unwrap();

    // Exactly one root block — the LinkedIn draft header.
    let roots = crate::tree::children_of(&workspace, NodeId::root());
    assert_eq!(roots.len(), 1, "expected 1 root block, got {}", roots.len());
    let header_id = roots[0].0;
    assert_eq!(
        workspace.block_text(header_id).as_deref(),
        Some("#LinkedIn #hot-take. Draft")
    );

    // Five children, the last one converted from {{[[TODO]]}}.
    let kids = crate::tree::children_of(&workspace, header_id);
    assert_eq!(kids.len(), 5);
    let last_text = workspace.block_text(kids[4].0).unwrap_or_default();
    assert_eq!(last_text, "TODO revisar antes de postar");
}

#[test]
fn paste_after_a_freshly_created_empty_block() {
    // The `o`/new-line block (Op::Create only, no text) as an
    // AfterBlock anchor: siblings graft after it, host stays put.
    let (mut workspace, hlc) = ws();
    let empty = append_block(&mut workspace, &hlc, None, None).unwrap();
    let out = paste_markdown(
        &mut workspace,
        &hlc,
        PasteAnchor::AfterBlock(empty),
        "- one\n- two",
    )
    .unwrap();
    assert_eq!(out.root_count, 2);
    let order: Vec<String> = crate::tree::children_of(&workspace, NodeId::root())
        .into_iter()
        .map(|(id, _)| workspace.block_text(id).unwrap_or_default())
        .collect();
    assert_eq!(order, vec!["".to_string(), "one".into(), "two".into()]);
}

#[test]
fn paste_as_child_of_a_freshly_created_empty_block() {
    // Empty host as an AsLastChildOf anchor: children graft under it.
    let (mut workspace, hlc) = ws();
    let empty = append_block(&mut workspace, &hlc, None, None).unwrap();
    paste_markdown(
        &mut workspace,
        &hlc,
        PasteAnchor::AsLastChildOf(empty),
        "- one\n- two",
    )
    .unwrap();
    let kids: Vec<String> = crate::tree::children_of(&workspace, empty)
        .into_iter()
        .map(|(id, _)| workspace.block_text(id).unwrap_or_default())
        .collect();
    assert_eq!(kids, vec!["one".to_string(), "two".into()]);
}

#[test]
fn plain_multi_line_pastes_one_block_each() {
    // The desktop paste report: a chat reply (no bullets, lines
    // separated by single `\n`) must land as one block per line, not
    // a single wall-of-text block.
    let (mut workspace, hlc) = ws();
    let host = append_block(&mut workspace, &hlc, None, Some("host")).unwrap();
    let raw = "First line.\nSecond line.\n\nThird line.";
    let out = paste_markdown(&mut workspace, &hlc, PasteAnchor::AsLastChildOf(host), raw).unwrap();
    assert_eq!(out.root_count, 3);
    let kids: Vec<String> = crate::tree::children_of(&workspace, host)
        .into_iter()
        .map(|(id, _)| workspace.block_text(id).unwrap_or_default())
        .collect();
    assert_eq!(
        kids,
        vec![
            "First line.".to_string(),
            "Second line.".to_string(),
            "Third line.".to_string(),
        ]
    );
}

#[test]
fn plain_single_line_stays_one_block() {
    // One line (a URL, a sentence) is not fragmented.
    let (mut workspace, hlc) = ws();
    let host = append_block(&mut workspace, &hlc, None, Some("host")).unwrap();
    let raw = "https://example.com/just-one-line";
    paste_markdown(&mut workspace, &hlc, PasteAnchor::AsLastChildOf(host), raw).unwrap();
    let kids = crate::tree::children_of(&workspace, host);
    assert_eq!(kids.len(), 1);
    assert_eq!(workspace.block_text(kids[0].0).as_deref(), Some(raw));
}

#[test]
fn paste_into_freshly_created_empty_block() {
    // The `o` / new-line block: created with no text, so it emits
    // only `Op::Create` and `block_text` returns None even though the
    // block IS in the tree. Pasting into it must not fail with
    // "block <id> is not in the tree" — the earlier code guarded
    // existence on `block_text` and rejected the empty host.
    let (mut workspace, hlc) = ws();
    // `append_block(text=None)` is exactly what `create_block`/`o` does.
    let empty = append_block(&mut workspace, &hlc, None, None).unwrap();
    assert_eq!(
        workspace.block_text(empty),
        None,
        "precondition: a create-only block has no materialized text"
    );

    // Paste with formatting (outline) at the caret of the empty host.
    let out = paste_markdown(
        &mut workspace,
        &hlc,
        PasteAnchor::AtCaret {
            block: empty,
            caret: 0,
        },
        "- one\n- two",
    )
    .expect("paste into an empty new block must not be NotInTree");
    assert_eq!(workspace.block_text(empty).as_deref(), Some("one"));
    assert_eq!(out.host_text.as_deref(), Some("one"));

    // Plain-text paste at the caret of an empty host (Cmd+Shift+V).
    let empty2 = append_block(&mut workspace, &hlc, None, None).unwrap();
    paste_plain(
        &mut workspace,
        &hlc,
        PasteAnchor::AtCaret {
            block: empty2,
            caret: 0,
        },
        "raw text",
    )
    .expect("plain paste into an empty new block must not be NotInTree");
    assert_eq!(workspace.block_text(empty2).as_deref(), Some("raw text"));
}

#[test]
fn paste_plain_never_splits_or_converts() {
    // The "without formatting" path: outline-looking text lands
    // verbatim, no conversion, no splitting.
    let (mut workspace, hlc) = ws();
    let host = append_block(&mut workspace, &hlc, None, Some("host")).unwrap();
    let raw = "- [ ] task\n\n- [ ] another";
    paste_plain(&mut workspace, &hlc, PasteAnchor::AsLastChildOf(host), raw).unwrap();
    let kids = crate::tree::children_of(&workspace, host);
    assert_eq!(kids.len(), 1, "plain paste is one block, never split");
    assert_eq!(workspace.block_text(kids[0].0).as_deref(), Some(raw));
}
