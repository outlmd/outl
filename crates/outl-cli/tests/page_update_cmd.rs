//! `outl page update --title` changes the title a reader sees.
//!
//! `page_meta` resolves the title from the `title::` property first, so a
//! page that already carries one (every page created with a title that
//! differs from its slug) ignored an update that only touched the page
//! root's text. These tests drive the real binary, the batch op and the
//! MCP tool, and check the meta, the projected `.md` and a fresh process.

mod mcp_support;

use mcp_support::{init_workspace, success_data, McpClient};
use serde_json::{json, Value};
use std::process::Command;
use tempfile::TempDir;

fn outl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_outl"))
}

fn run(ws: &TempDir, args: &[&str]) -> Value {
    let out = outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(args)
        .output()
        .expect("spawn outl");
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "non-JSON stdout ({e}):\nstatus: {:?}\nstdout: {}\nstderr: {}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        )
    })
}

fn page_md(ws: &TempDir, slug: &str) -> String {
    std::fs::read_to_string(ws.path().join("pages").join(format!("{slug}.md")))
        .unwrap_or_else(|e| panic!("pages/{slug}.md must exist: {e}"))
}

/// A page whose title differs from its slug, so it carries `title::`.
fn workspace_with_titled_page() -> TempDir {
    let ws = init_workspace();
    let created = run(
        &ws,
        &["page", "create", "alpha", "--title", "Alpha Page", "--json"],
    );
    assert_eq!(created["data"]["meta"]["title"], "Alpha Page", "{created}");
    assert!(page_md(&ws, "alpha").contains("title:: Alpha Page"));
    ws
}

fn assert_retitled(ws: &TempDir, meta: &Value) {
    assert_eq!(meta["title"], "Beta Title", "returned meta: {meta}");

    let md = page_md(ws, "alpha");
    assert!(md.contains("title:: Beta Title"), "projected .md: {md}");
    assert!(!md.contains("Alpha Page"), "old title left behind: {md}");

    // A fresh process reads the title back from the op log.
    let got = run(ws, &["page", "get", "alpha", "--json"]);
    assert_eq!(got["data"]["meta"]["title"], "Beta Title", "{got}");
}

#[test]
fn update_title_changes_a_page_that_already_has_a_title_property() {
    let ws = workspace_with_titled_page();
    let updated = run(
        &ws,
        &["page", "update", "alpha", "--title", "Beta Title", "--json"],
    );
    assert_eq!(updated["ok"], true, "{updated}");
    assert_retitled(&ws, &updated["data"]["meta"]);
}

#[test]
fn update_title_trims_the_value_like_page_meta_reads_it() {
    let ws = workspace_with_titled_page();
    let updated = run(
        &ws,
        &[
            "page",
            "update",
            "alpha",
            "--title",
            "  Beta Title  ",
            "--json",
        ],
    );
    assert_eq!(updated["ok"], true, "{updated}");
    assert_retitled(&ws, &updated["data"]["meta"]);
}

#[test]
fn update_with_a_blank_title_is_rejected_and_changes_nothing() {
    let ws = workspace_with_titled_page();
    for blank in ["", "   "] {
        let env = run(
            &ws,
            &["page", "update", "alpha", "--title", blank, "--json"],
        );
        assert_eq!(env["ok"], false, "{env}");
        assert_eq!(env["error"]["code"], "INVALID_ARG", "{env}");
    }
    assert!(page_md(&ws, "alpha").contains("title:: Alpha Page"));
}

#[test]
fn a_blank_title_rejects_the_whole_call_icon_included() {
    // The rejection is for the whole call: the icon is not applied either.
    let ws = workspace_with_titled_page();
    let env = run(
        &ws,
        &[
            "page", "update", "alpha", "--title", " ", "--icon", "star", "--json",
        ],
    );
    assert_eq!(env["error"]["code"], "INVALID_ARG", "{env}");
    assert!(!page_md(&ws, "alpha").contains("icon::"));
}

#[test]
fn batch_page_update_changes_the_title() {
    let ws = workspace_with_titled_page();
    let payload = json!({"ops": [
        {"op": "page_update", "args": {"slug": "alpha", "title": "Beta Title"}}
    ]})
    .to_string();
    let env = run(&ws, &["batch", "--ops", &payload, "--json"]);
    assert_eq!(env["ok"], true, "{env}");
    assert_retitled(&ws, &env["data"]["results"][0]["data"]["meta"]);
}

#[test]
fn update_title_over_mcp_changes_the_title_and_rejects_a_blank_one() {
    let ws = workspace_with_titled_page();
    let mut client = McpClient::spawn(ws.path());
    let _ = client.call(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "capabilities": {} }
    }));

    let blank = client.call(json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {
            "name": "outl_page_update",
            "arguments": { "slug": "alpha", "title": "  " }
        }
    }));
    assert_eq!(blank["result"]["isError"], true, "{blank}");
    assert_eq!(
        blank["result"]["structuredContent"]["error"]["code"],
        "INVALID_ARG"
    );

    let updated = client.call(json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {
            "name": "outl_page_update",
            "arguments": { "slug": "alpha", "title": "Beta Title" }
        }
    }));
    let meta = &success_data(&updated["result"])["meta"];
    assert_eq!(meta["title"], "Beta Title", "{meta}");
    let md = page_md(&ws, "alpha");
    assert!(md.contains("title:: Beta Title"), "projected .md: {md}");
}
