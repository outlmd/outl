//! End-to-end tests for `outl reconcile` — the `.md` side of the two
//! recovery routes. Its mirror, `outl recover`, is in `recover_cmd.rs`;
//! both share `recovery_support/`.
//!
//! ## Why these drive the binary instead of the library
//!
//! The library side of both is already proven — `outl-md`'s
//! `unlogged.rs` / `matching/guard.rs` / `reconcile.rs` and
//! `outl-actions`' `recover.rs` all sit above 92% with their own unit
//! batteries. What was not covered is **the command a human types**.
//!
//! Root `CLAUDE.md` invariant 8 names exactly two routes back to the
//! 1,426 lines of real user content a mirrored-pair reconciliation bug
//! deleted: `outl recover` (from the op log) and
//! `outl reconcile --ahead-of-log` (from the `.md`). Both only ever run
//! on a day that has already gone wrong. If argument parsing, output,
//! exit codes, or the additive-only contract regress, the user runs the
//! command, sees nothing useful, and concludes the data is gone — with a
//! green library test suite the whole time.
//!
//! So every test here spawns the real `outl` binary over a `TempDir`
//! workspace, and asserts on what reached stdout and on the bytes left
//! on disk.
//!
//! ## Deny outnumbers allow, on purpose
//!
//! A recovery tool is only as good as what it refuses. The dry run that
//! writes nothing, the guard that stops a bulk delete, the second
//! `--apply` that does not double-restore, and the page that was skipped
//! rather than judged are the cases that decide whether a user's content
//! survives — so they carry more tests here than the happy paths do.

mod recovery_support;

use recovery_support::{blank_sidecar_text, make_ahead_of_log, Ws};
use std::fs;

/// The `--ahead-of-log` happy path: a page holding content that exists
/// in no op is found and its content enters the log.
#[test]
fn ahead_of_log_brings_unlogged_content_into_the_op_log() {
    let ws = Ws::new();
    let root = ws.root_str();
    ws.seed_block("notes", "logged line");
    make_ahead_of_log(&ws.md("pages/notes.md"), "- unlogged line\n");

    let stdout = ws.ok(&["reconcile", &root, "--ahead-of-log"]);
    assert!(
        stdout.contains("1 page(s) hold 1 line(s)"),
        "the report must name the page count and the line count:\n{stdout}"
    );
    assert!(
        stdout.contains("notes"),
        "the report must name the page:\n{stdout}"
    );

    let found = ws.json(&["search", "unlogged line", "--json", "--workspace", &root]);
    assert!(
        serde_json::to_string(&found)
            .expect("serialize")
            .contains("unlogged line"),
        "the line must be in the op log afterwards: {found}"
    );

    let again = ws.ok(&["reconcile", &root, "--ahead-of-log"]);
    assert!(
        again.contains("no page holds content outside the op log"),
        "a second pass must find the workspace clean:\n{again}"
    );
}

/// The escape hatch, through the command line, in plain mode.
///
/// The guard refuses a pass that would trash most of a page — correct,
/// because a truncated `.md` (an undownloaded iCloud placeholder, a
/// half-flushed write) is indistinguishable from a real bulk delete by
/// shape. A guard with no escape hatch is a wall (root `CLAUDE.md`
/// invariant 9), and this is the hatch.
#[test]
fn allow_bulk_delete_applies_a_deletion_the_ordinary_pass_refused() {
    let ws = Ws::new();
    let root = ws.root_str();
    ws.seed_wide_page("wide", 60);
    fs::write(ws.md("pages/wide.md"), "- keeper\n").expect("truncate .md");

    let refused = ws.run(&["serve", &root, "--once"]);
    assert!(
        !refused.status.success(),
        "precondition: the ordinary reconcile must refuse this deletion"
    );

    let stdout = ws.ok(&["reconcile", &root, "--allow-bulk-delete"]);
    assert!(
        stdout.contains("60 block(s) moved to the trash"),
        "the opt-in must apply the whole deletion:\n{stdout}"
    );
    assert!(
        stdout.contains("Deleted blocks are in the trash, not gone"),
        "the user must be told the blocks are recoverable:\n{stdout}"
    );
}

