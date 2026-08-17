//! Doctor test battery.
//!
//! Every check gets the same shape of test: build a real workspace in a
//! `TempDir`, break exactly one thing on purpose, and assert the doctor
//! *names* it. A check that fires on a healthy workspace is as bad as
//! one that stays silent on a broken one, so the happy path is asserted
//! alongside.
//!
//! This file covers **detection**: does the doctor see the defect.
//! [`safety`] covers the other half — what the doctor is allowed to
//! write while it looks.

mod device_store;
mod safety;

use super::*;
use crate::workspace_layout::{init, Paths};
use outl_core::id::NodeId;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

// ---------------------------------------------------------------- setup

/// A fresh, initialized workspace.
fn fresh() -> (TempDir, PathBuf, Paths) {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path().join("notes");
    let paths = Paths::at(&root);
    init(&paths).expect("init");
    (dir, root, paths)
}

/// Create page `slug` with one block per entry of `blocks`, and project
/// it (`.md` + sidecar) so the workspace starts consistent.
fn seed_page(root: &Path, slug: &str, blocks: &[&str]) -> NodeId {
    let mut ctx = crate::ws::open(root).expect("open workspace");
    let page = outl_actions::open_or_create_page(
        &mut ctx.workspace,
        &ctx.hlc,
        slug,
        slug,
        outl_actions::PageKind::Page,
    )
    .expect("create page");
    for text in blocks {
        outl_actions::append_block(&mut ctx.workspace, &ctx.hlc, Some(page), Some(text))
            .expect("append block");
    }
    outl_actions::apply_page_md_with_sidecar(&ctx.workspace, root, page).expect("project page");
    page
}

/// The single `ops-<actor>.jsonl` a freshly-seeded workspace has.
fn ops_file(paths: &Paths) -> PathBuf {
    let mut found: Vec<PathBuf> = std::fs::read_dir(&paths.ops)
        .expect("read ops dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("ops-") && n.ends_with(".jsonl"))
        })
        .collect();
    found.sort();
    found
        .pop()
        .expect("an ops-*.jsonl must exist after seeding")
}

fn append_bytes(path: &Path, bytes: &[u8]) {
    use std::io::Write as _;
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("open ops file for append");
    f.write_all(bytes).expect("append");
}

fn messages(report: &DoctorReport) -> Vec<String> {
    report.findings.iter().map(|f| f.message.clone()).collect()
}

/// A doctor run with the default authority — no `--force`.
///
/// The whole battery goes through this rather than `collect_scoped` so
/// that a test which *wants* the forced scope has to say so, and reads
/// as the exception it is.
fn collect(path: &Path, do_repair: bool) -> Result<DoctorReport, ApiError> {
    collect_with_scope(path, do_repair, RepairScope::Guarded)
}

/// A doctor run against a device store **this call owns**.
///
/// Not a detail. `doctor` now reads (and under `--repair`, prunes) the
/// device store's actor bindings, and that store is machine-global: the
/// repo's `.cargo/config.toml` points every cargo-spawned process at one
/// shared `.dev-device-store`. Going through `collect_scoped` here would
/// make every test in this battery judge — and delete from — a directory
/// that other tests, other test binaries and the developer's own
/// `cargo run` are all writing to. The binding count would drift, the
/// `repairable[]` assertions with it, and the result is issue #211's
/// flaky doctor tests reintroduced by the fix for issue #211.
///
/// A `TempDir` per call is safe *here* precisely because it is not an
/// env-var mutation: `collect_internal` takes the store as an argument,
/// so nothing about this is process-wide (root `CLAUDE.md` invariant 9,
/// third question).
fn collect_with_scope(
    path: &Path,
    do_repair: bool,
    scope: RepairScope,
) -> Result<DoctorReport, ApiError> {
    let store_dir = tempfile::TempDir::new().expect("temp device store");
    super::collect_internal(
        path,
        true,
        do_repair,
        scope,
        &outl_core::device::DeviceStore::at(store_dir.path()),
    )
}

