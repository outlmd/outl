//! Smoke test for the MCP stdio surface.
//!
//! Spawns `outl mcp serve --workspace <tmp>` in a subprocess, sends
//! `initialize`, `tools/list`, and `tools/call outl_workspace_info`
//! through stdin, and asserts the JSON-RPC responses. This is the
//! ground truth — if Claude Desktop / Cursor break, this would too.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use tempfile::TempDir;

fn outl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_outl"))
}

fn init_workspace() -> TempDir {
    let dir = TempDir::new().unwrap();
    let status = outl()
        .arg("init")
        .arg(dir.path())
        .status()
        .expect("init failed");
    assert!(status.success());
    dir
}

struct McpClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl McpClient {
    fn spawn(workspace: &std::path::Path) -> Self {
        let mut child = outl()
            .args(["--workspace"])
            .arg(workspace)
            .args(["mcp", "serve"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn mcp serve");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn call(&mut self, payload: Value) -> Value {
        let line = payload.to_string();
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
        let mut response = String::new();
        self.stdout.read_line(&mut response).expect("read response");
        serde_json::from_str(response.trim()).expect("response was JSON")
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // Closing stdin makes the MCP loop exit.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn initialize_then_call_workspace_info() {
    let ws = init_workspace();
    let mut client = McpClient::spawn(ws.path());

    let init = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {}
        }
    }));
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["serverInfo"]["name"], "outl");

    let tools = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list"
    }));
    let list = tools["result"]["tools"]
        .as_array()
        .expect("tools list is an array");
    assert!(
        list.iter().any(|t| t["name"] == "outl_workspace_info"),
        "outl_workspace_info must be registered"
    );

    let call = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "outl_workspace_info",
            "arguments": {}
        }
    }));
    assert_eq!(call["id"], 3);
    let structured = &call["result"]["structuredContent"];
    assert_eq!(structured["ok"], true);
    assert!(structured["data"]["root"].is_string());
}

#[test]
fn doctor_via_mcp_does_not_lie_about_lock() {
    // Regression: doctor used to call `WorkspaceLock::acquire`, which
    // would always fail inside the MCP session (the server already
    // owns the lock) and report a non-existent contention. The fix
    // skips the lock probe and emits an info-level "probe skipped"
    // finding instead.
    let ws = init_workspace();
    let mut client = McpClient::spawn(ws.path());

    let _ = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "capabilities": {} }
    }));

    let resp = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": { "name": "outl_workspace_doctor", "arguments": {} }
    }));
    let structured = &resp["result"]["structuredContent"];
    assert_eq!(structured["ok"], true, "doctor must succeed inside MCP");
    let findings = structured["data"]["findings"].as_array().unwrap();
    let has_lock_warning = findings.iter().any(|f| {
        f["message"]
            .as_str()
            .unwrap_or("")
            .contains("another outl process is holding the workspace lock")
    });
    assert!(
        !has_lock_warning,
        "doctor must not warn about its own lock when running in MCP session"
    );
}

#[test]
fn resources_read_after_tool_call_does_not_deadlock_on_lock() {
    // Regression: resources/read used to call `ws::open` directly,
    // which re-acquires the workspace lock. After any tool call had
    // already cached the workspace through `ServerCtx`, the lock was
    // held for the session and the resource read would fail with
    // `LockError::AlreadyHeld`. The fix routes everything through
    // `ctx.with_workspace`, reusing the cached `WsCtx`.
    let ws = init_workspace();
    let mut client = McpClient::spawn(ws.path());

    let _ = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "capabilities": {} }
    }));

    // Warm the cached workspace via a tool call.
    let _ = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": { "name": "outl_workspace_info", "arguments": {} }
    }));

    // Now read a resource — used to deadlock against the cached lock.
    let read = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "resources/read",
        "params": { "uri": "outl://workspace/info" }
    }));
    assert!(
        read.get("error").is_none(),
        "resources/read after tool call must not fail: {read}"
    );
    assert_eq!(read["id"], 3);
    let contents = read["result"]["contents"]
        .as_array()
        .expect("resources/read returns a contents array");
    assert!(!contents.is_empty());
    assert_eq!(contents[0]["uri"], "outl://workspace/info");
}

