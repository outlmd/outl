//! A sidecar that describes a different page than the one being read.
//!
//! `read_page_outline` zips the parsed `.md` against `sidecar.blocks`
//! positionally and, until this check, never compared `sidecar.page_id`
//! with the page it was handed. On the maintainer's workspace that is
//! true of 1,775 sidecars: a Roam re-import on 2026-11-12 moved the
//! previous generation of the graph to the trash and nothing
//! re-projected the `.md` / `.outl` pairs, so those files describe a
//! page root the tree no longer holds under `NodeId::root()`.
//! Measurements and the staged repair this is pulled ahead of:
//! `docs/superpowers/specs/2026-11-20-stale-projection-repair-design.md`.
//!
//! **Half of these tests exist for the direction nobody reported.** The
//! check is on the page-open path of every client, so the tempting fix
//! — refuse the read — trades a stale page for no page at all, and
//! `build_backlink_index_from_disk` skips a page whose read errors, so
//! it would also take 1,775 pages' `[[ref]]`s out of the backlinks
//! panel the mismatch was reported *from*. That is the deleting half of
//! root `CLAUDE.md` invariant 8's mirrored pair, arrived at by fixing
//! the reported half. The three states a page with nothing on disk
//! takes are asserted rather than reasoned about, because refusing on a
//! predicate narrower than the question warranted is what froze every
//! empty page a peer wrote into ([#332]).
//!
//! [#332]: https://github.com/outlmd/outl/issues/332

use std::path::{Path, PathBuf};

use outl_actions::journal::page_md_path;
use outl_actions::page::{open_or_create, page_meta, PageKind, PageMeta};
use outl_actions::{append_block, read_page_outline, SidecarIdentity};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use tempfile::TempDir;

fn workspace() -> (Workspace, HlcGenerator) {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let ws = Workspace::open_in_memory(actor).expect("in-memory workspace");
    (ws, hlc)
}

/// A page with two blocks, one of them citing another page, projected to
/// `.md` + sidecar the ordinary way.
fn projected_page(tmp: &TempDir, slug: &str) -> (Workspace, PageMeta, PathBuf) {
    let (mut ws, hlc) = workspace();
    let page = open_or_create(&mut ws, &hlc, slug, slug, PageKind::Page).expect("create page");
    append_block(&mut ws, &hlc, Some(page), Some("cites [[elsewhere]]")).expect("first block");
    append_block(&mut ws, &hlc, Some(page), Some("and a second block")).expect("second block");
    let meta = page_meta(&ws, page).expect("page meta");
    outl_actions::apply_page_md_with_sidecar(&ws, tmp.path(), page).expect("project the page");
    let md = page_md_path(tmp.path(), &meta);
    (ws, meta, md)
}

/// Repoint the page's sidecar at a root the tree does not hold there,
/// leaving the block entries untouched. Returns the root it now claims.
fn repoint_sidecar_at_another_page(md: &Path) -> String {
    let path = outl_md::resolve_sidecar_path(md);
    let mut sc = outl_md::sidecar::read(&path).expect("read the projected sidecar");
    let other = NodeId::new();
    sc.page_id = other;
    outl_md::sidecar::write(&path, &sc).expect("write the repointed sidecar");
    other.to_string()
}

#[test]
fn a_healthy_page_reports_its_own_root() {
    let tmp = TempDir::new().unwrap();
    let (_ws, meta, _md) = projected_page(&tmp, "notes");

    let outline = read_page_outline(tmp.path(), &meta).expect("a healthy page reads");
    assert_eq!(outline.sidecar_identity, SidecarIdentity::MatchesPage);
}

#[test]
fn a_sidecar_naming_another_root_is_reported_with_the_root_it_claims() {
    let tmp = TempDir::new().unwrap();
    let (_ws, meta, md) = projected_page(&tmp, "notes");
    let other = repoint_sidecar_at_another_page(&md);

    let outline = read_page_outline(tmp.path(), &meta).expect("the read must still succeed");
    assert_eq!(
        outline.sidecar_identity,
        SidecarIdentity::NamesAnotherPage {
            page_id: other.clone()
        },
        "the verdict has to name the root the sidecar claims — a caller \
         cannot act on a bare boolean"
    );
    assert_ne!(other, meta.id, "the fixture repointed nothing");
}

/// **The mirror, stated as a test rather than as a sentence.**
///
/// The page opens, with its content and with the *sidecar's* block ids —
/// not the transient ones `outline_from_parsed` mints when there is no
/// sidecar to read. A refusal here would be the deleting direction; so
/// would quietly dropping the sidecar and re-minting every id, which
/// breaks every `((blk-…))` handle into the page.
#[test]
fn a_reported_mismatch_changes_nothing_about_the_outline() {
    let tmp = TempDir::new().unwrap();
    let (_ws, meta, md) = projected_page(&tmp, "notes");

    let before = read_page_outline(tmp.path(), &meta).expect("a healthy page reads");
    repoint_sidecar_at_another_page(&md);
    let after = read_page_outline(tmp.path(), &meta).expect("the read must not refuse");

    assert_eq!(
        after.nodes.len(),
        2,
        "the page lost blocks over a condition that is about the sidecar's \
         header, not its entries"
    );
    let texts: Vec<&str> = after.nodes.iter().map(|n| n.text.as_str()).collect();
    assert_eq!(texts, vec!["cites [[elsewhere]]", "and a second block"]);
    let ids_before: Vec<&str> = before.nodes.iter().map(|n| n.id.as_str()).collect();
    let ids_after: Vec<&str> = after.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(
        ids_after, ids_before,
        "the block ids must still come off the sidecar; re-minting them \
         breaks every ((blk-…)) handle into this page"
    );
}

