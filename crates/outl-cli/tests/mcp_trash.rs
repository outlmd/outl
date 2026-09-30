//! Restoring a deleted block over MCP.
//!
//! Its own file rather than another case in `mcp_smoke.rs`: that one is
//! at the file-size ratchet, and this exercises a different promise —
//! not "the protocol works" but "an agent that deleted something can
//! undo it" (issue #287).

mod mcp_support;

use mcp_support::{init_workspace, success_data, McpClient};

/// An agent that can delete a block has to be able to undo it. Before
/// `outl_trash_*`, the MCP surface could trash 683 blocks on a real
/// workspace and had no tool that could name one of them (issue #287).
#[test]
fn trash_list_and_restore_over_mcp() {
    let ws = init_workspace();
    let mut client = McpClient::spawn(ws.path());
    let _ = client.call(serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "capabilities": {} }
    }));
    let _ = client.call(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "outl_page_create", "arguments": { "slug": "ideas" } }
    }));
    let append = client.call(serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {
            "name": "outl_block_append",
            "arguments": { "page": "ideas", "text": "undo me" }
        }
    }));
    let id = success_data(&append["result"])["id"]
        .as_str()
        .unwrap()
        .to_string();

    let _ = client.call(serde_json::json!({
        "jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": {
            "name": "outl_block_delete",
            "arguments": { "id": id, "confirm": true }
        }
    }));

    let listed = client.call(serde_json::json!({
        "jsonrpc": "2.0", "id": 5, "method": "tools/call",
        "params": { "name": "outl_trash_list", "arguments": {} }
    }));
    let data = success_data(&listed["result"]);
    let entries = data["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], id);
    assert_eq!(entries[0]["restorable"], true);

    let restored = client.call(serde_json::json!({
        "jsonrpc": "2.0", "id": 6, "method": "tools/call",
        "params": { "name": "outl_trash_restore", "arguments": { "id": id } }
    }));
    assert_eq!(success_data(&restored["result"])["id"], id);

    // And the index the read tools share has to see the restore, or the
    // next `outl_page_get` reports a page the workspace no longer has.
    let page = client.call(serde_json::json!({
        "jsonrpc": "2.0", "id": 7, "method": "tools/call",
        "params": { "name": "outl_page_get", "arguments": { "slug": "ideas" } }
    }));
    let outline = serde_json::to_string(&success_data(&page["result"])["outline"]).unwrap();
    assert!(
        outline.contains("undo me"),
        "restored block missing: {outline}"
    );
}
