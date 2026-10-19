//! Block properties over MCP.
//!
//! Same handlers as `outl block prop`. This file exists so a tool that
//! is registered in `tools/list` but not dispatched (or the reverse)
//! fails here, and so delete is checked on the wire an agent actually
//! calls.

mod mcp_support;

use mcp_support::{init_workspace, success_data, McpClient};
use serde_json::json;

fn call(
    client: &mut McpClient,
    id: i64,
    name: &str,
    arguments: serde_json::Value,
) -> serde_json::Value {
    client.call(json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments }
    }))
}

#[test]
fn block_prop_tools_round_trip_and_delete_drops_the_line() {
    let ws = init_workspace();
    let mut client = McpClient::spawn(ws.path());
    let _ = client.call(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "capabilities": {} }
    }));

    let listed = client.call(json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/list"
    }));
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    for tool in [
        "outl_block_prop_set",
        "outl_block_prop_get",
        "outl_block_prop_list",
        "outl_block_prop_delete",
    ] {
        assert!(names.contains(&tool), "{tool} must be registered");
    }

    let _ = call(
        &mut client,
        3,
        "outl_page_create",
        json!({ "slug": "ideas", "title": "Ideas" }),
    );
    let append = call(
        &mut client,
        4,
        "outl_block_append",
        json!({ "page": "ideas", "text": "ship it" }),
    );
    let id = success_data(&append["result"])["id"]
        .as_str()
        .unwrap()
        .to_string();

    let set = call(
        &mut client,
        5,
        "outl_block_prop_set",
        json!({ "id": id, "key": "priority", "value": "high" }),
    );
    assert_eq!(success_data(&set["result"])["value"], "high");

    let get = call(
        &mut client,
        6,
        "outl_block_prop_get",
        json!({ "id": id, "key": "priority" }),
    );
    assert_eq!(success_data(&get["result"])["value"], "high");

    let _ = call(
        &mut client,
        7,
        "outl_block_prop_set",
        json!({ "id": id, "key": "owner", "value": "ada" }),
    );
    let list = call(&mut client, 8, "outl_block_prop_list", json!({ "id": id }));
    let listed = success_data(&list["result"]);
    let keys: Vec<&str> = listed["properties"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, vec!["owner", "priority"]);

    let deleted = call(
        &mut client,
        9,
        "outl_block_prop_delete",
        json!({ "id": id, "key": "priority" }),
    );
    assert_eq!(success_data(&deleted["result"])["key"], "priority");

    let md = std::fs::read_to_string(ws.path().join("pages/ideas.md")).expect("pages/ideas.md");
    assert!(
        !md.lines()
            .any(|line| line.trim_start().starts_with("priority::")),
        "MCP delete must not leave a ghost `priority::` line:\n{md}"
    );
    assert!(md.contains("owner:: ada"), "owner stays:\n{md}");
}

#[test]
fn a_line_break_or_null_in_the_value_is_rejected_and_the_markdown_stays() {
    let ws = init_workspace();
    let mut client = McpClient::spawn(ws.path());
    let _ = client.call(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "capabilities": {} }
    }));
    let _ = call(
        &mut client,
        2,
        "outl_page_create",
        json!({ "slug": "ideas", "title": "Ideas" }),
    );
    let append = call(
        &mut client,
        3,
        "outl_block_append",
        json!({ "page": "ideas", "text": "ship it" }),
    );
    let id = success_data(&append["result"])["id"]
        .as_str()
        .unwrap()
        .to_string();
    let path = ws.path().join("pages/ideas.md");
    let before = std::fs::read_to_string(&path).expect("pages/ideas.md");

    for (n, value) in [(4_i64, "high\nlow"), (5, "high\0low")] {
        let set = call(
            &mut client,
            n,
            "outl_block_prop_set",
            json!({ "id": id, "key": "priority", "value": value }),
        );
        assert_eq!(set["result"]["isError"], true, "{set}");
        assert_eq!(
            set["result"]["structuredContent"]["error"]["code"], "INVALID_ARG",
            "{set}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("pages/ideas.md"),
            before,
            "a rejected value must not touch the page"
        );
    }
}
