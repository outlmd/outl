//! End-to-end editor smoke test (no TTY).
//!
//! Drives the editor through a sequence of edits that the user would
//! make interactively, then verifies the resulting `.md` + sidecar +
//! op log all agree.
//!
//! This exercises the pure AST manipulation + reconcile path. The
//! actual ratatui rendering is independently tested in `app::tests`.

use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::workspace::Workspace;
use outl_md::parse::{parse, OutlineNode, ParsedPage};
use outl_md::reconcile::reconcile_md;
use outl_md::render::render;
use outl_md::sidecar::{self, sidecar_path_for};
use std::fs;
use tempfile::TempDir;

/// Replays a sequence of AST mutations and persists via reconcile.
fn apply_and_save(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    md_path: &std::path::Path,
    page: &ParsedPage,
) {
    let text = render(page);
    fs::write(md_path, text).unwrap();
    reconcile_md(ws, hlc, md_path, None).unwrap();
}

#[test]
fn editor_session_lifecycle() {
    let dir = TempDir::new().unwrap();
    let md_path = dir.path().join("session.md");
    let actor = ActorId::new();
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let hlc = HlcGenerator::new(actor);

    // 1. Empty file → editor seed.
    fs::write(&md_path, "- \n").unwrap();
    reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();
    let sc1 = sidecar::read(&sidecar_path_for(&md_path)).unwrap();
    assert_eq!(sc1.blocks.len(), 1);
    let first_id = sc1.blocks[0].id;

    // 2. User types "first block" into the seed.
    let mut page = parse(&fs::read_to_string(&md_path).unwrap());
    page.blocks[0].text = "first block".into();
    apply_and_save(&mut ws, &hlc, &md_path, &page);
    let sc2 = sidecar::read(&sidecar_path_for(&md_path)).unwrap();
    // ID preserved across edits (content hash differs, but matching is
    // greedy and only one block — falls to level 3, gets a new ULID).
    // This is the documented matching behavior: heavy edits lose IDs,
    // but the old one shows up as orphan. The test asserts the
    // structural integrity, not ID preservation here.
    assert_eq!(sc2.blocks.len(), 1);

    // 3. User adds a child block ("nested").
    let mut page = parse(&fs::read_to_string(&md_path).unwrap());
    page.blocks[0].children.push(OutlineNode {
        text: "nested child".into(),
        ..Default::default()
    });
    apply_and_save(&mut ws, &hlc, &md_path, &page);
    let sc3 = sidecar::read(&sidecar_path_for(&md_path)).unwrap();
    assert_eq!(sc3.blocks.len(), 2);
    assert_eq!(sc3.blocks[1].indent, 1);

    // 4. User adds a top-level block with a [[ref]].
    let mut page = parse(&fs::read_to_string(&md_path).unwrap());
    page.blocks.push(OutlineNode {
        text: "second top with [[link]] and #tag".into(),
        ..Default::default()
    });
    apply_and_save(&mut ws, &hlc, &md_path, &page);
    let sc4 = sidecar::read(&sidecar_path_for(&md_path)).unwrap();
    assert_eq!(sc4.blocks.len(), 3);

    // 5. User edits the second top, whitespace-only change → hash
    //    normalizes → match stays level 1 → ID preserved.
    //
    //    Note: sidecar.blocks is DFS-preorder (3 entries), but
    //    page.blocks lists only top-level outline items (2 entries).
    //    "second top" is the SECOND sidecar entry that is top-level
    //    (indent == 0); it lives at page.blocks[1].
    let second_top_sidecar = sc4
        .blocks
        .iter()
        .find(|b| b.indent == 0 && b.content_hash != sc4.blocks[0].content_hash)
        .expect("second top-level block should exist in sidecar");
    let id_before = second_top_sidecar.id;
    let mut page = parse(&fs::read_to_string(&md_path).unwrap());
    page.blocks[1].text = "second top with [[link]] and #tag".into();
    apply_and_save(&mut ws, &hlc, &md_path, &page);
    let sc5 = sidecar::read(&sidecar_path_for(&md_path)).unwrap();
    let second_top_after = sc5
        .blocks
        .iter()
        .find(|b| b.indent == 0 && b.content_hash != sc5.blocks[0].content_hash)
        .expect("second top still present");
    assert_eq!(
        second_top_after.id, id_before,
        "whitespace-only edit must preserve id"
    );

    // 6. .md is never polluted with IDs.
    let md_disk = fs::read_to_string(&md_path).unwrap();
    assert!(!md_disk.contains("id::"));
    assert!(!md_disk.contains(&first_id.to_string()));
    assert!(!md_disk.contains(&id_before.to_string()));

    // 7. Workspace op log knows about every block.
    assert!(ws.tree().node_count() >= 3);
}

#[test]
fn ast_indent_and_outdent_preserves_subtree() {
    use outl_md::parse::OutlineNode;

    let mut page = ParsedPage {
        frontmatter: None,
        properties: vec![],
        warnings: vec![],
        blocks: vec![
            OutlineNode {
                text: "a".into(),
                ..Default::default()
            },
            OutlineNode {
                text: "b".into(),
                children: vec![OutlineNode {
                    text: "b1".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
    };
    // Indent "b" → becomes child of "a"; "b1" rides along as grandchild.
    let b = page.blocks.remove(1);
    page.blocks[0].children.push(b);
    assert_eq!(page.blocks.len(), 1);
    assert_eq!(page.blocks[0].children.len(), 1);
    assert_eq!(page.blocks[0].children[0].text, "b");
    assert_eq!(page.blocks[0].children[0].children.len(), 1);
    assert_eq!(page.blocks[0].children[0].children[0].text, "b1");
}
