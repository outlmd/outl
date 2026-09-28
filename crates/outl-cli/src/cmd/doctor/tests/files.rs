//! The files beside the op log: sync-conflict copies, sidecars with no
//! `.md`, and `.md` lines the outl dialect does not recognise.
//!
//! Two of these are loud on purpose and one is deliberately quiet. A
//! conflict copy is user content outl will never read, so it is an
//! error; an orphaned sidecar is a lost file, so it is a warning; a
//! parse warning is only written to `.outl/orphans.log` under
//! `--repair`, because a read-only run that appends thousands of rows
//! buries the matching orphans already in there.

use super::*;

// ------------------------------------------------------ sync conflicts

#[test]
fn sync_conflict_copies_are_reported_as_errors() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["hello"]);

    std::fs::write(paths.pages.join("notes 2.md"), "- forked\n").unwrap();
    std::fs::write(
        paths
            .pages
            .join("notes.sync-conflict-20260805-101500-ABCDEFG.md"),
        "- forked\n",
    )
    .unwrap();

    let report = collect(&root, false).expect("doctor runs");
    let conflicts: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.message.contains("conflict copy"))
        .map(|f| f.message.clone())
        .collect();
    assert_eq!(
        conflicts.len(),
        2,
        "both conflict copies must be reported, got: {conflicts:#?}"
    );
    assert!(
        conflicts.iter().any(|m| m.contains("Syncthing"))
            && conflicts.iter().any(|m| m.contains("iCloud")),
        "each conflict must name its transport: {conflicts:#?}"
    );
    assert!(
        report.error_count > 0,
        "conflict copies are loud on purpose"
    );
}

/// `sprint 2.md` with no `sprint.md` next to it is a perfectly normal
/// note. Flagging it would train the user to ignore the loudest finding
/// the doctor emits.
#[test]
fn a_numeric_suffix_without_a_base_file_is_not_a_conflict() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["hello"]);
    std::fs::write(paths.pages.join("sprint 2.md"), "- real note\n").unwrap();

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        !has(&report, "conflict copy"),
        "`sprint 2.md` has no `sprint.md` sibling — not a conflict: {:#?}",
        messages(&report)
    );
}

// ----------------------------------------------------- orphan sidecars

/// A `.outl` with no `.md` next to it. The check used to look only for
/// the legacy dotted spelling, so it was dead on every workspace a
/// current build wrote.
#[test]
fn an_orphaned_modern_sidecar_is_reported() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["hello"]);
    std::fs::remove_file(paths.pages.join("notes.md")).unwrap();

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        has(&report, "orphaned sidecar"),
        "expected the orphan-sidecar warning, got: {:#?}",
        messages(&report)
    );
}

// ------------------------------------------------------ parse warnings

/// A journal whose content steps outside the outl dialect
/// (heading + paragraph + bullet) must surface a parser warning
/// in the doctor report AND — under `--repair`, the only writing
/// mode — drop a tagged row into `.outl/orphans.log` so the trail
/// persists.
#[test]
fn doctor_surfaces_parse_warnings_and_logs_them() {
    let (_dir, root, paths) = fresh();

    let journal = paths
        .journals
        .join(format!("{}.md", crate::workspace_layout::today()));
    std::fs::write(
        &journal,
        "# 2026-06-08\n\nfree paragraph\n\n- real bullet\n",
    )
    .unwrap();

    let report = collect(&root, true).expect("doctor must run on a dirty workspace");
    let dirty = report
        .findings
        .iter()
        .filter(|f| f.message.contains("outside outl dialect"))
        .count();
    assert!(
        dirty >= 1,
        "expected at least one parser-warning finding, got: {:#?}",
        report.findings
    );

    let log = std::fs::read_to_string(&paths.orphans).unwrap_or_default();
    assert!(
        log.contains("parse-warning"),
        "orphans.log should carry a `parse-warning` row, got: {log:?}"
    );
    assert!(
        log.contains("unrecognized_block_marker"),
        "kind tag missing in orphans.log: {log:?}"
    );
}

/// A dirty **page** and a clean **journal** must not produce a
/// contradiction. `check_parse_warnings` runs once per directory,
/// so emitting the all-clear from inside it printed "every `.md`
/// parses cleanly" three lines after listing a page's bad ones.
/// The tally is workspace-wide; so is the verdict.
#[test]
fn a_dirty_page_suppresses_the_all_clear_even_when_journals_are_clean() {
    let (_dir, root, paths) = fresh();

    // Journals stay clean; the page carries an unreadable rule.
    std::fs::write(
        paths.pages.join("tasks.md"),
        "- TODO ship it\n  remind:: every 1h\n",
    )
    .unwrap();

    let report = collect(&root, false).expect("doctor must run");
    let all_clear = report
        .findings
        .iter()
        .filter(|f| f.message.contains("parses cleanly"))
        .count();
    let dirty = report
        .findings
        .iter()
        .filter(|f| f.message.contains("outside outl dialect"))
        .count();

    assert_eq!(dirty, 1, "the page's bad rule must be reported");
    assert_eq!(
        all_clear, 0,
        "the all-clear must not appear alongside a reported warning: {:#?}",
        report.findings
    );
}

/// The `remind::` grammar is the one property the parser
/// validates, so its recoveries must reach the report and the log
/// with their own kind tags — not fold into
/// `unrecognized_block_marker`.
#[test]
fn doctor_tags_remind_warnings_by_kind() {
    let (_dir, root, paths) = fresh();

    std::fs::write(
        paths.pages.join("tasks.md"),
        "- TODO a\n  remind:: every 1h\n- TODO b\n  remind:: 10am max 50\n",
    )
    .unwrap();

    collect(&root, true).expect("doctor must run");
    let log = std::fs::read_to_string(&paths.orphans).unwrap_or_default();
    assert!(
        log.contains("remind_missing_anchor"),
        "missing-anchor tag absent: {log:?}"
    );
    assert!(
        log.contains("remind_max_clamped"),
        "clamped tag absent: {log:?}"
    );
}

/// A clean dialect file must NOT pollute orphans.log with
/// `parse-warning` rows and the doctor report should call out
/// the absence so the user knows the check ran.
#[test]
fn doctor_is_silent_on_clean_files() {
    let (_dir, root, paths) = fresh();

    let journal = paths
        .journals
        .join(format!("{}.md", crate::workspace_layout::today()));
    std::fs::write(&journal, "- one bullet\n- another\n").unwrap();

    let report = collect(&root, false).unwrap();
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.message.contains("every `.md` parses cleanly")),
        "expected the clean-parse OK finding, got: {:#?}",
        report.findings
    );

    let log = std::fs::read_to_string(&paths.orphans).unwrap_or_default();
    assert!(
        !log.contains("parse-warning"),
        "orphans.log must stay clean of parse-warning rows for a tidy workspace"
    );
}
