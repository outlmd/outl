//! Integration tests for `outl template run` — the CLI/MCP execution
//! path for callable templates (issue: callable templates could only be
//! *resolved*, never *run*, outside the TUI).
//!
//! Each test drives the real `outl` binary in a tempdir so it exercises
//! the same code path a user (or the MCP shim) would. The template uses
//! a `lisp` code block — the Steel runtime is always in the default
//! `outl-exec` feature set, so the assertion is deterministic regardless
//! of which optional language runtimes the build unifies in.

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

fn create_page(ws: &TempDir, slug: &str) {
    let _ = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "create", slug, "--json"])
        .output()
        .unwrap());
}

fn append_block(ws: &TempDir, page: &str, text: &str) -> String {
    let v = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["block", "append", "--page", page, "--text", text, "--json"])
        .output()
        .unwrap());
    v["data"]["id"].as_str().unwrap().to_string()
}

fn set_prop(ws: &TempDir, page: &str, assignment: &str) {
    let _ = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "prop", "set", page, assignment, "--json"])
        .output()
        .unwrap());
}

/// Define a callable `echo` template (a page with `template:: echo` and a
/// `lisp` code block) and run it against an anchor block on another page.
#[test]
fn template_run_writes_result_subtree() {
    let ws = init_workspace();

    // Callable template page: `template:: echo` + a lisp code block.
    create_page(&ws, "tpl-echo");
    set_prop(&ws, "tpl-echo", "template=echo");
    append_block(
        &ws,
        "tpl-echo",
        "```lisp\n(displayln \"hello from template\")\n```",
    );

    // Target page with an anchor block the result lands under.
    create_page(&ws, "notes");
    let anchor = append_block(&ws, "notes", "run it here");

    let run = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["template", "run", "echo", "--page", "notes", "--block"])
        .arg(&anchor)
        .args(["--json"])
        .output()
        .unwrap());
    assert_eq!(run["ok"], true, "template run must succeed: {run}");
    assert_eq!(run["data"]["template"], "echo");
    assert_eq!(run["data"]["page"], "notes");
    let stdout = run["data"]["result"]["stdout"].as_str().unwrap_or("");
    assert!(
        stdout.contains("hello from template"),
        "runtime stdout should carry the template output, got: {stdout:?}"
    );

    // The `> **result:**` subtree must be projected to disk under the
    // anchor block on the `notes` page.
    let md = std::fs::read_to_string(ws.path().join("pages").join("notes.md")).unwrap();
    assert!(
        md.contains("**result:**"),
        "result header must appear in notes.md, got:\n{md}"
    );
    assert!(
        md.contains("hello from template"),
        "result content must appear in notes.md, got:\n{md}"
    );
}

/// Audit fix #4: `--block` on a page different from `--page` must be
/// rejected with `INVALID_ARG` (instantiating there then reprojecting
/// only `--page` would silently drop the new blocks from disk).
#[test]
fn template_run_rejects_block_on_other_page() {
    let ws = init_workspace();

    create_page(&ws, "tpl-echo");
    set_prop(&ws, "tpl-echo", "template=echo");
    append_block(&ws, "tpl-echo", "```lisp\n(displayln \"hi\")\n```");

    create_page(&ws, "notes");
    create_page(&ws, "other");
    // Anchor lives on `other`, but we pass `--page notes`.
    let foreign = append_block(&ws, "other", "not on notes");

    let out = outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["template", "run", "echo", "--page", "notes", "--block"])
        .arg(&foreign)
        .args(["--json"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "cross-page --block must error, got success"
    );
    let env: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        env["error"]["code"], "INVALID_ARG",
        "cross-page --block must map to INVALID_ARG, got {env}"
    );
}

