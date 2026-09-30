//! The checks that need the materialized tree, and the repair they
//! hand to `--repair`.
//!
//! Deletion is `Move(node, TRASH_ROOT)` (root `CLAUDE.md` invariant 6),
//! so a deleted block is still in the tree and nothing but this check
//! tells the user it is there. Projection drift is the other direction:
//! the tree moved on and the `.md` did not, which is the one case where
//! `--repair` is allowed to overwrite a page — and the neighbouring
//! case, a `.md` holding content no op ever saw, where it must refuse
//! (root `CLAUDE.md` invariant 8).

use super::*;

// ----------------------------------------------------------------- trash

/// Deletion is `Move(node, TRASH_ROOT)`, so a deleted block never leaves
/// the tree — and until now nothing told the user it was there.
#[test]
fn deleted_blocks_are_counted_and_previewed() {
    let (_dir, root, _paths) = fresh();
    let page = seed_page(&root, "notes", &["keep me", "delete me"]);

    {
        let mut ctx = crate::ws::open(&root).expect("open");
        let victim = outl_actions::project_outline(&ctx.workspace, page)
            .into_iter()
            .find(|n| n.text.contains("delete me"))
            .expect("the block to delete");
        let id: NodeId = victim
            .id
            .parse::<ulid::Ulid>()
            .map(NodeId)
            .expect("node id");
        outl_actions::delete(&mut ctx.workspace, &ctx.hlc, id).expect("delete");
        outl_actions::apply_page_md_with_sidecar(&ctx.workspace, &root, page).expect("re-project");
    }

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        has(&report, "trash holds 1 block(s)"),
        "expected the trash count, got: {:#?}",
        messages(&report)
    );
    assert!(
        has(&report, "delete me"),
        "the trashed block's text must be previewed, got: {:#?}",
        messages(&report)
    );
}

/// The doctor is where a user first learns the trash is restorable at
/// all, so the count it prints is a verdict from `trash::refusal_for`
/// and not decoration. It had no assertion until this test: the string
/// appears exactly once in the repo, in the code that emits it.
#[test]
fn the_doctor_says_how_many_deletions_can_be_put_back() {
    let (_dir, root, _paths) = fresh();
    let page = seed_page(&root, "notes", &["keep me", "delete me"]);

    {
        let mut ctx = crate::ws::open(&root).expect("open");
        let victim = outl_actions::project_outline(&ctx.workspace, page)
            .into_iter()
            .find(|n| n.text.contains("delete me"))
            .expect("the block to delete");
        let id: NodeId = victim
            .id
            .parse::<ulid::Ulid>()
            .map(NodeId)
            .expect("node id");
        outl_actions::delete(&mut ctx.workspace, &ctx.hlc, id).expect("delete");
        outl_actions::apply_page_md_with_sidecar(&ctx.workspace, &root, page).expect("re-project");
    }

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        has(&report, "1 of 1 can be put back"),
        "the doctor has to name the restorable count, got: {:#?}",
        messages(&report)
    );
}

#[test]
fn an_empty_trash_says_so() {
    let (_dir, root, _paths) = fresh();
    seed_page(&root, "notes", &["hello"]);

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        has(&report, "trash is empty"),
        "expected the empty-trash all-clear, got: {:#?}",
        messages(&report)
    );
}

// ------------------------------------------------------ projection drift

#[test]
fn a_stale_md_is_flagged_and_repair_reprojects_it_with_a_backup() {
    let (_dir, root, paths) = fresh();
    let page = seed_page(&root, "notes", &["first"]);

    // Mutate the tree WITHOUT re-projecting: the `.md` stays a faithful
    // projection of an older tree, which is exactly the drift case.
    {
        let mut ctx = crate::ws::open(&root).expect("open");
        outl_actions::append_block(&mut ctx.workspace, &ctx.hlc, Some(page), Some("second"))
            .expect("append");
    }

    let md = paths.pages.join("notes.md");
    let before = std::fs::read_to_string(&md).unwrap();
    assert!(!before.contains("second"));

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        has(&report, "stale projection"),
        "expected the drift warning, got: {:#?}",
        messages(&report)
    );
    assert!(
        report.repairable.iter().any(|r| r.contains("re-project")),
        "drift must be listed as repairable: {:?}",
        report.repairable
    );

    let repaired = collect(&root, true).expect("doctor --repair runs");
    let rep = repaired.repair.expect("a repair report");
    assert_eq!(rep.failed, 0, "repair actions: {:#?}", rep.actions);

    let after = std::fs::read_to_string(&md).unwrap();
    assert!(
        after.contains("second"),
        "the `.md` must be re-projected from the op log, got: {after:?}"
    );
    let backup = Path::new(&rep.backup_dir).join("pages").join("notes.md");
    assert_eq!(
        std::fs::read_to_string(&backup).unwrap(),
        before,
        "the pre-repair `.md` must be recoverable from {}",
        backup.display()
    );
}

#[test]
fn a_missing_sidecar_is_rebuilt_when_the_md_matches_the_op_log() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["hello"]);
    let md = paths.pages.join("notes.md");
    let sidecar = outl_md::sidecar::sidecar_path_for(&md);
    let md_before = std::fs::read_to_string(&md).unwrap();
    std::fs::remove_file(&sidecar).unwrap();

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        report
            .repairable
            .iter()
            .any(|r| r.contains("rebuild the sidecar")),
        "a lost sidecar over matching content is repairable: {:?}",
        report.repairable
    );

    let repaired = collect(&root, true).expect("doctor --repair runs");
    let rep = repaired.repair.expect("a repair report");
    assert_eq!(rep.failed, 0, "repair actions: {:#?}", rep.actions);
    assert!(sidecar.exists(), "the sidecar must be back");
    assert_eq!(
        std::fs::read_to_string(&md).unwrap(),
        md_before,
        "rebuilding a sidecar must not change the `.md`"
    );
}

/// A `.md` with no sidecar whose content the op log has never seen may
/// be the only copy of that content. `--repair` must refuse it and send
/// the user to `outl reconcile`.
#[test]
fn a_missing_sidecar_over_diverged_content_is_never_repaired() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["hello"]);
    let md = paths.pages.join("notes.md");
    std::fs::remove_file(outl_md::sidecar::sidecar_path_for(&md)).unwrap();
    std::fs::write(&md, "- hello\n- typed only in the editor\n").unwrap();

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        !report
            .repairable
            .iter()
            .any(|r| r.contains("rebuild the sidecar")),
        "diverged content must NOT be repairable: {:?}",
        report.repairable
    );
    assert!(
        has(&report, "run `outl reconcile`"),
        "the user must be pointed at reconcile, got: {:#?}",
        messages(&report)
    );

    let repaired = collect(&root, true).expect("doctor --repair runs");
    assert_eq!(
        std::fs::read_to_string(&md).unwrap(),
        "- hello\n- typed only in the editor\n",
        "`--repair` must never clobber content the op log has not seen"
    );
    assert!(repaired.repair.is_none(), "there was nothing safe to do");
}