/// Every file under `dir`, by relative path and content.
///
/// The `ops/` assertions compare **directories**, not one file. The
/// original version of `repair_never_touches_the_op_log` diffed only the
/// `.jsonl` while its message promised "not a single byte into ops/" —
/// and the doctor was meanwhile creating `.ops-<actor>.idx` /
/// `.ops-<actor>.nodes.idx` next to it on every run, which the assert
/// could not see.
fn dir_snapshot(dir: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    for entry in walkdir::WalkDir::new(dir) {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let rel = path.strip_prefix(dir).unwrap_or(path).to_path_buf();
        out.insert(rel, std::fs::read(path).unwrap_or_default());
    }
    out
}

fn has(report: &DoctorReport, needle: &str) -> bool {
    report.findings.iter().any(|f| f.message.contains(needle))
}

// ------------------------------------------------- op log: corrupt lines

/// The gap this whole feature exists to close: `JsonlStorage::open`
/// swallows a malformed record so one torn line can't lock the user out,
/// which means nothing ever told the user *which* line was lost.
#[test]
fn a_corrupt_jsonl_line_is_named_with_its_line_number() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["hello"]);
    let ops = ops_file(&paths);
    let lines_before = std::fs::read_to_string(&ops).unwrap().lines().count();

    append_bytes(&ops, b"{\"ts\": not json at all}\n");

    let report = collect(&root, false).expect("doctor runs on a corrupt log");
    let expected = format!("line {}", lines_before + 1);
    assert!(
        has(&report, &expected) && has(&report, "invalid JSON"),
        "expected the bad line named as `{expected}` with a reason, got: {:#?}",
        messages(&report)
    );
    assert!(
        report.error_count > 0,
        "a lost op must be an error, not a warning"
    );
}

/// A partial file sync can leave raw bytes mid-file. `read_line` would
/// abort on those; the doctor must report them and keep scanning.
#[test]
fn non_utf8_bytes_in_the_op_log_are_reported() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["hello"]);
    let ops = ops_file(&paths);

    append_bytes(&ops, &[0xff, 0xfe, 0x00, b'\n']);

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        has(&report, "non-UTF8 bytes"),
        "expected a non-UTF8 finding, got: {:#?}",
        messages(&report)
    );
}

/// Two writers' `write_all`s interleaving glue two ops onto one line.
/// Every op is recoverable, so this is a warning — but a silent one is
/// how you miss that two processes are racing on one file.
#[test]
fn glued_op_lines_are_reported_as_recovered() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["hello"]);
    let ops = ops_file(&paths);
    let first = std::fs::read_to_string(&ops)
        .unwrap()
        .lines()
        .next()
        .expect("at least one op")
        .to_string();

    append_bytes(&ops, format!("{first}{first}\n").as_bytes());

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        has(&report, "glued onto one line"),
        "expected a glued-line warning, got: {:#?}",
        messages(&report)
    );
    assert!(
        !has(&report, "invalid JSON"),
        "a glued line is recoverable — it must not be reported as invalid JSON"
    );
}

/// A healthy workspace must say so, or the noisy checks above train the
/// user to ignore the report.
#[test]
fn a_clean_op_log_gets_an_all_clear() {
    let (_dir, root, _paths) = fresh();
    seed_page(&root, "notes", &["hello"]);

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        has(&report, "every op-log line parses"),
        "expected the op-log all-clear, got: {:#?}",
        messages(&report)
    );
    assert_eq!(
        report.error_count,
        0,
        "a freshly seeded workspace must be error-free: {:#?}",
        messages(&report)
    );
}

// ------------------------------------------------------------- snapshots

