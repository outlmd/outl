//! `outl doctor` and the dead `ops/` index caches.
//!
//! This is the one place `--repair` deletes inside `ops/`, a directory
//! the command otherwise promises never to touch, so the tests here are
//! as much about the boundary of that exception as about the deletion.

use super::*;

/// Plant one of each dead generation next to a real op log.
fn plant_debris(paths: &Paths) -> (PathBuf, PathBuf, PathBuf) {
    let log = ops_file(paths);
    let actor = log
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("ops-"))
        .and_then(|n| n.strip_suffix(".jsonl"))
        .expect("actor id in the ops filename")
        .to_string();

    let legacy = paths.ops.join(format!("ops-{actor}.idx"));
    let legacy_nodes = paths.ops.join(format!("ops-{actor}.nodes.idx"));
    let scratch = paths
        .ops
        .join(format!(".ops-{actor}.idx.tmp.{}", ulid::Ulid::new()));
    std::fs::write(&legacy, "offsets nothing reads\n").expect("write legacy idx");
    std::fs::write(&legacy_nodes, "offsets nothing reads\n").expect("write legacy nodes idx");
    std::fs::write(&scratch, "half a write\n").expect("write scratch");
    backdate(&scratch, 2);
    (legacy, legacy_nodes, scratch)
}

/// The read-only run names them and offers the deletion, and — the part
/// that matters — leaves every one of them on disk.
#[test]
fn dead_index_sidecars_are_reported_without_being_touched() {
    let (_tmp, root, paths) = fresh();
    seed_page(&root, "home", &["one", "two"]);
    let (legacy, legacy_nodes, scratch) = plant_debris(&paths);

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        has(&report, "dead index sidecar file(s)"),
        "the read-only run must name them: {:?}",
        messages(&report)
    );
    assert!(
        report
            .repairable
            .iter()
            .any(|line| line.contains("delete index sidecar") && line.contains("pre-dotfile")),
        "and offer the deletion with the GC's own reason: {:?}",
        report.repairable
    );
    for path in [&legacy, &legacy_nodes, &scratch] {
        assert!(
            path.exists(),
            "{} must survive a read-only run",
            path.display()
        );
    }
}

/// `--repair` collects them, and the live sidecars — the ones the next
/// boot reads — stay.
#[test]
fn repair_collects_the_dead_generations_and_keeps_the_live_ones() {
    let (_tmp, root, paths) = fresh();
    seed_page(&root, "home", &["one", "two"]);
    let (legacy, legacy_nodes, scratch) = plant_debris(&paths);

    let report = collect(&root, true).expect("doctor runs");
    for path in [&legacy, &legacy_nodes, &scratch] {
        assert!(
            !path.exists(),
            "{} is a cache nothing reads and must be collected",
            path.display()
        );
    }

    let live: Vec<PathBuf> = std::fs::read_dir(&paths.ops)
        .expect("read ops")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(".ops-") && n.ends_with(".idx"))
        })
        .collect();
    assert!(
        !live.is_empty(),
        "the dotted sidecars the next boot reads must survive"
    );
    assert!(
        report
            .repair
            .as_ref()
            .expect("a repair report")
            .actions
            .iter()
            .any(|a| a.kind == "prune_index_sidecar" && a.ok),
        "and the deletion is reported as its own action kind"
    );
}

/// An index sidecar is the only thing in `ops/` `--repair` may remove.
/// Everything else — above all a peer's op log, and this device's — must
/// come back byte-identical, which is the promise `ops_guard` exists for.
#[test]
fn repair_still_leaves_every_op_log_in_ops_untouched() {
    let (_tmp, root, paths) = fresh();
    seed_page(&root, "home", &["one", "two"]);
    plant_debris(&paths);
    // A peer's log, the file the guard must never treat as debris.
    let peer = paths.ops.join(format!("ops-{}.jsonl", ulid::Ulid::new()));
    std::fs::write(&peer, "").expect("write peer log");

    let logs_before: std::collections::BTreeMap<PathBuf, Vec<u8>> = dir_snapshot(&paths.ops)
        .into_iter()
        .filter(|(p, _)| p.extension().and_then(|e| e.to_str()) == Some("jsonl"))
        .collect();

    collect(&root, true).expect("doctor runs");

    let logs_after: std::collections::BTreeMap<PathBuf, Vec<u8>> = dir_snapshot(&paths.ops)
        .into_iter()
        .filter(|(p, _)| p.extension().and_then(|e| e.to_str()) == Some("jsonl"))
        .collect();
    assert_eq!(
        logs_before, logs_after,
        "`--repair` may collect a dead cache in `ops/` and nothing else"
    );
}

/// A workspace with no debris must not produce a finding — a check that
/// fires on a healthy graph is as bad as one that stays silent on a
/// broken one.
#[test]
fn a_clean_ops_directory_produces_no_sidecar_finding() {
    let (_tmp, root, _paths) = fresh();
    seed_page(&root, "home", &["one"]);

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        !has(&report, "dead index sidecar file(s)"),
        "{:?}",
        messages(&report)
    );
    assert!(
        !report
            .repairable
            .iter()
            .any(|l| l.contains("delete index sidecar")),
        "{:?}",
        report.repairable
    );
}

/// The lock files are counted, never offered for deletion. They are
/// arbitration state: removing one a live process holds lets a second
/// process claim the same actor and append to the same `.jsonl`.
#[test]
fn actor_write_locks_are_counted_and_never_collected() {
    let (_tmp, root, paths) = fresh();
    seed_page(&root, "home", &["one"]);
    let orphan = paths.ops.join(format!(".lock-{}", ulid::Ulid::new()));
    std::fs::write(&orphan, "").expect("write orphan lock");
    backdate(&orphan, 400);

    let report = collect(&root, true).expect("doctor runs");
    assert!(
        has(&report, "per-actor write lock file(s)"),
        "the count is a real signal about ephemeral actors: {:?}",
        messages(&report)
    );
    assert!(
        !report.repairable.iter().any(|l| l.contains(".lock-")),
        "a lock is not a cache and is never offered for deletion: {:?}",
        report.repairable
    );
    assert!(orphan.exists(), "and it is still there afterwards");
}