/// **Deny:** without the flag, the guard refuses and the page is left
/// exactly as the log has it.
///
/// Asserted through `--ahead-of-log`, because that is the mode where the
/// refusal has to travel through this command's own error handling: the
/// page is selected, the reconcile fails, and the run must exit non-zero
/// rather than report a clean migration.
#[test]
fn the_orphan_guard_refuses_a_bulk_delete_without_the_flag() {
    let ws = Ws::new();
    let root = ws.root_str();
    ws.seed_wide_page("wide", 60);
    fs::write(ws.md("pages/wide.md"), "- keeper\n").expect("truncate .md");
    make_ahead_of_log(&ws.md("pages/wide.md"), "- brand new unlogged line\n");

    let out = ws.run(&["reconcile", &root, "--ahead-of-log"]);
    assert!(
        !out.status.success(),
        "a refused page still holds unlogged content — the run must not report success"
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("refusing to delete"),
        "the refusal has to reach the user:\n{combined}"
    );
    assert!(
        combined.contains("1 page(s) still hold content outside the op log"),
        "the summary must name what is still outstanding:\n{combined}"
    );
}

/// **Allow:** the flag reaches `--ahead-of-log` too.
///
/// Pinned in both directions because an earlier bug had it wired into
/// only one mode. That version passed a clap test and was useless for
/// every page it existed to unblock.
#[test]
fn allow_bulk_delete_also_reaches_ahead_of_log_mode() {
    let ws = Ws::new();
    let root = ws.root_str();
    ws.seed_wide_page("wide", 60);
    fs::write(ws.md("pages/wide.md"), "- keeper\n").expect("truncate .md");
    make_ahead_of_log(&ws.md("pages/wide.md"), "- brand new unlogged line\n");

    let stdout = ws.ok(&["reconcile", &root, "--ahead-of-log", "--allow-bulk-delete"]);
    assert!(
        stdout.contains("reconciled 1 page(s)") && stdout.contains("0 failed"),
        "the flag must unblock the ahead-of-log pass too:\n{stdout}"
    );
}

/// **Deny:** an `.md` that exists and cannot be read is counted and
/// named, never folded into "clean".
///
/// This is the defect these tests were written for. Both modes used to
/// drop such a page silently, so a workspace where every page was
/// skipped printed "no page holds content outside the op log" and exited
/// 0 — a refusal that never reached the user, on the command they run
/// *after* a silent refusal already cost them content.
#[test]
#[cfg(unix)]
fn an_unreadable_md_is_counted_and_named_never_called_clean() {
    use std::os::unix::fs::PermissionsExt;

    let ws = Ws::new();
    let root = ws.root_str();
    ws.seed_block("locked", "some content");
    let md = ws.md("pages/locked.md");
    fs::set_permissions(&md, fs::Permissions::from_mode(0o000)).expect("chmod");
    if fs::read_to_string(&md).is_ok() {
        // Running as root, where mode 0o000 does not deny anything. The
        // fixture cannot be built, so assert nothing rather than assert
        // something weaker — a test that quietly checks less than its
        // name claims is worse than one that says it did not run.
        fs::set_permissions(&md, fs::Permissions::from_mode(0o644)).expect("chmod back");
        eprintln!("skipped: this user can read a 0o000 file (root?)");
        return;
    }

    let stdout = ws.ok(&["reconcile", &root, "--ahead-of-log"]);

    // Restore before any assertion so a failure still leaves a
    // deletable TempDir.
    fs::set_permissions(&md, fs::Permissions::from_mode(0o644)).expect("chmod back");

    assert!(
        stdout.contains("could NOT be judged"),
        "a skipped page must be reported:\n{stdout}"
    );
    assert!(
        stdout.contains("locked") && stdout.contains("`.md` could not be read"),
        "the report must name the page and why it was skipped:\n{stdout}"
    );
    assert!(
        stdout.contains("None of these is clean"),
        "the report must say a skip is not a clean result:\n{stdout}"
    );
}

/// **Deny:** a sidecar that will not parse is counted and named.
///
/// Without one there is no record of what the log held, so no question
/// this command asks can be answered about the page. That is a third
/// state — not clean, not reconciled — and it needs its own line.
#[test]
fn an_unparseable_sidecar_is_counted_and_named() {
    let ws = Ws::new();
    let root = ws.root_str();
    ws.seed_block("broken", "some content");
    fs::write(ws.md("pages/broken.outl"), "this is not json").expect("corrupt sidecar");

    let stdout = ws.ok(&["reconcile", &root, "--ahead-of-log"]);
    assert!(
        stdout.contains("broken — its sidecar will not parse"),
        "a page with a corrupt sidecar must be named:\n{stdout}"
    );
    assert!(
        !stdout.contains("broken —  "),
        "the reason must be a sentence, not blank:\n{stdout}"
    );
}