#[test]
fn a_corrupt_snapshot_is_flagged_and_repair_moves_it_to_the_backup() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["hello"]);
    let cfg = crate::workspace_layout::read_config(&paths).unwrap();
    let snap_dir = paths.dot_outl.join("snapshots");
    std::fs::create_dir_all(&snap_dir).unwrap();
    let snap = snap_dir.join(format!("snap-{}.bin", cfg.workspace.actor_id));
    std::fs::write(&snap, b"this is not a postcard snapshot").unwrap();

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        has(&report, "snapshot unusable"),
        "expected a corrupt-snapshot warning, got: {:#?}",
        messages(&report)
    );
    assert!(
        report
            .repairable
            .iter()
            .any(|r| r.contains("delete corrupt snapshot")),
        "the corrupt snapshot must be listed as repairable: {:?}",
        report.repairable
    );

    let repaired = collect(&root, true).expect("doctor --repair runs");
    let rep = repaired.repair.expect("a repair report");
    assert_eq!(rep.failed, 0, "repair actions: {:#?}", rep.actions);
    assert!(!snap.exists(), "the corrupt snapshot must be gone");
    let backup = Path::new(&rep.backup_dir);
    assert!(
        backup.exists(),
        "the deleted snapshot must be recoverable from {}",
        rep.backup_dir
    );
}

/// A snapshot written by the real encoder must pass, or the check is
/// just noise on every healthy workspace.
#[test]
fn a_valid_snapshot_passes() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["hello"]);
    let cfg = crate::workspace_layout::read_config(&paths).unwrap();
    let actor = cfg.actor().unwrap();

    let mut ws = crate::ws::open(&root).expect("open");
    ws.workspace.save_snapshot().expect("write snapshot");
    drop(ws);

    let snap = paths
        .dot_outl
        .join("snapshots")
        .join(format!("snap-{actor}.bin"));
    assert!(snap.exists(), "save_snapshot should have written {snap:?}");

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        has(&report, "snapshot decodes, hash verified"),
        "a real snapshot must verify, got: {:#?}",
        messages(&report)
    );
    assert!(
        report.repairable.is_empty(),
        "a healthy workspace has nothing to repair: {:?}",
        report.repairable
    );
}

// ---------------------------------------------------------- offset index

#[test]
fn an_offset_index_pointing_past_eof_is_flagged() {
    use outl_core::hlc::HlcGenerator;
    use outl_core::storage::{ActorIndex, OffsetIndex};

    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["hello"]);
    let cfg = crate::workspace_layout::read_config(&paths).unwrap();
    let actor = cfg.actor().unwrap();

    let mut index = OffsetIndex::new();
    index.insert(HlcGenerator::new(actor).next(), 999_999_999);
    index
        .save(&ActorIndex::sidecar_path(&paths.ops, actor))
        .expect("write idx");

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        has(&report, "past EOF"),
        "expected a stale-index warning, got: {:#?}",
        messages(&report)
    );
}

#[test]
fn an_offset_index_pointing_mid_line_is_flagged() {
    use outl_core::hlc::HlcGenerator;
    use outl_core::storage::{ActorIndex, OffsetIndex};

    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["hello"]);
    let ops = ops_file(&paths);
    let size = std::fs::metadata(&ops).unwrap().len();
    let cfg = crate::workspace_layout::read_config(&paths).unwrap();
    let actor = cfg.actor().unwrap();

    let mut index = OffsetIndex::new();
    // Byte 3 is inside the first record, never the start of one.
    index.insert(HlcGenerator::new(actor).next(), 3);
    assert!(size > 3, "the seeded log must be longer than 3 bytes");
    index
        .save(&ActorIndex::sidecar_path(&paths.ops, actor))
        .expect("write idx");

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        has(&report, "not on a record boundary"),
        "expected a mid-line offset warning, got: {:#?}",
        messages(&report)
    );
}

// ------------------------------------------------------------------ trash

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

// -------------------------------------------------------- sync conflicts

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

// ------------------------------------------------------- projection drift

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

// -------------------------------------------------------- existing checks

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