#[test]
fn page_create_then_get_via_mcp() {
    let ws = init_workspace();
    let mut client = McpClient::spawn(ws.path());

    // The handshake is required by some hosts; harmless if skipped, but
    // we go through it so the test mirrors real usage.
    let _ = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "capabilities": {} }
    }));

    let create = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "outl_page_create",
            "arguments": { "slug": "ideas", "title": "Ideas" }
        }
    }));
    let structured = &create["result"]["structuredContent"];
    assert_eq!(structured["ok"], true);
    assert_eq!(structured["data"]["meta"]["slug"], "ideas");

    let get = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "outl_page_get",
            "arguments": { "slug": "ideas" }
        }
    }));
    let s2 = &get["result"]["structuredContent"];
    assert_eq!(s2["ok"], true);
    assert_eq!(s2["data"]["meta"]["title"], "Ideas");
}

/// RFC 0255 Part 1: a page that stopped syncing must come back as a
/// distinct, structured refusal, not a generic `INTERNAL` failure —
/// the caller needs to be able to tell "this page stopped syncing"
/// from "that operation failed" without parsing prose.
#[test]
fn frozen_page_update_returns_structured_refusal_not_a_generic_error() {
    let ws = init_workspace();

    // Create the page through the MCP itself so its `.md` + sidecar
    // start out consistent, then drop the client (closing stdin ends
    // the `mcp serve` process and releases the workspace lock) before
    // hand-corrupting the file on disk.
    {
        let mut client = McpClient::spawn(ws.path());
        let _ = client.call(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2024-11-05", "capabilities": {} }
        }));
        let create = client.call(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "outl_page_create",
                "arguments": {
                    "slug": "frozen",
                    "title": "Frozen",
                    "content": [{ "text": "first" }]
                }
            }
        }));
        assert_eq!(create["result"]["structuredContent"]["ok"], true);
    }

    // Reproduce the exact state invariant 8 guards against: the `.md`
    // holds a line the op log never recorded, and its sidecar has
    // been re-stamped to declare those bytes faithful — the same
    // construction `outl-actions`'s own
    // `if_stale_refuses_when_the_md_carries_content_the_log_lacks`
    // uses. The hash gate alone cannot tell this apart from an
    // ordinary stale projection; only `content_lines_missing_from`
    // can, which is what the guarded write now consults.
    let md_path = ws.path().join("pages").join("frozen.md");
    let mut md = std::fs::read_to_string(&md_path).expect("read frozen.md");
    md.push_str("- only ever on disk\n");
    std::fs::write(&md_path, &md).expect("write unlogged line");

    let sidecar_path = outl_md::sidecar::sidecar_path_for(&md_path);
    let mut sidecar = outl_md::sidecar::read(&sidecar_path).expect("read sidecar");
    sidecar.last_synced_hash = outl_md::sidecar::file_hash(&md);
    outl_md::sidecar::write(&sidecar_path, &sidecar).expect("restamp sidecar");

    let mut client = McpClient::spawn(ws.path());
    let _ = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "capabilities": {} }
    }));

    let update = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "outl_page_update",
            "arguments": { "slug": "frozen", "title": "Frozen (renamed)" }
        }
    }));

    assert_eq!(
        update["result"]["isError"], true,
        "a frozen page's update must fail: {update}"
    );
    let structured = &update["result"]["structuredContent"];
    assert_eq!(structured["ok"], false);
    let error = &structured["error"];
    assert_eq!(
        error["code"], "PAGE_MARKDOWN_AHEAD_OF_LOG",
        "must be the distinct code, not a generic INTERNAL failure: {error}"
    );
    assert_eq!(error["data"]["lines"], 1);
    assert!(
        error["data"]["sample"]
            .as_str()
            .unwrap_or_default()
            .contains("only ever on disk"),
        "the sample must name the content at risk: {error}"
    );
    assert_eq!(
        error["data"]["recovery_command"],
        outl_actions::error::AHEAD_OF_LOG_RECOVERY_COMMAND
    );
    assert!(
        error["data"]["path"]
            .as_str()
            .unwrap_or_default()
            .ends_with("frozen.md"),
        "must name the file at risk: {error}"
    );

    // The whole point of the guard: the bytes on disk must survive
    // untouched, not get silently deleted by the refused write.
    let after = std::fs::read_to_string(&md_path).expect("re-read frozen.md");
    assert!(
        after.contains("only ever on disk"),
        "a refused write must never delete the unlogged content: {after:?}"
    );
}
