//! End-to-end tests for `outl recover` — the op-log side of the two
//! recovery routes. Its mirror, `outl reconcile`, is in
//! `reconcile_cmd.rs`; both share `recovery_support/`.
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

use recovery_support::{write_md_hash_faithful, Ws};
use std::fs;

/// The whole point of the command: a block truncated by an `Op::Edit`
/// whose predecessor is still in the log is found, and `--apply` puts
/// the text back.
#[test]
fn a_truncated_block_is_reported_and_restored_by_apply() {
    let ws = Ws::new();
    let root = ws.root_str();
    let id = ws.seed_block("notes", "line one\nline two\nline three");
    ws.json(&[
        "block",
        "update",
        &id,
        "--text",
        "line one",
        "--json",
        "--workspace",
        &root,
    ]);

    let listing = ws.ok(&["recover", &root]);
    assert!(
        listing.contains("1 block(s) hold 2 line(s)"),
        "the dry run must name the block and the size of the loss:\n{listing}"
    );
    assert!(
        listing.contains("line two") && listing.contains("line three"),
        "the dry run must show the text that would come back:\n{listing}"
    );

    let applied = ws.ok(&["recover", &root, "--apply"]);
    assert!(
        applied.contains("restored 1 block(s), 0 failed"),
        "--apply must report the restore:\n{applied}"
    );

    let text = ws.json(&["block", "get", &id, "--json", "--workspace", &root]);
    assert_eq!(
        text["text"].as_str().expect("block text"),
        "line one\nline two\nline three",
        "the block must hold the full pre-truncation text again"
    );
    let md = fs::read_to_string(ws.md("pages/notes.md")).expect("read .md");
    assert!(
        md.contains("line three"),
        "the restore must re-project the page:\n{md}"
    );
}

/// The additive-only contract, checked against the bytes.
///
/// `outl-actions`' `recover/tests.rs` pins the rule at the library
/// level; this pins that the **command** uses it. A restore is a new
/// `Op::Edit` appended to the log — the truncating edit stays exactly
/// where it was, because removing it would break the correctness of
/// every later replay (root `CLAUDE.md` invariant 1).
#[test]
fn the_restore_appends_to_the_op_log_and_never_rewrites_it() {
    let ws = Ws::new();
    let root = ws.root_str();
    let id = ws.seed_block("notes", "keep\ndropped one\ndropped two");
    ws.json(&[
        "block",
        "update",
        &id,
        "--text",
        "keep",
        "--json",
        "--workspace",
        &root,
    ]);

    let before = ws.op_log();
    ws.ok(&["recover", &root, "--apply"]);
    let after = ws.op_log();

    assert_eq!(
        &after[..before.len()],
        &before[..],
        "every op that existed before the restore must survive it verbatim"
    );
    assert_eq!(
        after.len(),
        before.len() + 1,
        "a restore is exactly one new op, not a rewrite"
    );
    let history = ws.ok(&["block", "history", &id, "--workspace", &root]);
    assert!(
        history.contains("keep"),
        "the truncating edit must still be in the history:\n{history}"
    );
}

/// Nothing to recover is a normal outcome, not a failure.
#[test]
fn a_workspace_with_nothing_to_recover_says_so_and_exits_zero() {
    let ws = Ws::new();
    let root = ws.root_str();
    ws.seed_block("notes", "untouched");

    let out = ws.run(&["recover", &root]);
    assert!(out.status.success(), "a clean workspace must exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("no block holds text that an edit truncated"),
        "the clean case needs a sentence a human can act on:\n{stdout}"
    );
}

