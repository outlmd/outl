//! The survey and its executor, driven end to end.
//!
//! The two halves share every fixture — a projected page, a blanked
//! sidecar, a withheld hash — and most cases assert on both: the state
//! the survey reports *and* what the sweep then does with it. Splitting
//! the cases across the two modules would duplicate the fixtures and
//! let the pair drift, which is the divergence the single-owner rule
//! here exists to prevent.

use std::path::{Path, PathBuf};

use crate::block::append_block;
use crate::journal::{
    apply_page_md_with_sidecar, page_md_path, reproject_stale_pages, survey_page_projections,
    PageProjectionState,
};
use crate::page::{open_or_create, page_meta, PageKind};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use tempfile::TempDir;

/// A workspace with one projected page holding `lines`, plus the
/// path of its `.md`.
fn projected_page(lines: &[&str]) -> (TempDir, Workspace, HlcGenerator, NodeId, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    for line in lines {
        append_block(&mut ws, &hlc, Some(page), Some(line)).unwrap();
    }
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
    let md_path = page_md_path(tmp.path(), &page_meta(&ws, page).unwrap());
    (tmp, ws, hlc, page, md_path)
}

/// Rewrite the sidecar the way every pre-0.11 build did: block ids
/// and hashes, no text.
fn blank_sidecar_texts(md_path: &Path) {
    let sidecar_path = outl_md::sidecar::sidecar_path_for(md_path);
    let mut sc = outl_md::sidecar::read(&sidecar_path).unwrap();
    for block in &mut sc.blocks {
        block.text = String::new();
    }
    outl_md::sidecar::write(&sidecar_path, &sc).unwrap();
}

/// The sentinel `reconcile_md` writes when it read content it could
/// not turn into ops: an empty `last_synced_hash`, block entries
/// untouched.
fn withhold_sidecar_hash(md_path: &Path) {
    let sidecar_path = outl_md::sidecar::sidecar_path_for(md_path);
    let mut sc = outl_md::sidecar::read(&sidecar_path).unwrap();
    sc.last_synced_hash = String::new();
    outl_md::sidecar::write(&sidecar_path, &sc).unwrap();
}

fn restamp_sidecar_as_faithful(md_path: &Path) {
    let sidecar_path = outl_md::sidecar::sidecar_path_for(md_path);
    let mut sc = outl_md::sidecar::read(&sidecar_path).unwrap();
    let disk = std::fs::read_to_string(md_path).unwrap();
    sc.last_synced_hash = outl_md::sidecar::file_hash(&disk);
    outl_md::sidecar::write(&sidecar_path, &sc).unwrap();
}

#[test]
fn survey_reports_a_page_whose_tree_ran_ahead_as_stale() {
    let (tmp, mut ws, hlc, page, _md) = projected_page(&["first"]);
    append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();

    let survey = survey_page_projections(&ws, tmp.path(), false);

    let found = survey.iter().find(|p| p.page_root == page).unwrap();
    assert_eq!(
        found.state,
        PageProjectionState::Stale { lines_removed: 0 },
        "an append-only tree advance removes nothing from disk"
    );
}

#[test]
fn survey_counts_the_lines_a_reprojection_would_remove() {
    let (tmp, mut ws, hlc, page, _md) = projected_page(&["keep", "peer deleted this"]);
    let doomed = crate::tree::children_of(&ws, page)[1].0;
    crate::block::delete(&mut ws, &hlc, doomed).unwrap();

    let survey = survey_page_projections(&ws, tmp.path(), false);

    let found = survey.iter().find(|p| p.page_root == page).unwrap();
    assert_eq!(
        found.state,
        PageProjectionState::Stale { lines_removed: 1 },
        "a peer delete is a stale projection that removes one disk line"
    );
}

#[test]
fn survey_reports_a_page_holding_unlogged_content_as_ahead_of_the_log() {
    let (tmp, ws, _hlc, page, md_path) = projected_page(&["first"]);
    std::fs::write(&md_path, "- first\n- only ever on disk\n").unwrap();
    restamp_sidecar_as_faithful(&md_path);

    let survey = survey_page_projections(&ws, tmp.path(), false);

    let found = survey.iter().find(|p| p.page_root == page).unwrap();
    match &found.state {
        PageProjectionState::AheadOfLog { lines, sample } => {
            assert_eq!(*lines, 1);
            assert!(sample.contains("only ever on disk"), "got {sample:?}");
        }
        other => panic!("expected AheadOfLog, got {other:?}"),
    }
}

#[test]
fn the_sweep_reprojects_a_stale_page_that_loses_nothing() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page(&["first"]);
    append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();

    let sweep = reproject_stale_pages(&ws, tmp.path());

    assert_eq!(sweep.written, vec![md_path.clone()]);
    assert!(std::fs::read_to_string(&md_path)
        .unwrap()
        .contains("synced-in"));
}

