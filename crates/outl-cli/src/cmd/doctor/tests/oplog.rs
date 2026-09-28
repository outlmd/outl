//! What `doctor` sees in `ops/` without booting a tree.
//!
//! `JsonlStorage::open` skips a malformed record on purpose, so one
//! torn tail line can never lock a user out of their workspace — and so
//! nothing but this sweep can name *which* line was lost. The offset
//! indexes are the same shape of question one level down: a cache that
//! disagrees with its `.jsonl` is silent until it is asked.

use super::*;

// ------------------------------------------------------- corrupt lines

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

// ---------------------------------------------------------- offset index

#[test]
fn an_offset_index_pointing_past_eof_is_flagged() {
    use outl_core::hlc::HlcGenerator;
    use outl_core::storage::sidecar::{self, SidecarKind};
    use outl_core::storage::{OffsetIndex, PageScope};

    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["hello"]);
    let cfg = crate::workspace_layout::read_config(&paths).unwrap();
    let actor = cfg.actor().unwrap();

    let mut index = OffsetIndex::new();
    index.insert(HlcGenerator::new(actor).next(), 999_999_999);
    index
        .save(&sidecar::path_for(
            &paths.ops,
            actor,
            &PageScope::Global,
            SidecarKind::Offset,
        ))
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
    use outl_core::storage::sidecar::{self, SidecarKind};
    use outl_core::storage::{OffsetIndex, PageScope};

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
        .save(&sidecar::path_for(
            &paths.ops,
            actor,
            &PageScope::Global,
            SidecarKind::Offset,
        ))
        .expect("write idx");

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        has(&report, "not on a record boundary"),
        "expected a mid-line offset warning, got: {:#?}",
        messages(&report)
    );
}