/// The same cross-page guard applies to `template apply` (the original
/// audit finding: it accepted any `--block` ULID and instantiated under
/// it even on a foreign page).
#[test]
fn template_apply_rejects_block_on_other_page() {
    let ws = init_workspace();

    // Structural template page.
    create_page(&ws, "tpl-struct");
    set_prop(&ws, "tpl-struct", "template=struct");
    append_block(&ws, "tpl-struct", "seed block");

    create_page(&ws, "notes");
    create_page(&ws, "other");
    let foreign = append_block(&ws, "other", "not on notes");

    let out = outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["template", "apply", "struct", "--page", "notes", "--block"])
        .arg(&foreign)
        .args(["--json"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "cross-page --block must error on apply, got success"
    );
    let env: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(env["error"]["code"], "INVALID_ARG");
}

/// RFC 0255 follow-up: `apply()` / `run_template()` re-render an
/// *existing* page's `.md` after instantiating a template under it.
/// Before this fix both routed through the unconditional
/// `apply_page_md_with_sidecar` and discarded the `Result` (`let _ =
/// ...`), so a frozen page — one whose `.md` holds content the op log
/// never recorded — had that content silently deleted, with nobody told.
/// This is the same class of bug the CLI/MCP write paths had (page
/// update, block append, ...); `template apply` was simply the last
/// place it survived.
///
/// This test reproduces the frozen-page state by hand (same
/// construction `outl-actions`'s own
/// `if_stale_refuses_when_the_md_carries_content_the_log_lacks` and the
/// MCP's `frozen_page_update_returns_structured_refusal_not_a_generic_error`
/// use): write a line to `notes.md` that no op ever produced, then
/// re-stamp the sidecar's hash so the hash gate alone would call the
/// file "faithful". `template apply` must refuse to reproject rather
/// than delete that line, and it must say so instead of swallowing the
/// refusal into a discarded `Result`.
#[test]
fn template_apply_refuses_to_reproject_a_frozen_page() {
    let ws = init_workspace();

    // Structural template page.
    create_page(&ws, "tpl-struct");
    set_prop(&ws, "tpl-struct", "template=struct");
    append_block(&ws, "tpl-struct", "seed block");

    // Target page with real, already-projected content.
    create_page(&ws, "notes");
    append_block(&ws, "notes", "existing content");

    let md_path = ws.path().join("pages").join("notes.md");
    let mut md = std::fs::read_to_string(&md_path).expect("read notes.md");
    md.push_str(
        "- only ever on disk
",
    );
    std::fs::write(&md_path, &md).expect("write unlogged line");

    let sidecar_path = outl_md::sidecar::sidecar_path_for(&md_path);
    let mut sidecar = outl_md::sidecar::read(&sidecar_path).expect("read sidecar");
    sidecar.last_synced_hash = outl_md::sidecar::file_hash(&md);
    outl_md::sidecar::write(&sidecar_path, &sidecar).expect("restamp sidecar");

    let out = outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["template", "apply", "struct", "--page", "notes", "--json"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "applying a template onto a frozen page must fail, got success: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let env: Value = serde_json::from_slice(&out.stdout).expect("non-JSON stdout");
    assert_eq!(
        env["error"]["code"], "PAGE_MARKDOWN_AHEAD_OF_LOG",
        "must be the distinct refusal, not INTERNAL or silent success: {env}"
    );
    assert_eq!(env["error"]["data"]["lines"], 1);
    assert!(
        env["error"]["data"]["sample"]
            .as_str()
            .unwrap_or_default()
            .contains("only ever on disk"),
        "the sample must name the content at risk: {env}"
    );
    assert_eq!(
        env["error"]["data"]["recovery_command"],
        outl_actions::error::AHEAD_OF_LOG_RECOVERY_COMMAND
    );

    // The whole point: the unlogged line must survive on disk, not get
    // silently deleted by the refused reprojection.
    let after = std::fs::read_to_string(&md_path).expect("re-read notes.md");
    assert!(
        after.contains("only ever on disk"),
        "a refused reprojection must never delete the unlogged content: {after:?}"
    );
}
