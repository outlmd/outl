//! `outl_page_merge` and `outl_page_rename` with `merge: true`, over the
//! real `outl mcp serve` stdio loop.
//!
//! The merge itself is owned and tested in `outl-actions`; what this file
//! pins is the MCP surface around it: the `confirm` gate, the reply shape
//! an agent reads, that a re-run is a no-op, the rename `merge` flag, and
//! that the catalog advertises the tool.

mod mcp_support;

use mcp_support::{init_workspace, success_data, McpClient};
use serde_json::{json, Value};
use tempfile::TempDir;

fn started(ws: &TempDir) -> McpClient {
    let mut client = McpClient::spawn(ws.path());
    let _ = client.call(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "capabilities": {} }
    }));
    client
}

/// Call a tool and return the whole `result` object (success or error).
fn tool(client: &mut McpClient, name: &str, arguments: Value) -> Value {
    let reply = client.call(json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": name, "arguments": arguments }
    }));
    reply["result"].clone()
}

/// Call a tool that must succeed and return its data payload.
fn tool_ok(client: &mut McpClient, name: &str, arguments: Value) -> Value {
    success_data(&tool(client, name, arguments))
}

fn error_code(result: &Value) -> &str {
    assert_eq!(result["isError"], true, "expected an error reply: {result}");
    result["structuredContent"]["error"]["code"]
        .as_str()
        .expect("error code is a string")
}

fn slugs(client: &mut McpClient) -> Vec<String> {
    let data = tool_ok(client, "outl_page_list", json!({}));
    data["pages"]
        .as_array()
        .expect("pages is an array")
        .iter()
        .filter_map(|p| p["slug"].as_str().map(str::to_string))
        .collect()
}

fn page_md(ws: &TempDir, slug: &str) -> String {
    std::fs::read_to_string(ws.path().join("pages").join(format!("{slug}.md")))
        .unwrap_or_else(|e| panic!("pages/{slug}.md must exist: {e}"))
}

/// `oldnotes` (two blocks, `status:: open`), `notes` (one block,
/// `status:: done`) and `third`, which mentions `oldnotes` two ways.
fn workspace_with_two_notes_pages(client: &mut McpClient) {
    for slug in ["oldnotes", "notes", "third"] {
        tool_ok(client, "outl_page_create", json!({ "slug": slug }));
    }
    for (page, text) in [
        ("oldnotes", "first old block"),
        ("oldnotes", "second old block"),
        ("notes", "the kept block"),
        ("third", "see [[oldnotes]] and #oldnotes for more"),
    ] {
        tool_ok(client, "outl_block_append", json!({ "page": page, "text": text }));
    }
    for (page, value) in [("oldnotes", "open"), ("notes", "done")] {
        tool_ok(
            client,
            "outl_page_prop_set",
            json!({ "page": page, "key": "status", "value": value }),
        );
    }
}

#[test]
fn merge_without_confirm_is_refused_and_changes_nothing() {
    let ws = init_workspace();
    let mut client = started(&ws);
    workspace_with_two_notes_pages(&mut client);

    let refused = tool(
        &mut client,
        "outl_page_merge",
        json!({ "source": "oldnotes", "target": "notes" }),
    );
    assert_eq!(error_code(&refused), "CONFIRM_REQUIRED");

    let explicit_false = tool(
        &mut client,
        "outl_page_merge",
        json!({ "source": "oldnotes", "target": "notes", "confirm": false }),
    );
    assert_eq!(error_code(&explicit_false), "CONFIRM_REQUIRED");

    let listed = slugs(&mut client);
    assert!(listed.contains(&"oldnotes".to_string()), "{listed:?}");
    assert!(
        !page_md(&ws, "notes").contains("old block"),
        "a refused merge must not move blocks"
    );
    assert!(
        page_md(&ws, "third").contains("[[oldnotes]]"),
        "a refused merge must not rewrite references"
    );
}