#[test]
fn the_sweep_withholds_a_page_whose_reprojection_would_remove_content() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page(&["keep", "peer deleted this"]);
    let doomed = crate::tree::children_of(&ws, page)[1].0;
    crate::block::delete(&mut ws, &hlc, doomed).unwrap();
    let before = std::fs::read_to_string(&md_path).unwrap();

    let sweep = reproject_stale_pages(&ws, tmp.path());

    assert!(sweep.written.is_empty(), "nothing may be written");
    assert_eq!(sweep.withheld.len(), 1);
    assert_eq!(sweep.withheld[0].lines_removed, 1);
    assert_eq!(
        std::fs::read_to_string(&md_path).unwrap(),
        before,
        "a content-removing write belongs to `outl doctor --repair`, not to a \
         background pass"
    );
}

#[test]
fn the_sweep_surfaces_a_page_that_stopped_syncing_as_a_refusal() {
    let (tmp, ws, _hlc, page, md_path) = projected_page(&["first"]);
    std::fs::write(&md_path, "- first\n- only ever on disk\n").unwrap();
    restamp_sidecar_as_faithful(&md_path);

    let sweep = reproject_stale_pages(&ws, tmp.path());

    assert!(sweep.written.is_empty());
    assert_eq!(sweep.refused.len(), 1, "a refusal has to reach the user");
    assert_eq!(sweep.refused[0].path, md_path);
    assert!(matches!(
        sweep.refused[0].error,
        crate::ActionError::PageMarkdownAheadOfLog { .. }
    ));
    let _ = page;
}

#[test]
fn the_sweep_leaves_a_page_with_a_pending_external_edit_alone() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page(&["first"]);
    // A hand edit nobody reconciled yet: the sidecar hash no longer
    // matches the file.
    std::fs::write(&md_path, "- first\n- typed by hand\n").unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();

    let sweep = reproject_stale_pages(&ws, tmp.path());

    assert!(sweep.written.is_empty());
    assert_eq!(
        std::fs::read_to_string(&md_path).unwrap(),
        "- first\n- typed by hand\n",
        "`.md → tree` owns an unreconciled edit; the projection pass must not clobber it"
    );
}

/// Every sidecar written before 0.11 carries `text: ""`, so it can
/// say which blocks a page had and nothing about what they said.
/// That is **not** "nothing at risk" — it is "I cannot tell", and a
/// background pass that read the two as the same thing would
/// overwrite exactly the pages nobody can vouch for.
#[test]
fn the_sweep_never_writes_a_page_whose_sidecar_cannot_answer() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page(&["first"]);
    blank_sidecar_texts(&md_path);
    append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();
    let before = std::fs::read_to_string(&md_path).unwrap();

    let survey = survey_page_projections(&ws, tmp.path(), false);
    assert_eq!(
        survey.iter().find(|p| p.page_root == page).unwrap().state,
        PageProjectionState::SidecarCannotAnswer
    );

    let sweep = reproject_stale_pages(&ws, tmp.path());

    assert!(sweep.written.is_empty());
    assert_eq!(std::fs::read_to_string(&md_path).unwrap(), before);
}

/// An undownloaded iCloud file reads as `NotFound` — the real name
/// does not exist, only `.notes.md.icloud` does — and the survey
/// mapped `NotFound` straight to `Absent`, which is routed to a
/// write.
#[test]
fn survey_does_not_call_an_undownloaded_page_absent() {
    let (tmp, ws, _hlc, page, md_path) = projected_page(&["first"]);
    std::fs::remove_file(outl_md::sidecar::sidecar_path_for(&md_path)).unwrap();
    std::fs::remove_file(&md_path).unwrap();
    std::fs::write(md_path.with_file_name(".notes.md.icloud"), "").unwrap();

    let survey = survey_page_projections(&ws, tmp.path(), false);

    let found = survey.iter().find(|p| p.page_root == page).unwrap();
    assert!(
        !matches!(found.state, PageProjectionState::Absent),
        "a file iCloud has not fetched yet is not an absent page, got {:?}",
        found.state
    );
    let sweep = reproject_stale_pages(&ws, tmp.path());
    assert!(sweep.written.is_empty());
    assert!(!md_path.exists(), "nothing may be written over the page");
}

