//! Who may write the boot cache, and who may not.
//!
//! `OpenOptions::write_snapshots` is off by default and the CLI's
//! **writing** open turns it on. That asymmetry is the whole point, and
//! both halves are easy to lose:
//!
//! - flip it on for reads and `outl page get` starts paying a multi-MB
//!   encode plus an fsync for the benefit of whatever runs next;
//! - leave it off for writes and a terminal-only machine never gets a
//!   snapshot at all, so every invocation full-replays the log forever
//!   (330-357ms per command against 132-166ms, measured on 10k pages).
//!
//! These tests assert on the snapshots **directory**, not on a timing,
//! so they fail on the policy rather than on how fast the runner is.

use std::path::Path;

use outl_core::id::NodeId;
use outl_core::op::Op;
use outl_ws::layout::{init, Paths};
use outl_ws::{open_read, open_with, OpenOptions};
use tempfile::TempDir;

fn workspace() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    init(&Paths::at(dir.path().to_path_buf())).expect("init");
    dir
}

/// One `Create`, so the snapshot cutoff is non-empty —
/// `build_snapshot_body` returns `None` on an empty log, by design.
fn seed_one_op(ctx: &mut outl_ws::WsCtx) {
    let ts = ctx.hlc.next();
    let actor = ctx.actor;
    let parent = NodeId::root();
    ctx.workspace
        .apply(outl_core::op::LogOp {
            ts,
            actor,
            op: Op::Create {
                node: NodeId::new(),
                parent,
                position: outl_core::fractional::Fractional::first(),
            },
        })
        .expect("apply");
}

fn snapshot_count(root: &Path) -> usize {
    let dir = root.join(".outl").join("snapshots");
    let Ok(read) = std::fs::read_dir(&dir) else {
        return 0;
    };
    read.flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".bin"))
        .count()
}

/// A write open with `write_snapshots` asked for persists one when the
/// workspace asks for a refresh, which after a full replay it always
/// does — the "there was no usable snapshot" case.
#[test]
fn a_writing_open_can_persist_the_boot_cache() {
    let dir = workspace();
    let mut ctx = open_with(
        dir.path(),
        OpenOptions {
            write_snapshots: true,
            ..Default::default()
        },
    )
    .expect("open");

    seed_one_op(&mut ctx);

    assert!(
        ctx.workspace.refresh_snapshot_in_background(),
        "a full-replay boot is always owed a snapshot"
    );
    // `Drop` waits the worker out; do it explicitly so the assertion
    // below does not race it.
    ctx.workspace.wait_for_snapshots();

    assert_eq!(
        snapshot_count(dir.path()),
        1,
        "a writing open asked to write snapshots wrote none"
    );
}

/// The half that protects the hot path: a read must not pay for the
/// next process. `open_read` leaves the policy disabled, so even an
/// explicit request is a no-op through it.
#[test]
fn a_read_open_never_writes_one() {
    let dir = workspace();
    {
        let mut ctx = open_with(
            dir.path(),
            OpenOptions {
                write_snapshots: true,
                ..Default::default()
            },
        )
        .expect("open");
        seed_one_op(&mut ctx);
        ctx.workspace.wait_for_snapshots();
    }
    let before = snapshot_count(dir.path());
    std::fs::remove_dir_all(dir.path().join(".outl").join("snapshots")).ok();
    assert_eq!(snapshot_count(dir.path()), 0, "fixture not reset");

    let mut ctx = open_read(dir.path()).expect("read open");
    // Asking directly is the sharpest form of the question: the policy
    // is what refuses, not the absence of a caller.
    assert!(
        !ctx.workspace.refresh_snapshot_in_background(),
        "a read open agreed to write a snapshot"
    );
    ctx.workspace.wait_for_snapshots();
    assert_eq!(
        snapshot_count(dir.path()),
        0,
        "a read open wrote into the snapshots dir"
    );
    let _ = before;
}