/// **Deny:** read-only means read-only. Without `--apply` the command
/// prints and writes nothing — not the block, not the `.md`, not one
/// line of the op log.
#[test]
fn a_dry_run_changes_nothing_on_disk() {
    let ws = Ws::new();
    let root = ws.root_str();
    let id = ws.seed_block("notes", "head\ntail one\ntail two");
    ws.json(&[
        "block",
        "update",
        &id,
        "--text",
        "head",
        "--json",
        "--workspace",
        &root,
    ]);

    let ops_before = ws.op_log();
    let md_before = fs::read_to_string(ws.md("pages/notes.md")).expect("read .md");

    let stdout = ws.ok(&["recover", &root]);
    assert!(
        stdout.contains("Nothing was written"),
        "the dry run must say it wrote nothing:\n{stdout}"
    );

    assert_eq!(ws.op_log(), ops_before, "a dry run must not append an op");
    assert_eq!(
        fs::read_to_string(ws.md("pages/notes.md")).expect("read .md"),
        md_before,
        "a dry run must not touch the `.md`"
    );
    let block = ws.json(&["block", "get", &id, "--json", "--workspace", &root]);
    assert_eq!(
        block["text"].as_str().expect("block text"),
        "head",
        "a dry run must leave the block truncated"
    );
}

/// **Deny:** running `--apply` twice must not restore twice.
///
/// After the first restore the block's text *is* the longest revision in
/// its history, so the truncation signature no longer matches. A second
/// run that still found something would mean the signature is matching
/// on shape rather than on loss — and it would append an op per run,
/// forever.
#[test]
fn a_second_apply_finds_nothing_and_appends_nothing() {
    let ws = Ws::new();
    let root = ws.root_str();
    let id = ws.seed_block("notes", "alpha\nbeta\ngamma");
    ws.json(&[
        "block",
        "update",
        &id,
        "--text",
        "alpha",
        "--json",
        "--workspace",
        &root,
    ]);
    ws.ok(&["recover", &root, "--apply"]);

    let ops_after_first = ws.op_log();
    let second = ws.ok(&["recover", &root, "--apply"]);
    assert!(
        second.contains("no block holds text that an edit truncated"),
        "the second run must find nothing:\n{second}"
    );
    assert_eq!(
        ws.op_log(),
        ops_after_first,
        "a second run must not append a redundant restore"
    );
}

/// **Deny:** a workspace that cannot be opened is an error the user
/// sees, with a non-zero exit — not an empty "nothing found" report.
#[test]
fn recover_on_a_directory_that_is_not_a_workspace_fails_loudly() {
    let ws = Ws::new();
    let elsewhere = ws.dir.path().join("not-a-workspace");
    fs::create_dir_all(&elsewhere).expect("mkdir");

    let out = ws.run(&["recover", elsewhere.to_str().expect("utf-8 path")]);
    assert!(
        !out.status.success(),
        "a missing workspace must not exit 0:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("NO_WORKSPACE"),
        "the error must name the problem:\n{stderr}"
    );
}

/// **Deny:** `--min-lines 0` is rejected by the parser.
///
/// Zero would mean "report a block that lost nothing", which is every
/// block in the workspace — a listing with no signal, produced by the
/// command a user runs when they are already looking for a needle.
#[test]
fn min_lines_below_one_is_rejected() {
    let ws = Ws::new();
    let root = ws.root_str();
    let out = ws.run(&["recover", &root, "--min-lines", "0"]);
    assert!(
        !out.status.success(),
        "`--min-lines 0` must not be accepted"
    );
}

/// **Deny:** raising `--min-lines` past the loss drops the finding.
///
/// This is the flag's whole job — a one-line loss is usually ordinary
/// editing, and the user needs a way to say so without the command
/// deciding on their behalf.
#[test]
fn min_lines_above_the_loss_drops_the_finding() {
    let ws = Ws::new();
    let root = ws.root_str();
    let id = ws.seed_block("notes", "kept\nlost");
    ws.json(&[
        "block",
        "update",
        &id,
        "--text",
        "kept",
        "--json",
        "--workspace",
        &root,
    ]);

    let reported = ws.ok(&["recover", &root]);
    assert!(
        reported.contains("1 block(s)"),
        "the default threshold must report a one-line loss:\n{reported}"
    );

    let filtered = ws.ok(&["recover", &root, "--min-lines", "2"]);
    assert!(
        filtered.contains("no block holds text that an edit truncated"),
        "`--min-lines 2` must drop a one-line loss:\n{filtered}"
    );
}

