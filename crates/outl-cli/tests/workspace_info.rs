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
    for key in [
        "ops",
        "ops_since_snapshot",
        "pages",
        "journals",
        "blocks",
        "tree_nodes",
        "properties",
        "tags",
        "backlinks",
        "assets",
        "trashed",
        "snapshots",
        "snapshot_bytes",
    ] {
        assert!(
            info["data"][key].is_number(),
            "workspace info should report `{key}`: {info}"
        );
    }
    // `ops` is the persisted op-log total; for a fresh workspace with no
    // snapshot it equals the in-memory log.
    assert_eq!(
        info["data"]["ops_since_snapshot"], info["data"]["ops"],
        "a fresh workspace has no folded ops, so total and delta match: {info}"
    );
    // A fresh workspace has one journal (today) and one root page.
    assert_eq!(
        info["data"]["pages"], 1,
        "fresh workspace has a root page: {info}"
    );
    assert_eq!(
        info["data"]["journals"], 1,
        "fresh workspace has today's journal: {info}"
    );
}

/// A workspace that booted from a snapshot still reports the persisted
/// op-log total, even though the in-memory log only holds the delta.
#[test]
fn workspace_info_reports_total_ops_after_snapshot_boot() {
    let ws = init_workspace();
    let _ = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "create", "ideas", "--json"])
        .output()
        .unwrap());

    // The CLI never writes a snapshot of its own — snapshot writes are
    // opt-in and a short-lived opener leaves them off, which binds a
    // direct `save_snapshot` too and not just the in-band threshold
    // writer. So the fixture asks on the terms a long-lived client
    // opens under, and lets the next open adopt what it wrote.
    {
        let mut ctx = outl_ws::open_with(
            ws.path(),
            outl_ws::OpenOptions {
                write_snapshots: true,
                writes: true,
                device_actor_only: false,
            },
        )
        .expect("the workspace opens for writing");
        assert!(
            !ctx.workspace.log().is_empty(),
            "the fixture has to hold ops for a snapshot to hide any"
        );
        ctx.workspace
            .save_snapshot()
            .expect("the snapshot is written");
    }

    // Apply an op after the snapshot so the next boot replays a non-zero
    // delta above the cutoff.
    let _ = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "create", "later", "--json"])
        .output()
        .unwrap());

    let info = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["workspace", "info", "--json"])
        .output()
        .unwrap());
    let ops = info["data"]["ops"].as_u64().expect("ops is a number");
    let delta = info["data"]["ops_since_snapshot"]
        .as_u64()
        .expect("ops_since_snapshot is a number");
    assert!(
        delta > 0,
        "the resident log must hold the post-snapshot ops: {info}"
    );
    assert!(
        ops > delta,
        "ops must be the persisted total, larger than the delta: {info}"
    );

    // The human line is the surface a person reads; it now shows both
    // numbers, so a snapshot boot cannot be mistaken for an empty log.
    let human = outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["workspace", "info"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.contains("total") && text.contains("since the last snapshot"),
        "the human line must show total and delta: {text}"
    );
}
