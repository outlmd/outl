//! The projection side of [issue #281]: a page whose `.md` opens with a
//! YAML frontmatter fence.
//!
//! The parser half is pinned in `outl-md/tests/frontmatter_roundtrip.rs`.
//! This is the half the issue actually reported, because the fence was not
//! lost when it was read — it was lost when something wrote the page back.
//!
//! [issue #281]: https://github.com/outlmd/outl/issues/281

use super::*;
use crate::ActionError;

/// The file from the issue report, verbatim.
const REPORTED: &str = "---\ntitle: My Note\ntags: [a, b]\n---\n\n- body\n";

/// The whole reported sequence: reconcile the file, append a block, project
/// the page back. Four commands in the issue, three calls here.
#[test]
fn a_block_append_leaves_the_frontmatter_fence_intact() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    let md_path = tmp.path().join("pages").join("fm.md");
    std::fs::create_dir_all(md_path.parent().unwrap()).unwrap();
    std::fs::write(&md_path, REPORTED).unwrap();
    outl_md::reconcile::reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();

    let page = NodeId::from_slug("fm");
    append_block(&mut ws, &hlc, Some(page), Some("novo bloco")).unwrap();
    apply_page_md_with_sidecar_guarded(&ws, tmp.path(), page).unwrap();

    let disk = std::fs::read_to_string(&md_path).unwrap();
    assert_eq!(
        disk, "---\ntitle: My Note\ntags: [a, b]\n---\n\n- body\n- novo bloco\n",
        "the append rewrote the frontmatter"
    );
    assert!(
        !disk.contains("- ---"),
        "a fence delimiter came back as a bullet: {disk:?}"
    );
}

/// A page whose op log holds the fence **as bullets** — what an older
/// parser produced — must be refused, not overwritten.
///
/// This is the window between upgrading and that page's first reconcile:
/// the sidecar's hash matches the file, every disk line is accounted for by
/// some block, and the only thing wrong is that the render carries no
/// fence. The block-level guard is silent here by construction, so without
/// the frontmatter channel's own verdict this write goes through and the
/// reported rewrite happens once more — unattended, on `outl serve`'s
/// sweep.
#[test]
fn if_stale_refuses_a_projection_that_would_drop_the_frontmatter() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let page = open_or_create(&mut ws, &hlc, "fm", "Fm", PageKind::Page).unwrap();
    // The old parser's reading of REPORTED: five blocks, one of them `---`.
    for text in ["---", "title: My Note", "tags: [a, b]", "---", "body"] {
        append_block(&mut ws, &hlc, Some(page), Some(text)).unwrap();
    }
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
    let md_path = page_md_path(tmp.path(), &page_meta(&ws, page).unwrap());

    // The user's file still holds the real fence, and the sidecar vouches
    // for those bytes — exactly the state `serve --once` left behind.
    std::fs::write(&md_path, REPORTED).unwrap();
    restamp_sidecar_as_faithful(&md_path);

    let err = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page)
        .expect_err("a projection that drops the fence must be refused");
    assert!(
        matches!(err, ActionError::PageMarkdownAheadOfLog { .. }),
        "the refusal must name the recovery the user can run: {err:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&md_path).unwrap(),
        REPORTED,
        "the file must be untouched by a refused projection"
    );
}

/// And the ordinary case is not refused.
///
/// A false positive freezes the page in both directions, so a page whose
/// log *does* know its fence has to project like any other.
#[test]
fn if_stale_still_projects_a_page_whose_log_knows_its_fence() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    let md_path = tmp.path().join("pages").join("fm.md");
    std::fs::create_dir_all(md_path.parent().unwrap()).unwrap();
    std::fs::write(&md_path, REPORTED).unwrap();
    outl_md::reconcile::reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();

    // A peer's block lands in the tree; nothing has re-projected the `.md`.
    let page = NodeId::from_slug("fm");
    append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page)
        .expect("a page whose log holds its fence must project");
    assert!(
        wrote.is_some(),
        "the tree ran ahead and must be re-projected"
    );
    let disk = std::fs::read_to_string(&md_path).unwrap();
    assert!(
        disk.starts_with("---\ntitle: My Note\ntags: [a, b]\n---\n"),
        "the fence must survive the re-projection: {disk:?}"
    );
    assert!(disk.contains("- synced-in"), "{disk:?}");
}

