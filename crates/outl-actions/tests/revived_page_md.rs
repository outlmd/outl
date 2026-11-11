//! What reviving a deleted page is allowed to do to its `.md`.
//!
//! Reopening a deleted page brings its root back under `root()` and leaves
//! the blocks it was deleted with in the trash (issue #473,
//! `page::revive`). So the page renders **empty**, and the next projection
//! wants to write that over whatever is on disk.
//!
//! That is the mirror direction root `CLAUDE.md` invariant 8 exists for,
//! and the two cases are not the same question:
//!
//! - a `.md` holding lines **no op accounts for** must still refuse. The
//!   revive is `Op::Move`s only — it never advances `last_synced_hash` and
//!   never emits ops for anything it read — so the guard's verdict is
//!   untouched by it. This test is what makes that a fact rather than a
//!   claim about code nobody re-read.
//! - a `.md` holding lines the op log **does** hold, now in the trash, is
//!   overwritten. That is the delete the user asked for finally landing on
//!   disk, and invariant 6 keeps every one of those blocks in the log and
//!   in `trash list`. Content *hidden*, not content *deleted* — the same
//!   asymmetry invariant 8 argues from.

use outl_actions::append_block;
use outl_actions::error::ActionError;
use outl_actions::journal::{
    apply_page_md_with_sidecar, apply_page_md_with_sidecar_if_stale, page_md_path,
};
use outl_actions::page::{delete, open_or_create, page_meta, PageKind};
use outl_actions::refusal::Surface;
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::workspace::Workspace;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A projected page with one block, then deleted — the `.md` and sidecar
/// deliberately left behind, which is the state a peer's delete (or a
/// client that never ran `remove_page_projection`) produces locally.
fn deleted_page_with_its_md_still_on_disk() -> (TempDir, Workspace, HlcGenerator, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("yesterday's line")).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
    let md_path = page_md_path(tmp.path(), &page_meta(&ws, page).unwrap());

    delete(&mut ws, &hlc, "notes").unwrap();
    (tmp, ws, hlc, md_path)
}

/// Declare the bytes currently on disk as the last faithful projection,
/// without touching the sidecar's block entries. This is the state a
/// `reconcile_md` leaves behind when it rewrites the sidecar to agree with
/// a `.md` whose content never became ops — the #210 shape, and the reason
/// the hash gate alone is not evidence (invariant 8).
fn restamp_sidecar_as_faithful(md_path: &Path) {
    let sidecar_path = outl_md::sidecar::sidecar_path_for(md_path);
    let mut sidecar = outl_md::sidecar::read(&sidecar_path).unwrap();
    let disk = std::fs::read_to_string(md_path).unwrap();
    sidecar.last_synced_hash = outl_md::sidecar::file_hash(&disk);
    outl_md::sidecar::write(&sidecar_path, &sidecar).unwrap();
}

#[test]
fn reviving_a_page_does_not_let_its_md_lose_unlogged_content() {
    let (tmp, mut ws, hlc, md_path) = deleted_page_with_its_md_still_on_disk();

    // A line that exists in no op: typed into the file by hand, or by any
    // editor that is not outl.
    let on_disk = format!(
        "{}- a line the op log never saw\n",
        std::fs::read_to_string(&md_path).unwrap()
    );
    std::fs::write(&md_path, &on_disk).unwrap();
    restamp_sidecar_as_faithful(&md_path);

    // Reopening revives the page, empty.
    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();

    let refused = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap_err();
    // Exactly one line, and that line: "yesterday's line" is in the
    // sidecar, so the guard must account for it and refuse only on the one
    // the log never saw. A looser `>= 1` would also pass if the guard
    // started refusing over the trashed block, which is a different (and
    // page-freezing) bug.
    match &refused {
        ActionError::PageMarkdownAheadOfLog { lines, sample, .. } => {
            assert_eq!(*lines, 1, "refused over the wrong lines: {sample}");
            assert!(
                sample.contains("a line the op log never saw"),
                "the refusal has to name what is at risk: {sample}"
            );
        }
        other => panic!("the revive must not make unlogged content overwritable, got {other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(&md_path).unwrap(),
        on_disk,
        "a refused write must leave the file byte-identical"
    );
}

#[test]
fn reviving_a_page_reprojects_an_md_whose_lines_the_log_still_holds() {
    let (tmp, mut ws, hlc, md_path) = deleted_page_with_its_md_still_on_disk();
    assert!(std::fs::read_to_string(&md_path)
        .unwrap()
        .contains("yesterday's line"));

    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();
    assert!(
        wrote.is_some(),
        "the revived page's `.md` is stale and nothing on it is unaccounted for"
    );
    assert!(
        !std::fs::read_to_string(&md_path)
            .unwrap()
            .contains("yesterday's line"),
        "the delete the user asked for has to reach the file"
    );

    // And it is hidden, not gone: the op log still holds it, which is what
    // makes the write above acceptable.
    let still_recoverable = outl_actions::trash::list(&ws, Surface::Cli)
        .into_iter()
        .any(|entry| entry.preview.contains("yesterday's line"));
    assert!(
        still_recoverable,
        "the line has to be restorable, or that write was data loss"
    );
}