#[test]
fn merge_moves_blocks_rewrites_refs_and_reports_what_it_did() {
    let ws = init_workspace();
    let mut client = started(&ws);
    workspace_with_two_notes_pages(&mut client);

    let data = tool_ok(
        &mut client,
        "outl_page_merge",
        json!({ "source": "oldnotes", "target": "notes", "confirm": true }),
    );
    assert_eq!(data["outcome"], "merged");
    assert_eq!(data["source"]["slug"], "oldnotes");
    assert_eq!(data["target"]["slug"], "notes");
    assert_eq!(data["moved_blocks"], 2, "{data}");
    assert_eq!(data["rewritten_refs"].as_array().map(Vec::len), Some(1));
    assert_eq!(data["nothing_changed"], false);

    let aliases = data["aliases_added"].as_array().expect("aliases_added");
    assert!(
        aliases.iter().any(|a| a == "oldnotes"),
        "the source's name must survive as an alias: {data}"
    );

    let conflicts = data["property_conflicts"]
        .as_array()
        .expect("property_conflicts is an array");
    assert_eq!(conflicts.len(), 1, "{data}");
    assert_eq!(conflicts[0]["key"], "status");
    assert_eq!(conflicts[0]["kept"], "done", "the target's value wins");
    assert_eq!(conflicts[0]["dropped"], "open");

    let listed = slugs(&mut client);
    assert!(!listed.contains(&"oldnotes".to_string()), "{listed:?}");
    assert!(listed.contains(&"notes".to_string()), "{listed:?}");

    let notes = page_md(&ws, "notes");
    assert!(notes.contains("first old block"), "{notes}");
    assert!(notes.contains("second old block"), "{notes}");
    assert!(notes.contains("the kept block"), "{notes}");
    let third = page_md(&ws, "third");
    assert!(third.contains("see [[notes]] and #notes for more"), "{third}");
    assert!(
        !ws.path().join("pages/oldnotes.md").exists(),
        "the source's projection must be gone"
    );
}

#[test]
fn running_the_same_merge_again_changes_nothing() {
    let ws = init_workspace();
    let mut client = started(&ws);
    workspace_with_two_notes_pages(&mut client);
    let args = json!({ "source": "oldnotes", "target": "notes", "confirm": true });

    let first = tool_ok(&mut client, "outl_page_merge", args.clone());
    assert_eq!(first["nothing_changed"], false);
    let notes_before = page_md(&ws, "notes");

    let second = tool_ok(&mut client, "outl_page_merge", args);
    assert_eq!(second["nothing_changed"], true, "{second}");
    assert_eq!(second["moved_blocks"], 0);
    assert_eq!(second["rewritten_refs"].as_array().map(Vec::len), Some(0));
    assert_eq!(page_md(&ws, "notes"), notes_before);
}

#[test]
fn rename_onto_an_existing_page_merges_only_when_asked() {
    let ws = init_workspace();
    let mut client = started(&ws);
    workspace_with_two_notes_pages(&mut client);

    let conflict = tool(
        &mut client,
        "outl_page_rename",
        json!({ "old_slug": "oldnotes", "new_slug": "notes" }),
    );
    assert_eq!(error_code(&conflict), "SLUG_CONFLICT");
    assert!(
        slugs(&mut client).contains(&"oldnotes".to_string()),
        "a refused rename must leave the page alone"
    );

    let merged = tool_ok(
        &mut client,
        "outl_page_rename",
        json!({ "old_slug": "oldnotes", "new_slug": "notes", "merge": true }),
    );
    assert_eq!(merged["outcome"], "merged", "{merged}");
    assert_eq!(merged["target"]["slug"], "notes");
    assert_eq!(merged["moved_blocks"], 2);

    let listed = slugs(&mut client);
    assert!(!listed.contains(&"oldnotes".to_string()), "{listed:?}");
    assert!(page_md(&ws, "third").contains("[[notes]]"));
}

#[test]
fn the_catalog_advertises_the_merge_tool_and_its_confirm_gate() {
    let ws = init_workspace();
    let mut client = started(&ws);
    let reply = client.call(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    let tools = reply["result"]["tools"].as_array().expect("tools array");

    let merge = tools
        .iter()
        .find(|t| t["name"] == "outl_page_merge")
        .expect("outl_page_merge must be registered");
    let description = merge["description"].as_str().unwrap_or_default();
    assert!(description.contains("confirm:true"), "{description}");
    assert_eq!(merge["inputSchema"]["required"], json!(["source", "target"]));
    assert_eq!(merge["inputSchema"]["properties"]["confirm"]["type"], "boolean");

    let rename = tools
        .iter()
        .find(|t| t["name"] == "outl_page_rename")
        .expect("outl_page_rename must be registered");
    assert_eq!(rename["inputSchema"]["properties"]["merge"]["type"], "boolean");
}
