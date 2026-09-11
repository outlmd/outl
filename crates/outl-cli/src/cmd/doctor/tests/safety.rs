//! What the doctor is allowed to **write**.
//!
//! The sibling module asserts that every check *sees* its defect. This
//! one asserts the other half, which is the half that can cost a user
//! their notes: a diagnostic that writes where it promised not to, and a
//! repair that trusts a source of truth it just declared broken.
//!
//! Every test here was written against a real defect found by audit, not
//! against a hypothetical. They are the regression net for:
//!
//! - `--repair` re-projecting a `.md` out of a **truncated** tree,
//! - `doctor` (no flags) writing index sidecars into `ops/`,
//! - `doctor` (no flags) appending to `.outl/orphans.log`, forever,
//! - `--repair` re-projecting more content than the user authorised.

use super::super::repair;
use super::*;

// ------------------------------------------- a damaged log has no authority

/// **The one that destroys data.**
///
/// `JsonlStorage` skips unreadable records by design, so a torn `ops/`
/// file replays a **truncated** tree that looks entirely healthy from
/// the inside. A `.md` that is still a faithful projection of the whole
/// page then compares as "stale" against that shorter render — and the
/// old `--repair` happily overwrote it, printing
/// `err: N op-log line(s) carry no usable op` three lines above
/// `done reproject pages/notes.md`.
///
/// The `.md` here holds two blocks and the log knows about one. Nothing
/// may write that file.
#[test]
fn a_torn_op_log_never_lets_repair_overwrite_a_good_md() {
    let (_dir, root, paths) = fresh();
    let page = seed_page(&root, "notes", &["first"]);
    let ops = ops_file(&paths);
    let healthy_lines = std::fs::read_to_string(&ops).unwrap().lines().count();

    // Second session: add a block and project it, so the `.md` and its
    // sidecar are a faithful, complete projection of the whole page.
    {
        let mut ctx = crate::ws::open(&root).expect("open");
        outl_actions::append_block(&mut ctx.workspace, &ctx.hlc, Some(page), Some("second"))
            .expect("append");
        outl_actions::apply_page_md_with_sidecar(&ctx.workspace, &root, page).expect("project");
    }
    let md = paths.pages.join("notes.md");
    let before = std::fs::read_to_string(&md).unwrap();
    assert!(
        before.contains("first") && before.contains("second"),
        "the `.md` must start out holding both blocks, got: {before:?}"
    );

    // Now tear every op the second session appended — an iCloud write
    // that landed half a file. The storage layer skips them, the tree
    // replays back to one block, the `.md` on disk is untouched.
    let text = std::fs::read_to_string(&ops).unwrap();
    let torn: String = text
        .lines()
        .enumerate()
        .map(|(i, line)| {
            if i < healthy_lines {
                format!("{line}\n")
            } else {
                "{ torn by a partial sync }\n".to_string()
            }
        })
        .collect();
    assert_ne!(torn, text, "the second session must have appended ops");
    std::fs::write(&ops, torn).unwrap();

    let report = collect(&root, true).expect("doctor --repair runs on a torn log");

    assert_eq!(
        std::fs::read_to_string(&md).unwrap(),
        before,
        "a damaged op log has no authority to overwrite its own projection"
    );
    assert!(
        !report.repairable.iter().any(|r| r.contains("re-project")),
        "re-projection must not even be offered while the log is torn: {:?}",
        report.repairable
    );
    assert!(
        has(&report, "page repair(s) suppressed"),
        "the user must be told why the repair did not run, got: {:#?}",
        messages(&report)
    );
    assert!(
        has(&report, "Recover the op log first"),
        "the suppression message must say what to do next, got: {:#?}",
        messages(&report)
    );
}

/// The gate must not fire on a healthy log, or drift never gets fixed.
///
/// Same drift as `a_stale_md_is_flagged_and_repair_reprojects_it_with_a_backup`,
/// asserted from the gate's side: an intact log still authorises the
/// write.
#[test]
fn a_healthy_op_log_still_authorises_reprojection() {
    let (_dir, root, paths) = fresh();
    let page = seed_page(&root, "notes", &["first"]);
    {
        let mut ctx = crate::ws::open(&root).expect("open");
        outl_actions::append_block(&mut ctx.workspace, &ctx.hlc, Some(page), Some("second"))
            .expect("append");
    }

    let report = collect(&root, true).expect("doctor --repair runs");
    assert!(
        !has(&report, "page repair(s) suppressed"),
        "nothing is wrong with this log: {:#?}",
        messages(&report)
    );
    assert!(
        std::fs::read_to_string(paths.pages.join("notes.md"))
            .unwrap()
            .contains("second"),
        "a healthy log must still re-project its drifted `.md`"
    );
}

