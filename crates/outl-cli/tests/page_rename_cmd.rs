//! `outl page rename` rewrites what pointed at the old name.
//!
//! Before the rename moved into `outl-actions` the command only changed the
//! slug and listed the blocks still naming the old page, which left every
//! `[[old]]` and `#old` dangling. These tests drive the real binary (and the
//! MCP tool over stdio) so the projection, the old file's removal and the
//! stable conflict code are checked on the surfaces a user and an agent
//! actually call.

mod mcp_support;

use mcp_support::{init_workspace, success_data, McpClient};
use serde_json::{json, Value};
use std::process::Command;
use tempfile::TempDir;

fn outl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_outl"))
}

fn run(ws: &TempDir, args: &[&str]) -> std::process::Output {
    outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(args)
        .output()
        .expect("spawn outl")
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

fn page_md(ws: &TempDir, slug: &str) -> String {
    std::fs::read_to_string(ws.path().join("pages").join(format!("{slug}.md")))
        .unwrap_or_else(|e| panic!("pages/{slug}.md must exist: {e}"))
}

/// Two pages, `alpha` mentioning `beta` three ways, and `beta` itself.
fn workspace_with_mentions() -> TempDir {
    let ws = init_workspace();
    for slug in ["alpha", "beta"] {
        ok(run(&ws, &["page", "create", slug, "--json"]));
    }
    ok(run(
        &ws,
        &[
            "block",
            "append",
            "--page",
            "alpha",
            "--text",
            "see [[beta]] and #beta for more",
            "--json",
        ],
    ));
    ok(run(
        &ws,
        &[
            "block",
            "append",
            "--page",
            "alpha",
            "--text",
            "a code span `[[beta]]` stays as written",
            "--json",
        ],
    ));
    ws
}

#[test]
fn rename_rewrites_refs_and_tags_on_other_pages_and_moves_the_file() {
    let ws = workspace_with_mentions();

    let env = ok(run(&ws, &["page", "rename", "beta", "gamma", "--json"]));
    assert_eq!(env["data"]["meta"]["slug"], "gamma");
    assert_eq!(env["data"]["old_slug"], "beta");
    assert_eq!(
        env["data"]["rewritten_refs"].as_array().map(Vec::len),
        Some(1),
        "only the block with a real mention is rewritten: {env}"
    );
    assert!(env["data"]["affected_refs"].is_array());

    let alpha = page_md(&ws, "alpha");
    assert!(
        alpha.contains("see [[gamma]] and #gamma for more"),
        "{alpha}"
    );
    assert!(
        alpha.contains("`[[beta]]`"),
        "a code span is not a mention and stays as written: {alpha}"
    );

    assert!(ws.path().join("pages/gamma.md").exists());
    assert!(
        !ws.path().join("pages/beta.md").exists(),
        "the old projection must be gone"
    );
    assert!(
        !ws.path().join("pages/beta.outl").exists(),
        "the old sidecar must be gone"
    );
}

#[test]
fn rename_onto_an_existing_page_is_slug_conflict_and_says_merge_is_possible() {
    let ws = workspace_with_mentions();

    let out = run(&ws, &["page", "rename", "beta", "alpha", "--json"]);
    assert!(!out.status.success(), "renaming onto a page must fail");
    let env: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(env["error"]["code"], "SLUG_CONFLICT");
    let message = env["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("merge"),
        "message must offer a merge: {message}"
    );

    // Nothing moved.
    assert!(ws.path().join("pages/beta.md").exists());
    assert!(page_md(&ws, "alpha").contains("[[beta]]"));
}

#[test]
fn rename_of_an_unknown_page_is_page_not_found() {
    let ws = init_workspace();
    let out = run(&ws, &["page", "rename", "nowhere", "somewhere", "--json"]);
    assert!(!out.status.success());
    let env: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(env["error"]["code"], "PAGE_NOT_FOUND");
}

#[test]
fn rename_over_mcp_reports_the_rewritten_blocks() {
    let ws = workspace_with_mentions();
    let mut client = McpClient::spawn(ws.path());
    let _ = client.call(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "capabilities": {} }
    }));

    let renamed = client.call(json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {
            "name": "outl_page_rename",
            "arguments": { "old_slug": "beta", "new_slug": "gamma" }
        }
    }));
    let data = success_data(&renamed["result"]);
    assert_eq!(data["meta"]["slug"], "gamma");
    assert_eq!(data["rewritten_refs"].as_array().map(Vec::len), Some(1));

    let conflict = client.call(json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {
            "name": "outl_page_rename",
            "arguments": { "old_slug": "gamma", "new_slug": "alpha" }
        }
    }));
    assert_eq!(conflict["result"]["isError"], true);
    assert_eq!(
        conflict["result"]["structuredContent"]["error"]["code"],
        "SLUG_CONFLICT"
    );
}

#[test]
fn rename_with_merge_folds_into_the_existing_page() {
    let ws = workspace_with_mentions();
    ok(run(&ws, &["page", "create", "delta", "--json"]));
    ok(run(
        &ws,
        &[
            "block",
            "append",
            "--page",
            "delta",
            "--text",
            "ref [[beta]] here",
            "--json",
        ],
    ));
    ok(run(
        &ws,
        &[
            "block",
            "append",
            "--page",
            "beta",
            "--text",
            "from beta",
            "--json",
        ],
    ));

    let args = ["page", "rename", "beta", "alpha", "--merge", "--json"];
    let env = ok(run(&ws, &args));
    let data = &env["data"];
    assert_eq!(data["outcome"], "merged", "{env}");
    assert_eq!(data["source"]["slug"], "beta");
    assert_eq!(
        data["target"]["slug"],
        "alpha",
        "the existing page keeps its name"
    );
    assert_eq!(data["moved_blocks"], 1);
    assert!(
        data["rewritten_refs"].as_array().is_some_and(|r| !r.is_empty()),
        "mentions of the old name are rewritten: {env}"
    );

    let alpha = page_md(&ws, "alpha");
    assert!(alpha.contains("from beta"), "blocks moved: {alpha}");
    assert!(
        alpha.contains("`[[beta]]`"),
        "a code span is not a mention and stays as written: {alpha}"
    );
    assert!(
        page_md(&ws, "delta").contains("ref [[alpha]] here"),
        "refs now name the target"
    );

    assert!(
        !ws.path().join("pages/beta.md").exists(),
        "the old projection must be gone"
    );
    assert!(
        !ws.path().join("pages/beta.outl").exists(),
        "the old sidecar must be gone"
    );
}

#[test]
fn rename_reports_how_many_references_it_rewrote() {
    let ws = workspace_with_mentions();

    let out = run(&ws, &["page", "rename", "beta", "gamma"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("renamed: gamma"), "{stdout}");
    assert!(
        stdout.contains("rewrote 1 reference(s) to the old name"),
        "{stdout}"
    );
}
