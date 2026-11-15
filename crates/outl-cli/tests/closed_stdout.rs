//! `outl <cmd> | head -1` must not crash.
//!
//! The reader closing stdout early used to surface as
//! `panic: failed printing to stdout: Broken pipe` (Unix `EPIPE`, Windows
//! `ERROR_NO_DATA` 232 / `ERROR_BROKEN_PIPE` 109), a fatal crash report
//! (OUTL-4) for nothing the user did wrong.
//!
//! The documented behaviour: output the reader no longer wants is dropped, the
//! command runs to completion, and the exit status is the command's own, so a
//! closed pipe on a successful command exits 0 with nothing on stderr.

use std::path::Path;
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;

fn outl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_outl"))
}

/// A workspace whose `big` page renders to far more than a pipe buffer
/// (64 KiB on Linux), so the final print cannot fit and must hit the reader.
/// Under `MAX_ARG_STRLEN` (128 KiB) so it still fits on the command line.
fn workspace_with_a_big_page() -> TempDir {
    let ws = TempDir::new().unwrap();
    let init = outl().arg("init").arg(ws.path()).output().expect("init");
    assert!(init.status.success(), "outl init must succeed");
    let text = "x".repeat(120_000);
    for args in [
        vec!["page", "create", "big", "--json"],
        vec![
            "block", "append", "--page", "big", "--text", &text, "--json",
        ],
    ] {
        let out = outl()
            .arg("--workspace")
            .arg(ws.path())
            .args(&args)
            .output()
            .expect("setup command");
        assert!(
            out.status.success(),
            "setup failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    ws
}

fn render(ws: &Path) -> Command {
    let mut cmd = outl();
    cmd.arg("--workspace")
        .arg(ws)
        .args(["page", "render", "big"]);
    cmd
}

/// Runs `cmd` with the read end of its stdout already closed.
fn run_with_closed_stdout(mut cmd: Command) -> Output {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawning outl");
    drop(child.stdout.take());
    child.wait_with_output().expect("waiting for outl")
}

#[test]
fn a_closed_stdout_does_not_panic_and_exits_cleanly() {
    let ws = workspace_with_a_big_page();

    let out = run_with_closed_stdout(render(ws.path()));

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("failed printing to stdout") && !stderr.contains("panicked"),
        "a closed stdout panicked:\n{stderr}"
    );
    assert_eq!(out.status.code(), Some(0), "stderr:\n{stderr}");
}

#[test]
fn an_open_stdout_still_gets_every_byte() {
    let ws = workspace_with_a_big_page();

    let out = render(ws.path()).output().expect("running outl");

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&"x".repeat(120_000)),
        "the swallow must not eat output nobody closed ({} bytes)",
        stdout.len()
    );
}