/// **Deny:** a block the user deleted is not offered back.
///
/// Its history is just as recoverable, which is exactly why this needs
/// pinning: a recovery command that resurrects deliberately deleted
/// content is undoing the user's decision, not a bug.
#[test]
fn a_trashed_block_is_never_offered_for_recovery() {
    let ws = Ws::new();
    let root = ws.root_str();
    let id = ws.seed_block("notes", "surviving\ntruncated away");
    ws.json(&[
        "block",
        "update",
        &id,
        "--text",
        "surviving",
        "--json",
        "--workspace",
        &root,
    ]);
    assert!(
        ws.ok(&["recover", &root]).contains("1 block(s)"),
        "precondition: the block is recoverable while it lives"
    );

    ws.json(&[
        "block",
        "delete",
        &id,
        "--confirm",
        "--json",
        "--workspace",
        &root,
    ]);
    let after = ws.ok(&["recover", &root]);
    assert!(
        after.contains("no block holds text that an edit truncated"),
        "a trashed block must not be offered back:\n{after}"
    );
}

/// **Deny:** a restore whose page the re-projection guard refuses is
/// reported, and the `.md` it refused to touch is left intact.
///
/// This is the common, harmless outcome of `--apply`: the log now holds
/// the full text, but the page's `.md` carries a line no op has, so
/// `apply_page_md_with_sidecar_if_stale` will not overwrite it
/// (invariant 8). The content is safe either way — it is in the op log —
/// so the run still exits 0. What must not happen is the refusal being
/// swallowed into a log line the user never opens, because the page then
/// stops syncing in both directions with nothing on screen saying so.
#[test]
fn a_page_the_projection_guard_refuses_is_named_not_swallowed() {
    let ws = Ws::new();
    let root = ws.root_str();
    let id = ws.seed_block("notes", "head\ntail one\ntail two");
    ws.json(&[
        "block",
        "update",
        &id,
        "--text",
        "head",
        "--json",
        "--workspace",
        &root,
    ]);
    let md_path = ws.md("pages/notes.md");
    let on_disk = "- head\n- an extra line the log never saw\n";
    write_md_hash_faithful(&md_path, on_disk);

    let out = ws.run(&["recover", &root, "--apply"]);
    assert!(
        out.status.success(),
        "the text reached the op log, so the run is not a failure"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("restored 1 block(s), 0 failed"),
        "the restore itself must succeed:\n{stdout}"
    );
    assert!(
        stdout.contains("kept their old `.md`"),
        "the refusal has to reach the user, not just a log line:\n{stdout}"
    );
    assert!(
        stdout.contains("outl reconcile --ahead-of-log"),
        "and it must name the command that clears it:\n{stdout}"
    );
    assert_eq!(
        fs::read_to_string(&md_path).expect("read .md"),
        on_disk,
        "the line the log never saw must still be on disk"
    );
}

/// A long loss is elided rather than printed in full.
///
/// Not cosmetic: the listing is what a user reads to decide whether to
/// run `--apply`, and one 40-line block pushing every other finding off
/// the screen is how the finding that mattered gets missed.
#[test]
fn a_long_loss_is_elided_in_the_listing() {
    let ws = Ws::new();
    let root = ws.root_str();
    let id = ws.seed_block("notes", "head\nt1\nt2\nt3\nt4\nt5");
    ws.json(&[
        "block",
        "update",
        &id,
        "--text",
        "head",
        "--json",
        "--workspace",
        &root,
    ]);

    let stdout = ws.ok(&["recover", &root]);
    assert!(
        stdout.contains("t1") && stdout.contains("t3"),
        "the first lines must be shown:\n{stdout}"
    );
    assert!(
        stdout.contains("2 more line(s)"),
        "the rest must be counted, not printed:\n{stdout}"
    );
    assert!(
        !stdout.contains("t5"),
        "the elided lines must not be printed:\n{stdout}"
    );
}