/// **Deny:** the same for a sidecar that parses and records no block
/// text — a pre-0.11 sidecar. It lists blocks, so it is not empty, but
/// it cannot say what the op log held.
#[test]
fn a_sidecar_that_cannot_answer_is_counted_and_named() {
    let ws = Ws::new();
    let root = ws.root_str();
    ws.seed_block("stale", "real content on disk");
    blank_sidecar_text(&ws.md("pages/stale.md"));

    let stdout = ws.ok(&["reconcile", &root, "--ahead-of-log"]);
    assert!(
        stdout.contains("stale — its sidecar records no block text"),
        "a sidecar that cannot answer must be named, not skipped in silence:\n{stdout}"
    );
}

/// **Deny (a false positive):** an *empty* page whose sidecar cannot
/// answer is not reported.
///
/// `outl init` leaves two such pages behind — the fresh journal and the
/// template page each hold one empty block, so their sidecars record no
/// text. Their `.md` holds no text either, so nothing on disk could be
/// outside the log. Reporting them would make the skip list mostly noise
/// on every workspace, and a guard that fires on the happy path only
/// teaches the user to skim past the entries that matter.
#[test]
fn an_empty_page_is_not_reported_as_unjudged() {
    let ws = Ws::new();
    let root = ws.root_str();

    let stdout = ws.ok(&["reconcile", &root, "--ahead-of-log"]);
    assert!(
        !stdout.contains("could NOT be judged"),
        "a freshly initialised workspace must produce no skip list:\n{stdout}"
    );
    assert!(
        stdout.contains("no page holds content outside the op log"),
        "and it must read as clean:\n{stdout}"
    );
}

/// **Deny (a false positive):** a page with no `.md` at all is not a
/// skip either.
///
/// There is no file, so it cannot hold content the log lacks. That is
/// the ordinary state of every page on a freshly paired device, and
/// reporting thousands of them would bury the handful that mean
/// something.
#[test]
fn a_page_with_no_md_is_not_reported_as_unjudged() {
    let ws = Ws::new();
    let root = ws.root_str();
    ws.seed_block("ghost", "content");
    fs::remove_file(ws.md("pages/ghost.md")).expect("remove .md");
    fs::remove_file(ws.md("pages/ghost.outl")).expect("remove sidecar");

    let stdout = ws.ok(&["reconcile", &root, "--ahead-of-log"]);
    assert!(
        !stdout.contains("could NOT be judged"),
        "an unprojected page is answerable, not unjudged:\n{stdout}"
    );
}

/// **Deny:** the skip report reaches the user in plain
/// `--allow-bulk-delete` mode too, not only under `--ahead-of-log`.
///
/// The two modes select opposite sets of pages, and an earlier bug in
/// this file wired a flag into one of them only. A report is no
/// different: a skip that is visible in one mode and silent in the other
/// is still a silence.
#[test]
fn the_skip_report_reaches_the_bulk_delete_mode_too() {
    let ws = Ws::new();
    let root = ws.root_str();
    ws.seed_block("broken", "some content");
    fs::write(ws.md("pages/broken.outl"), "this is not json").expect("corrupt sidecar");

    let stdout = ws.ok(&["reconcile", &root, "--allow-bulk-delete"]);
    assert!(
        stdout.contains("broken — its sidecar will not parse"),
        "the skip must be reported in plain mode as well:\n{stdout}"
    );
}

/// **Deny:** `reconcile` on a directory that is not a workspace errors
/// with a non-zero exit.
#[test]
fn reconcile_on_a_directory_that_is_not_a_workspace_fails_loudly() {
    let ws = Ws::new();
    let elsewhere = ws.dir.path().join("not-a-workspace");
    fs::create_dir_all(&elsewhere).expect("mkdir");
    let elsewhere = elsewhere.to_str().expect("utf-8 path");

    let out = ws.run(&["reconcile", elsewhere, "--ahead-of-log"]);
    assert!(!out.status.success(), "`--ahead-of-log` must not exit 0");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("NO_WORKSPACE"),
        "the error must name the problem"
    );

    let out = ws.run(&["reconcile", elsewhere, "--allow-bulk-delete"]);
    assert!(
        !out.status.success(),
        "`--allow-bulk-delete` must not exit 0 either"
    );
}

/// The no-flag mode is a read-only listing of `.outl/orphans.log`, and
/// on a fresh workspace it has nothing to list.
#[test]
fn plain_reconcile_lists_orphans_and_writes_nothing() {
    let ws = Ws::new();
    let root = ws.root_str();
    ws.seed_block("notes", "content");

    let ops_before = ws.op_log();
    let stdout = ws.ok(&["reconcile", &root]);
    assert!(
        stdout.contains("no orphans recorded"),
        "a clean workspace has no orphans to resolve:\n{stdout}"
    );
    assert_eq!(
        ws.op_log(),
        ops_before,
        "the listing mode must never write an op"
    );
}