/// The second half of the same mirror, on the surface that reported the
/// bug in the first place.
///
/// `build_backlink_index_from_disk` `warn!`s and `continue`s on an `Err`
/// from `read_page_outline`, so a refusing guard would take this page's
/// `[[elsewhere]]` out of every backlinks panel. Asserting on the index
/// rather than on the read is what makes that unfakeable.
#[test]
fn the_backlink_index_still_carries_the_refs_of_a_mismatched_page() {
    let tmp = TempDir::new().unwrap();
    let (ws, meta, md) = projected_page(&tmp, "notes");
    repoint_sidecar_at_another_page(&md);

    let metas = vec![meta.clone()];
    let index = outl_actions::build_backlink_index_from_disk(&metas, tmp.path());
    let target = PageMeta {
        slug: "elsewhere".to_string(),
        title: "elsewhere".to_string(),
        ..meta.clone()
    };
    let rows = index.for_page(&ws, &target);

    assert_eq!(
        rows.len(),
        1,
        "the citing page was skipped, so its [[ref]] went quiet — that is \
         the deleting direction of the same divergence"
    );
    assert_eq!(rows[0].source_block.text, "cites [[elsewhere]]");
}

/// One "nothing on disk to vouch for" shape: what it is called, how to
/// put a workspace into it, and the verdict it must produce.
type EmptyPageCase = (
    &'static str,
    fn(&TempDir) -> (Workspace, PageMeta),
    SidecarIdentity,
);

/// The shapes a page with no content of its own takes, each of which is
/// ordinary rather than a defect — and each of which a tightened guard
/// would freeze.
fn empty_page_cases() -> Vec<EmptyPageCase> {
    vec![
        // A live page whose `.md` this device has never written: a peer
        // shipped only the ops.
        (
            "a live page with no .md yet",
            |_tmp: &TempDir| {
                let (mut ws, hlc) = workspace();
                let page =
                    open_or_create(&mut ws, &hlc, "synced", "Synced", PageKind::Page).unwrap();
                let meta = page_meta(&ws, page).unwrap();
                (ws, meta)
            },
            SidecarIdentity::Absent,
        ),
        // A `.md` with no sidecar beside it: a peer shipped the
        // projection, or an orphan scan has not run yet.
        (
            "a .md with no sidecar beside it",
            |tmp: &TempDir| {
                let (mut ws, hlc) = workspace();
                let page =
                    open_or_create(&mut ws, &hlc, "shipped", "Shipped", PageKind::Page).unwrap();
                append_block(&mut ws, &hlc, Some(page), Some("from a peer")).unwrap();
                let meta = page_meta(&ws, page).unwrap();
                let md = page_md_path(tmp.path(), &meta);
                std::fs::create_dir_all(md.parent().unwrap()).unwrap();
                std::fs::write(&md, "- from a peer\n").unwrap();
                (ws, meta)
            },
            SidecarIdentity::Absent,
        ),
        // A sidecar with the right `page_id` and no entries. It names
        // this page, so it is not a mismatch — the ids just come out
        // transient, which `read_page_outline` already documents.
        (
            "an empty sidecar for this page",
            |tmp: &TempDir| {
                let (mut ws, hlc) = workspace();
                let page = open_or_create(&mut ws, &hlc, "fresh", "Fresh", PageKind::Page).unwrap();
                let meta = page_meta(&ws, page).unwrap();
                let md = page_md_path(tmp.path(), &meta);
                std::fs::create_dir_all(md.parent().unwrap()).unwrap();
                let text = "- \n";
                std::fs::write(&md, text).unwrap();
                let sc = outl_md::sidecar::Sidecar::new_for_page(
                    page,
                    &outl_md::sidecar::file_hash(text),
                );
                outl_md::sidecar::write(&outl_md::resolve_sidecar_path(&md), &sc).unwrap();
                (ws, meta)
            },
            SidecarIdentity::MatchesPage,
        ),
        // Journal-first means this is the product's first screen, every
        // day, before anything has written it.
        (
            "a brand-new journal day, before its first projection",
            |_tmp: &TempDir| {
                let (mut ws, hlc) = workspace();
                let page = outl_actions::open_today(&mut ws, &hlc).unwrap();
                let meta = page_meta(&ws, page).unwrap();
                (ws, meta)
            },
            SidecarIdentity::Absent,
        ),
        // …and the same day once outl itself has written it.
        (
            "a brand-new journal day, freshly projected",
            |tmp: &TempDir| {
                let (mut ws, hlc) = workspace();
                let page = outl_actions::open_today(&mut ws, &hlc).unwrap();
                let meta = page_meta(&ws, page).unwrap();
                outl_actions::apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
                (ws, meta)
            },
            SidecarIdentity::MatchesPage,
        ),
    ]
}

#[test]
fn a_page_with_nothing_to_vouch_for_is_never_reported_as_a_mismatch() {
    for (name, prepare, expected) in empty_page_cases() {
        let tmp = TempDir::new().unwrap();
        let (_ws, meta) = prepare(&tmp);

        let outline = read_page_outline(tmp.path(), &meta)
            .unwrap_or_else(|e| panic!("{name}: the read must not refuse: {e}"));

        assert_eq!(
            outline.sidecar_identity, expected,
            "{name}: reporting this as a mismatch is how a later \"tighten \
             the check\" freezes an ordinary page"
        );
    }
}
