//! MCP tool-output wrapping.
//!
//! Turns a handler's `serde_json::Value` (or an [`ApiError`]) into the
//! `tools/call` result shape. This is the one place the MCP surface
//! diverges from the CLI's JSON envelope, on purpose: the shared
//! `cmd/*` handler is untouched, only the copy that crosses the MCP
//! wire is projected down to what an LLM consumer needs.

use serde_json::{json, Value};

use crate::output::{ApiError, Envelope};

/// Wrap an [`ApiError`] into MCP tool output. MCP tool errors flow
/// through the response shape `{ content: [...], isError: true }`
/// rather than as JSON-RPC errors, so the client gets a recoverable
/// signal instead of a protocol-level fault.
///
/// `structuredContent` carries the full `{ ok, data, error }` envelope
/// — the deliberate exception to the content-only success shape. This
/// is what makes `ApiError::data` reach the wire: a code like
/// `PAGE_MARKDOWN_AHEAD_OF_LOG` alone tells a caller *that* a page
/// stopped syncing, but `error.data.path` / `.lines` / `.sample` /
/// `.recovery_command` is what lets it act instead of just reporting
/// the failure onward (RFC 0255).
pub(crate) fn tool_error_payload(err: &ApiError) -> Value {
    let envelope = Envelope::<Value>::failure(err.clone());
    json!({
        "content": [
            { "type": "text", "text": format!("{}: {}", err.code, err.message) }
        ],
        "structuredContent": serde_json::to_value(&envelope).unwrap_or(Value::Null),
        "isError": true,
    })
}

/// Wrap a successful tool result for the MCP wire.
///
/// The result is **content-only**. A client speaking this server's
/// protocol version (`2024-11-05`) reads `content[0].text`; that
/// revision has no `structuredContent`, so a success envelope carried
/// there was a second, discarded copy of the same payload. Errors are
/// the exception — [`tool_error_payload`] keeps `structuredContent`
/// because a caller needs `error.data` (RFC 0255) and the failure
/// signal is otherwise just prose.
///
/// Two token-saving projections happen here, both scoped to this
/// consumer so the CLI's own `--json` output is untouched:
///
/// - **GUI-only fields are pruned** ([`lean_for_mcp`]) — an
///   `OutlineNode`'s `tokens` (a pre-tokenized inline AST the mobile /
///   desktop renderers consume) restates `text` at 2-3x its size, and
///   an LLM already has `text`.
/// - **JSON is compact, not pretty** — `content[0].text` is what the
///   model reads, and the indentation was pure overhead.
///
/// `tool_name` still lets markdown-first tools (`export_md`,
/// `page_render`, `daily_*`) flatten to their raw `.md` string, which
/// is leaner still than any JSON form.
pub(crate) fn tool_success_payload(tool_name: &str, payload: &Value) -> Value {
    let lean = lean_for_mcp(payload);
    let text = preferred_text_for(tool_name, &lean);
    json!({
        "content": [ { "type": "text", "text": text } ],
        "isError": false,
    })
}

/// Project a tool payload down to what an LLM consumer needs, dropping
/// fields only a GUI renderer reads. Per-consumer: the shared handler
/// still returns the full shape to the CLI, this only trims the copy
/// that crosses the MCP wire.
fn lean_for_mcp(payload: &Value) -> Value {
    let mut v = payload.clone();
    prune_gui_fields(&mut v);
    v
}

/// Recursively strip GUI-only noise from a payload.
///
/// - `tokens` is only ever [`outl_actions::OutlineNode`]'s inline AST,
///   so it is safe to drop wherever it appears.
/// - `collapsed: false`, `todo: null`, and an empty `properties` are
///   default-valued noise emitted on every outline node. They are
///   pruned **only** on objects carrying the outline-node signature
///   (`text` + `children`), so a same-named field on another tool's
///   payload (e.g. `page_prop_list`'s `properties`) is left intact.
fn prune_gui_fields(v: &mut Value) {
    match v {
        Value::Object(map) => {
            map.remove("tokens");
            let is_outline_node = map.contains_key("text") && map.contains_key("children");
            if is_outline_node {
                if map.get("collapsed") == Some(&Value::Bool(false)) {
                    map.remove("collapsed");
                }
                if map.get("todo") == Some(&Value::Null) {
                    map.remove("todo");
                }
                if matches!(map.get("properties"), Some(Value::Array(a)) if a.is_empty()) {
                    map.remove("properties");
                }
            }
            for child in map.values_mut() {
                prune_gui_fields(child);
            }
        }
        Value::Array(arr) => arr.iter_mut().for_each(prune_gui_fields),
        _ => {}
    }
}