/// A forked `ops-*.jsonl` (Syncthing wrote both sides of a concurrent
/// write) is the same hazard as a torn line: the ops in the fork never
/// reached the tree, so the tree is short.
#[test]
fn a_forked_op_log_also_blocks_reprojection() {
    let (_dir, root, paths) = fresh();
    let page = seed_page(&root, "notes", &["first"]);
    {
        let mut ctx = crate::ws::open(&root).expect("open");
        outl_actions::append_block(&mut ctx.workspace, &ctx.hlc, Some(page), Some("second"))
            .expect("append");
    }
    let ops = ops_file(&paths);
    let forked = paths.ops.join(format!(
        "{}.sync-conflict-20260805-101500-ABCDEFG.jsonl",
        ops.file_stem().and_then(|s| s.to_str()).unwrap()
    ));
    std::fs::copy(&ops, &forked).unwrap();

    let md = paths.pages.join("notes.md");
    let before = std::fs::read_to_string(&md).unwrap();

    let report = collect(&root, true).expect("doctor --repair runs");
    assert_eq!(
        std::fs::read_to_string(&md).unwrap(),
        before,
        "a forked op log is an incomplete one; it may not authorise a rewrite"
    );
    assert!(
        has(&report, "page repair(s) suppressed"),
        "expected the suppression notice, got: {:#?}",
        messages(&report)
    );
}

/// Rebuilding a sidecar also writes a file derived from the tree, and it
/// stamps that tree's block ids into `last_synced_hash`. Same gate.
#[test]
fn a_torn_op_log_also_blocks_sidecar_rebuild() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["hello"]);
    let md = paths.pages.join("notes.md");
    let sidecar = outl_md::sidecar::sidecar_path_for(&md);
    std::fs::remove_file(&sidecar).unwrap();

    append_bytes(&ops_file(&paths), b"{ torn }\n");

    let report = collect(&root, true).expect("doctor --repair runs");
    assert!(
        !report
            .repairable
            .iter()
            .any(|r| r.contains("rebuild the sidecar")),
        "a sidecar rebuild is a tree-derived write too: {:?}",
        report.repairable
    );
    assert!(
        !sidecar.exists(),
        "nothing derived from a torn log may be written"
    );
}

// ------------------------------------------------- doctor stays out of ops/

/// The promise is "`doctor` never writes into `ops/`", and it used to be
/// false in the *default* mode: `JsonlStorage::open` runs `create_dir_all`
/// plus a `reload` that persists rebuilt `.ops-<actor>.idx` /
/// `.ops-<actor>.nodes.idx` sidecars — for peer actors too.
///
/// Compares the **whole directory**, names and bytes. The old version of
/// this assertion diffed a single `.jsonl` while promising more.
#[test]
fn a_read_only_run_writes_nothing_into_ops() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["hello"]);
    let before = dir_snapshot(&paths.ops);
    assert!(
        !before.is_empty(),
        "the seeded workspace must have an op log"
    );

    collect(&root, false).expect("doctor runs");

    assert_eq!(
        dir_snapshot(&paths.ops),
        before,
        "a read-only doctor run must leave ops/ byte-identical — including the index sidecars"
    );
}

/// Two runs, because the first is the one that would rebuild a missing
/// index and the second is the one that would rebuild a *stale* one.
#[test]
fn repeated_read_only_runs_write_nothing_into_ops() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["hello"]);
    let before = dir_snapshot(&paths.ops);

    collect(&root, false).expect("first run");
    collect(&root, false).expect("second run");

    assert_eq!(dir_snapshot(&paths.ops), before, "still byte-identical");
}

/// The op log is the source of truth and repair only ever writes
/// projections. A repair pass that touched `ops/` would be rewriting
/// history on one device and diverging it from every peer.
#[test]
fn repair_never_touches_the_op_log() {
    let (_dir, root, paths) = fresh();
    let page = seed_page(&root, "notes", &["first"]);
    {
        let mut ctx = crate::ws::open(&root).expect("open");
        outl_actions::append_block(&mut ctx.workspace, &ctx.hlc, Some(page), Some("second"))
            .expect("append");
    }
    let before = dir_snapshot(&paths.ops);

    let repaired = collect(&root, true).expect("doctor --repair runs");
    assert!(
        repaired.repair.is_some(),
        "there was drift, so repair must have run"
    );
    assert_eq!(
        dir_snapshot(&paths.ops),
        before,
        "`--repair` must not write a single byte into ops/ — the whole directory, not just the log"
    );
}

