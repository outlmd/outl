//! The volume guard: how much content one `--repair` may remove before
//! it stops and asks.
//!
//! `--repair` used to print `708 fixed` for a pass that removed 1,426
//! lines of work across 233 pages (RFC 0210). A destructive operation
//! that reports its scale only afterwards cannot be consented to, so the
//! count is measured *before* the write, announced in both modes, and
//! capped — above the ceiling the pass refuses and names `--force`.

use super::*;

/// Seed a page with `total` blocks, project it, then delete `to_delete`
/// of them from the tree **without** re-projecting.
///
/// That is the shape re-projection is for — the log moved on, the `.md`
/// is behind — and also the shape that removes content when the repair
/// runs. Returns the `.md` path and the text of one deleted block, which
/// is the witness for "was anything written".
fn page_with_deleted_blocks(root: &Path, total: usize, to_delete: usize) -> (PathBuf, String) {
    let texts: Vec<String> = (0..total).map(|i| format!("line number {i}")).collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let page = seed_page(root, "notes", &refs);

    let mut ctx = crate::ws::open(root).expect("open");
    let doomed: Vec<outl_core::id::NodeId> = outl_actions::children_of(&ctx.workspace, page)
        .into_iter()
        .map(|(id, _)| id)
        .take(to_delete)
        .collect();
    for node in doomed {
        outl_actions::delete(&mut ctx.workspace, &ctx.hlc, node).expect("delete block");
    }
    drop(ctx);

    (
        root.join("pages").join("notes.md"),
        texts[to_delete - 1].clone(),
    )
}

/// **The one the issue is about.**
///
/// Matching and re-projection both scaled silently: `--repair` printed
/// `708 fixed` while removing 1,426 lines from 233 pages, and nothing in
/// the output mentioned a line. A destructive operation whose scale is
/// invisible turns a small bug into an unrecoverable one, so past the
/// ceiling the page writes stand down and the user is told the number.
#[test]
fn a_repair_that_would_delete_a_lot_of_content_stops_and_asks() {
    let (_dir, root, _paths) = fresh();
    let (md, witness) = page_with_deleted_blocks(&root, 150, 120);

    let report = collect(&root, true).expect("doctor --repair runs");

    assert!(
        has(&report, "would remove 120 content line(s)"),
        "the count must be named, not just the page total: {:#?}",
        messages(&report)
    );
    assert!(
        has(&report, "--force"),
        "the refusal must point at the way to authorise it: {:#?}",
        messages(&report)
    );
    assert!(
        std::fs::read_to_string(&md).unwrap().contains(&witness),
        "the guard must refuse the WHOLE write — half a page is the failure it exists to prevent"
    );
}

/// A refusal nobody can script against is a refusal that gets ignored.
///
/// The guard works in place: it empties the page-write plan and records
/// why through `b.err(...)`. Both CLI entry points decide the process
/// status from `error_count` alone (`run` exits 1, `run_json` forces 1),
/// so recording the refusal at any lower severity would exit 0 and every
/// cron / CI job reading that status would treat a repair that never
/// happened as a repair that succeeded — the silent scaling this guard
/// exists to end, moved into the exit code.
#[test]
fn a_refused_repair_is_an_error_so_the_process_exits_non_zero() {
    let (_dir, root, _paths) = fresh();
    let (_md, _witness) = page_with_deleted_blocks(&root, 150, 120);

    let report = collect(&root, true).expect("doctor --repair runs");

    assert!(
        report.error_count > 0,
        "the refusal must be an error — it is the only thing the exit status reads: {:#?}",
        messages(&report)
    );
    if let Some(rep) = &report.repair {
        assert_eq!(
            rep.repaired, 0,
            "the suppressed page writes must not be counted as fixed: {rep:#?}"
        );
    }
}

/// The escape hatch has to actually work, or the guard is a wall.
#[test]
fn the_same_repair_runs_once_it_is_explicitly_forced() {
    let (_dir, root, _paths) = fresh();
    let (md, witness) = page_with_deleted_blocks(&root, 150, 120);

    let report = super::collect_with_scope(&root, true, RepairScope::Forced)
        .expect("doctor --repair --force runs");

    assert!(
        has(&report, "proceeding: `--force` was given"),
        "a forced run must still say what it is about to remove: {:#?}",
        messages(&report)
    );
    assert!(
        !std::fs::read_to_string(&md).unwrap().contains(&witness),
        "an authorised bulk repair must apply"
    );
}

/// A guard that fires on ordinary work gets disabled, and then it guards
/// nothing. Deleting a handful of blocks must still repair unattended.
#[test]
fn an_ordinary_amount_of_deletion_repairs_without_a_flag() {
    let (_dir, root, _paths) = fresh();
    let (md, witness) = page_with_deleted_blocks(&root, 12, 4);

    let report = collect(&root, true).expect("doctor --repair runs");

    assert!(
        has(&report, "so it runs without `--force`"),
        "the count is still reported, just not blocking: {:#?}",
        messages(&report)
    );
    assert!(
        !std::fs::read_to_string(&md).unwrap().contains(&witness),
        "4 lines is an ordinary peer delete — repairing it must not need a flag"
    );
}

/// The count has to reach the user in the mode where they are still
/// deciding — the read-only one — not only after `--repair` refused.
#[test]
fn the_volume_is_announced_by_a_read_only_run_too() {
    let (_dir, root, _paths) = fresh();
    let (md, witness) = page_with_deleted_blocks(&root, 150, 120);

    let report = collect(&root, false).expect("doctor runs");

    assert!(
        has(&report, "would remove 120 content line(s)"),
        "read-only mode must state the volume: {:#?}",
        messages(&report)
    );
    assert!(
        has(&report, "will refuse this without `--force`"),
        "and the condition attached to it, so the listing promises nothing it cannot do: {:#?}",
        messages(&report)
    );
    assert!(
        report
            .repairable
            .iter()
            .any(|r| r.contains("removes 120 content line(s) from disk")),
        "each offered action carries its own cost: {:#?}",
        report.repairable
    );
    assert!(
        std::fs::read_to_string(&md).unwrap().contains(&witness),
        "read-only means read-only"
    );
}