/// Pick a text content best suited for `tool_name`.
///
/// Tools that produce a single big string (rendered markdown, summary
/// text) flatten the payload by reading its natural field. Everything
/// else is emitted as compact JSON — `content[0].text` is what the
/// model reads, so no whitespace is spent on indentation.
fn preferred_text_for(tool_name: &str, payload: &Value) -> String {
    let take_field = |field: &str| -> Option<String> {
        payload
            .get(field)
            .and_then(Value::as_str)
            .map(str::to_string)
    };

    match tool_name {
        // Pure-markdown surfaces: prefer the raw `md` field.
        "outl_export_md" | "outl_page_render" => take_field("md"),
        // Daily / page surfaces ship both `md` and a structured outline;
        // the host shows the markdown as the "natural" text content.
        "outl_daily_today" | "outl_daily_get" => take_field("md"),
        _ => None,
    }
    .unwrap_or_else(|| serde_json::to_string(payload).unwrap_or_else(|_| payload.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::codes;

    /// A success reply is content-only and compact — no `structuredContent`
    /// second copy, no pretty-print whitespace. The model reads
    /// `content[0].text`, so that is the only thing worth paying for.
    #[test]
    fn success_payload_is_content_only_and_compact() {
        let payload = json!({ "meta": { "slug": "ideas", "title": "Ideas" } });
        let out = tool_success_payload("outl_page_get", &payload);

        assert_eq!(out["isError"], false);
        assert!(
            out.get("structuredContent").is_none(),
            "success replies must not carry a duplicate structuredContent envelope"
        );
        let text = out["content"][0]["text"].as_str().unwrap();
        assert!(
            !text.contains('\n'),
            "content text must be compact JSON: {text}"
        );
        // Round-trips back to the same payload.
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed, payload);
    }

    /// GUI-only outline fields (`tokens`, and default-valued `collapsed` /
    /// `todo` / empty `properties`) are stripped from the MCP copy, while a
    /// same-named field on a non-outline payload is left intact.
    #[test]
    fn success_payload_strips_gui_only_outline_fields() {
        let payload = json!({
            "outline": [
                {
                    "id": "01ABC",
                    "text": "hello",
                    "todo": null,
                    "collapsed": false,
                    "properties": [],
                    "tokens": [ { "type": "plain", "text": "hello" } ],
                    "children": [
                        {
                            "id": "01DEF",
                            "text": "child",
                            "todo": "TODO",
                            "collapsed": true,
                            "properties": [ ["k", "v"] ],
                            "tokens": [ { "type": "plain", "text": "child" } ],
                            "children": []
                        }
                    ]
                }
            ]
        });
        let lean = lean_for_mcp(&payload);
        let root = &lean["outline"][0];

        assert!(root.get("tokens").is_none(), "tokens must be dropped");
        assert!(root.get("collapsed").is_none(), "collapsed:false is noise");
        assert!(root.get("todo").is_none(), "todo:null is noise");
        assert!(
            root.get("properties").is_none(),
            "empty properties is noise"
        );
        assert_eq!(root["text"], "hello");

        // Meaningful (non-default) values survive.
        let child = &root["children"][0];
        assert!(child.get("tokens").is_none());
        assert_eq!(child["todo"], "TODO");
        assert_eq!(child["collapsed"], true);
        assert_eq!(child["properties"], json!([["k", "v"]]));

        // A `properties` key outside an outline node (no text+children
        // signature) is not an outline node and must be untouched.
        let other = lean_for_mcp(&json!({ "properties": [] }));
        assert_eq!(other, json!({ "properties": [] }));
    }

    /// Errors are the deliberate exception: they keep `structuredContent`
    /// so a caller can read `error.data` (RFC 0255) rather than parse prose.
    #[test]
    fn error_payload_keeps_structured_content() {
        let err = ApiError::new(codes::PAGE_NOT_FOUND, "page `x` not found".to_string());
        let out = tool_error_payload(&err);

        assert_eq!(out["isError"], true);
        assert_eq!(out["structuredContent"]["ok"], false);
        assert_eq!(
            out["structuredContent"]["error"]["code"],
            codes::PAGE_NOT_FOUND
        );
    }
}