// ---------------------------------------------------------- orphans.log

/// `.outl/orphans.log` is where level-3 matching orphans live — the
/// record of blocks that could not be matched back into the op log, and
/// the most valuable thing that file ever holds.
///
/// The doctor used to append a `parse-warning` row per warning on every
/// run, read-only included, and the MCP `outl_workspace_doctor` tool
/// (documented "never repairs") went through the same path. On a freshly
/// imported graph, where a leading `# heading` and free prose are the
/// normal shape of an imported page, that is thousands of rows per run.
#[test]
fn a_read_only_run_never_appends_to_the_orphans_log() {
    let (_dir, root, paths) = fresh();
    std::fs::write(
        paths.pages.join("imported.md"),
        "# a heading\n\nfree prose\n\n- a real bullet\n",
    )
    .unwrap();

    let report = collect(&root, false).expect("doctor runs");
    assert!(
        has(&report, "outside outl dialect"),
        "the warning itself must still be reported, got: {:#?}",
        messages(&report)
    );

    let log = std::fs::read_to_string(&paths.orphans).unwrap_or_default();
    assert!(
        !log.contains("parse-warning"),
        "a read-only diagnostic must not write to the user's workspace, got: {log:?}"
    );
}

/// Even under `--repair`, the same defect must not accumulate a row per
/// run. Three runs, one row.
#[test]
fn repair_logs_each_parse_warning_exactly_once() {
    let (_dir, root, paths) = fresh();
    std::fs::write(
        paths.pages.join("imported.md"),
        "# a heading\n\nfree prose\n\n- a real bullet\n",
    )
    .unwrap();

    for _ in 0..3 {
        collect(&root, true).expect("doctor --repair runs");
    }

    let log = std::fs::read_to_string(&paths.orphans).unwrap_or_default();
    let rows = log
        .lines()
        .filter(|l| l.starts_with("parse-warning"))
        .count();
    let distinct: std::collections::BTreeSet<&str> = log
        .lines()
        .filter(|l| l.starts_with("parse-warning"))
        // `parse-warning <iso> <path>:<line> <kind> <raw>` — drop the
        // stamp, keep the identity.
        .filter_map(|l| l.split_once("parse-warning ").map(|(_, rest)| rest))
        .filter_map(|rest| rest.split_once(' ').map(|(_, rest)| rest))
        .collect();
    assert!(rows > 0, "the warnings must be logged at all, got: {log:?}");
    assert_eq!(
        rows,
        distinct.len(),
        "three runs must not triple the rows, got {rows} row(s): {log:?}"
    );
}

// -------------------------------------------------------- backup pruning

/// `.outl/repair-backup/` used to grow without bound. `.outl/` is
/// dot-prefixed, so iCloud drops it — but Syncthing, Dropbox and a
/// shared volume all replicate it, and on a 66k-block graph every
/// `--repair` generation is a full copy of every `.md` it touched.
///
/// The policy is a pure function so both guards can be asserted without
/// backdating a directory on disk. What it must guarantee: a young
/// generation never goes, however many there are; a recent generation
/// never goes, however old the rest are; and metadata we could not read
/// keeps the backup rather than losing it.
#[test]
fn backup_pruning_needs_both_age_and_surplus() {
    use std::time::{Duration, SystemTime};

    let now = SystemTime::now();
    let ancient = Some(now - Duration::from_secs(60 * 60 * 24 * 400));
    let yesterday = Some(now - Duration::from_secs(60 * 60 * 24));
    let gen_at = |i: usize| PathBuf::from(format!("/backups/2020010{}T0000{:02}", i / 10, i % 10));

    // Under the keep floor: nothing goes, whatever its age.
    let mut few: Vec<_> = (0..5).map(|i| (gen_at(i), ancient)).collect();
    assert!(
        repair::prunable_backups(&mut few, now).is_empty(),
        "the newest generations are kept whatever the TTL says"
    );

    // Over the floor, but all young: the age guard alone holds.
    let mut young: Vec<_> = (0..25).map(|i| (gen_at(i), yesterday)).collect();
    assert!(
        repair::prunable_backups(&mut young, now).is_empty(),
        "a young backup is never a candidate, however many there are"
    );

    // Over the floor and old: only the surplus goes, oldest-first.
    let mut old: Vec<_> = (0..25).map(|i| (gen_at(i), ancient)).collect();
    let pruned = repair::prunable_backups(&mut old, now);
    assert_eq!(pruned.len(), 15, "25 generations, 10 kept: {pruned:?}");
    assert!(
        !pruned.contains(&gen_at(24)),
        "the newest generation — the one this run just wrote — must never prune itself"
    );
    assert!(
        pruned.contains(&gen_at(0)),
        "the oldest surplus generation is the first to go"
    );

    // Unreadable metadata keeps the backup.
    let mut unknown: Vec<_> = (0..25).map(|i| (gen_at(i), None)).collect();
    assert!(
        repair::prunable_backups(&mut unknown, now).is_empty(),
        "a backup whose age we cannot establish is never deleted"
    );
}