/// The same absence, with the evidence that actually survives iCloud:
/// the placeholder is a dotfile (dropped in cross-device sync), the
/// `.outl` sidecar is not. A sidecar is only ever written beside a
/// `.md` this device projected, so it is proof the page existed here.
#[test]
fn survey_does_not_call_a_vanished_md_absent() {
    let (tmp, ws, _hlc, page, md_path) = projected_page(&["first"]);
    std::fs::remove_file(&md_path).unwrap();

    let survey = survey_page_projections(&ws, tmp.path(), false);

    let found = survey.iter().find(|p| p.page_root == page).unwrap();
    assert!(
        !matches!(found.state, PageProjectionState::Absent),
        "a missing .md beside a live sidecar is a lost file, got {:?}",
        found.state
    );
    let sweep = reproject_stale_pages(&ws, tmp.path());
    assert!(sweep.written.is_empty());
    assert!(!md_path.exists(), "nothing may be written over the page");
}

/// `ReprojectionSweep::declined`'s own doc: "a skip nobody can see is
/// the failure invariant 8 exists to prevent." An unreadable `.md`
/// folded into the same empty arm as `InSync` and the sweep had no
/// field for it, so `outl serve` said nothing at all about a page it
/// could not read.
#[test]
fn the_sweep_names_a_page_it_could_not_read() {
    let (tmp, ws, _hlc, _page, md_path) = projected_page(&["first"]);
    std::fs::write(&md_path, [0xff, 0xfe, 0x00]).unwrap();

    let sweep = reproject_stale_pages(&ws, tmp.path());

    assert!(sweep.written.is_empty());
    assert_eq!(
        sweep.unreadable.len(),
        1,
        "a page the sweep could not read has to be nameable"
    );
    assert_eq!(sweep.unreadable[0].path, md_path);
    assert!(
        !sweep.unreadable[0].reason.is_empty(),
        "the reason is what the daemon prints"
    );
}

/// A withheld hash over a `.md` that happens to equal the render.
///
/// `reconcile_md` writes `last_synced_hash = ""` when it read content
/// it could not log. The `InSync` shortcut returned before the
/// unlogged question was ever asked, so the `withheld` branch above
/// was dead for the *dominant* shape: the page is frozen in both
/// directions and neither `doctor` nor `serve` ever names it.
#[test]
fn survey_reports_a_withheld_hash_page_whose_render_matches_disk() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page(&["first"]);
    append_block(&mut ws, &hlc, Some(page), Some("only ever on disk")).unwrap();
    // Disk equals the render, so the hash gate below it would read
    // `InSync` — but the sidecar still holds only the logged block.
    let rendered = crate::journal::render_page_md(&ws, page);
    std::fs::write(&md_path, &rendered).unwrap();
    withhold_sidecar_hash(&md_path);

    let survey = survey_page_projections(&ws, tmp.path(), false);

    let found = survey.iter().find(|p| p.page_root == page).unwrap();
    match &found.state {
        PageProjectionState::AheadOfLog { lines, sample } => {
            assert_eq!(*lines, 1);
            assert!(sample.contains("only ever on disk"), "got {sample:?}");
        }
        other => panic!("expected AheadOfLog, got {other:?}"),
    }
}

/// The mirror, and the listing-promises-what-the-writer-refuses shape
/// invariant 8 names: a withheld hash whose unlogged set is now
/// empty classified as `Stale` — an offered repair — while
/// `_if_stale` stops at `last_synced_hash != disk_hash` and returns
/// `Ok(None)` every time. The user was then told the file "changed
/// underneath", which is not true either.
///
/// It needs a `.md → tree` reconcile to restamp the hash, not a
/// re-projection, so no repair is offered and nothing is declined.
#[test]
fn survey_offers_no_repair_for_a_withheld_hash_with_nothing_unlogged() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page(&["first"]);
    withhold_sidecar_hash(&md_path);
    // The tree moves ahead, so the render differs from disk: the
    // exact input that used to read as a repairable `Stale`.
    append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();

    let survey = survey_page_projections(&ws, tmp.path(), false);

    let found = survey.iter().find(|p| p.page_root == page).unwrap();
    assert!(
        !matches!(found.state, PageProjectionState::Stale { .. }),
        "a withheld hash cannot be repaired by a re-projection, got {:?}",
        found.state
    );

    let sweep = reproject_stale_pages(&ws, tmp.path());
    assert!(sweep.written.is_empty());
    assert!(
        sweep.declined.is_empty(),
        "a state the writer structurally refuses must not be offered to it"
    );
}

#[test]
fn the_sweep_projects_a_page_that_has_no_md_on_disk_at_all() {
    let (tmp, ws, _hlc, page, md_path) = projected_page(&["first"]);
    std::fs::remove_file(&md_path).unwrap();
    std::fs::remove_file(outl_md::sidecar::sidecar_path_for(&md_path)).unwrap();

    let sweep = reproject_stale_pages(&ws, tmp.path());

    assert_eq!(sweep.written, vec![md_path.clone()]);
    assert!(std::fs::read_to_string(&md_path).unwrap().contains("first"));
    let _ = page;
}
