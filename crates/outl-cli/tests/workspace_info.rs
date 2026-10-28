//! `outl workspace info` — the summary a user runs to ask how big their
//! graph is.
//!
//! Its own file rather than a pair of cases in `cli_machine.rs` because
//! one of them needs plumbing nothing else in that suite does: a
//! workspace that **booted from a snapshot**, which the CLI will not
//! produce on its own (its snapshot threshold is 0 — an ephemeral
//! process has no business churning the boot cache), so the fixture
//! writes one through `outl_ws` the way a GUI client does.

use serde_json::Value;
use std::process::Command;
use tempfile::TempDir;

fn outl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_outl"))
}

fn ok(out: std::process::Output) -> Value {
    if !out.status.success() {
        panic!(
            "command failed:\nstatus: {:?}\nstdout: {}\nstderr: {}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }
    serde_json::from_slice(&out.stdout).expect("non-JSON stdout")
}

fn init_workspace() -> TempDir {
    let dir = TempDir::new().unwrap();
    let status = outl()
        .arg("init")
        .arg(dir.path())
        .status()
        .expect("init failed");
    assert!(status.success(), "outl init must succeed");
    dir
}

#[test]
fn workspace_info_returns_summary() {
    let ws = init_workspace();
    let info = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["workspace", "info", "--json"])
        .output()
        .unwrap());
    assert!(info["data"]["root"].is_string());
    assert!(info["data"]["actor"].is_string());
    assert!(info["data"]["ops"].is_number());
    // `ops` counts the resident log, which after a snapshot boot is the
    // delta and not the total. A fresh workspace has no snapshot, so the
    // count is complete — and the flag has to say so, because a reader
    // with no flag cannot tell 332 ops from 332 ops out of 228,943.
    assert_eq!(
        info["data"]["ops_complete"], true,
        "a workspace with no snapshot reports a complete count: {info}"
    );
}

/// The other half of the pair above: the branch the flag exists for.
///
/// A workspace that booted from a snapshot replays only the delta above
/// the cutoff, so `ops` is not the total — and with no op since the
/// snapshot it is **zero**, on a workspace that plainly has ops. Without
/// this, reverting the label (and the `ops_complete` field with it) would
/// not fail a test, because the only covered branch was the one that was
/// already correct.
#[test]
fn workspace_info_says_when_the_op_count_is_only_the_snapshot_delta() {
    let ws = init_workspace();
    let _ = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "create", "ideas", "--json"])
        .output()
        .unwrap());

    // The CLI never writes a snapshot of its own — its threshold is 0,
    // an ephemeral process having no business churning the boot cache —
    // so the fixture writes one the way a GUI client does and lets the
    // next open adopt it.
    {
        let mut ctx = outl_ws::open(ws.path()).expect("the workspace opens for writing");
        assert!(
            !ctx.workspace.log().is_empty(),
            "the fixture has to hold ops for a snapshot to hide any"
        );
        ctx.workspace
            .save_snapshot()
            .expect("the snapshot is written");
    }

    let info = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["workspace", "info", "--json"])
        .output()
        .unwrap());
    assert_eq!(
        info["data"]["ops_complete"], false,
        "a snapshot boot replays a delta, and this flag is what says so: {info}"
    );
    assert_eq!(
        info["data"]["ops"], 0,
        "nothing landed since the snapshot, so the resident log is empty — \
         which is exactly the count that must never read as a total: {info}"
    );

    // The human line is the surface a person reads. A flag only the JSON
    // carries leaves the printed number saying the thing it was labelled
    // to stop saying.
    let human = outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["workspace", "info"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.contains("since the last snapshot"),
        "the human line must qualify a partial count: {text}"
    );
}