/// And the wiring: a `--repair` on a workspace with a handful of
/// generations prunes nothing and keeps its own backup.
#[test]
fn repair_keeps_its_own_backup_generation() {
    let (_dir, root, paths) = fresh();
    let page = seed_page(&root, "notes", &["first"]);
    {
        let mut ctx = crate::ws::open(&root).expect("open");
        outl_actions::append_block(&mut ctx.workspace, &ctx.hlc, Some(page), Some("second"))
            .expect("append");
    }

    let backups = paths.dot_outl.join("repair-backup");
    for i in 0..4 {
        std::fs::create_dir_all(backups.join(format!("2020010{i}T000000"))).unwrap();
    }

    let report = collect(&root, true).expect("doctor --repair runs");
    let rep = report.repair.expect("a repair report");
    assert!(
        !rep.actions.iter().any(|a| a.kind == "prune_backup"),
        "five generations is under the keep floor: {:#?}",
        rep.actions
    );
    assert!(
        Path::new(&rep.backup_dir).exists(),
        "the generation this run wrote must survive its own prune"
    );
}

/// Backdate `dir`'s mtime so the TTL guard sees it as ancient.
///
/// `File::set_times` on a directory fd is `futimens(2)`, which needs
/// ownership rather than a writable handle — true for a `TempDir` we
/// just created, on every platform CI runs.
/// **The prune that never ran.**
///
/// `Plan::is_empty` used to ignore prunables and the prune itself was
/// discovered *inside* `repair::run` — so a workspace with nothing else
/// wrong never reached `run` at all, and `.outl/repair-backup/`
/// accumulated forever on exactly the graph the pruning exists for. The
/// prune was also invisible in `repairable[]`, contradicting the promise
/// that `--repair` prints what it will do.
#[test]
fn a_clean_workspace_still_prunes_its_stale_backups() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["first"]);

    // 25 generations: 15 over the keep floor, all past the TTL.
    let backups = paths.dot_outl.join("repair-backup");
    let gens: Vec<PathBuf> = (0..25)
        .map(|i| backups.join(format!("2020{:04}T000000", 100 + i)))
        .collect();
    for gen in &gens {
        std::fs::create_dir_all(gen).unwrap();
        backdate(gen, 400);
    }

    // Read-only first: the prune has to be announced, not just done.
    let dry = collect(&root, false).expect("doctor runs");
    let announced: Vec<&String> = dry
        .repairable
        .iter()
        .filter(|l| l.starts_with("prune stale backup generation"))
        .collect();
    assert_eq!(
        announced.len(),
        15,
        "every prune must show up in `repairable[]`: {:#?}",
        dry.repairable
    );
    assert!(
        dry.repair.is_none() && gens.iter().all(|g| g.exists()),
        "a read-only run never deletes anything"
    );

    let report = collect(&root, true).expect("doctor --repair runs");
    let rep = report
        .repair
        .expect("a prune alone is enough work to run `--repair`");
    let pruned: Vec<&repair::RepairAction> = rep
        .actions
        .iter()
        .filter(|a| a.kind == "prune_backup")
        .collect();
    assert_eq!(pruned.len(), 15, "25 generations, 10 kept: {pruned:#?}");
    assert!(pruned.iter().all(|a| a.ok), "{pruned:#?}");
    assert!(
        gens.iter().take(15).all(|g| !g.exists()),
        "the surplus generations are gone from disk"
    );
    assert!(
        gens.iter().skip(15).all(|g| g.exists()),
        "the newest 10 stay, whatever their age"
    );
}