/// The projection lifts the fence out of the page's properties **before**
/// the page-model keys are filtered out of them, so the fence surviving is
/// not a side effect of the filter's order.
///
/// `PAGE_FRONTMATTER_KEY` is in `tree::is_page_model_key` (it must be
/// hidden from the property panel, the suggestion menu and the clipboard),
/// which means a `retain` running first would delete the fence from every
/// projection — the exact write issue #281 reported. This is the test that
/// fails if the two steps swap back.
#[test]
fn the_render_lifts_the_fence_before_hiding_the_page_model_keys() {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let page = open_or_create(&mut ws, &hlc, "fm", "Fm", PageKind::Page).unwrap();
    crate::page::set_property(
        &mut ws,
        &hlc,
        page,
        outl_md::PAGE_FRONTMATTER_KEY,
        Some(outl_core::property::PropValue::Text(
            "title: My Note\ntags: [a, b]".into(),
        )),
    )
    .unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("body")).unwrap();

    let md = render_page_md(&ws, page);
    assert!(
        md.starts_with("---\ntitle: My Note\ntags: [a, b]\n---\n"),
        "the fence did not survive the projection: {md:?}"
    );
    assert!(
        !md.contains("page-frontmatter"),
        "the fence came back as a `key:: value` line: {md:?}"
    );
    assert!(
        !md.contains("page-slug") && !md.contains("page-kind"),
        "a page-model key reached the `.md`: {md:?}"
    );
    assert!(md.contains("- body"), "{md:?}");
}

/// [`REPORTED`] as a Windows editor writes it: a UTF-8 BOM in front of the
/// opening delimiter.
const BOM_REPORTED: &str = "\u{feff}---\ntitle: My Note\ntags: [a, b]\n---\n\n- body\n";

/// A vault page written on Windows must project like any other.
///
/// The false-positive direction, and the expensive one. `parse` drops a
/// BOM before splitting, so the log holds one block plus the fence as a
/// page property — nothing is at risk. But `frontmatter_line_count` could
/// not see a fence behind a BOM, so the block-level guard skipped no
/// leading lines and reported all four of them as content the log never
/// saw. Every write was refused and the page froze in both directions,
/// permanently: the projection that would have dropped the BOM is the same
/// projection being refused, so the state never changes on its own.
#[test]
fn if_stale_still_projects_a_page_whose_fence_sits_behind_a_byte_order_mark() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    let md_path = tmp.path().join("pages").join("fm.md");
    std::fs::create_dir_all(md_path.parent().unwrap()).unwrap();
    std::fs::write(&md_path, BOM_REPORTED).unwrap();
    outl_md::reconcile::reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();

    // A peer's block lands in the tree; nothing has re-projected the `.md`.
    let page = NodeId::from_slug("fm");
    append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page)
        .expect("a BOM is an encoding artifact, not content the log lacks");
    assert!(
        wrote.is_some(),
        "the tree ran ahead and must be re-projected"
    );
    let disk = std::fs::read_to_string(&md_path).unwrap();
    assert!(
        disk.starts_with("---\ntitle: My Note\ntags: [a, b]\n---\n"),
        "the fence must survive the re-projection: {disk:?}"
    );
    assert!(disk.contains("- synced-in"), "{disk:?}");
}

/// The write guard still refuses when the log holds the fence as bullets,
/// even though the file it is defending opens with a BOM.
///
/// The mirror of the test above and the destructive half: `render` carries
/// no fence, so the write would delete four lines of the user's metadata.
/// `frontmatter_lines_missing_from` is the only channel that can say so,
/// and it answered `0` for every BOM'd file — blind in precisely the class
/// of file where the #281 rewrite happens.
#[test]
fn if_stale_refuses_to_drop_a_frontmatter_fence_that_sits_behind_a_bom() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let page = open_or_create(&mut ws, &hlc, "fm", "Fm", PageKind::Page).unwrap();
    // The old parser's reading of the file: five blocks, two of them `---`.
    for text in ["---", "title: My Note", "tags: [a, b]", "---", "body"] {
        append_block(&mut ws, &hlc, Some(page), Some(text)).unwrap();
    }
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
    let md_path = page_md_path(tmp.path(), &page_meta(&ws, page).unwrap());

    std::fs::write(&md_path, BOM_REPORTED).unwrap();
    restamp_sidecar_as_faithful(&md_path);

    let err = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page)
        .expect_err("a projection that drops the fence must be refused");
    assert!(
        matches!(err, ActionError::PageMarkdownAheadOfLog { .. }),
        "the refusal must name the recovery the user can run: {err:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&md_path).unwrap(),
        BOM_REPORTED,
        "the file must be untouched by a refused projection"
    );
}
