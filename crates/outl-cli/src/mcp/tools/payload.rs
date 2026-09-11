//! MCP tool-output wrapping.
//!
//! Turns a handler's `serde_json::Value` (or an [`ApiError`]) into the
//! `tools/call` result shape, projected for an LLM consumer. The shared
//! `cmd/*` handler is untouched; only this wire copy is trimmed.

use serde_json::{json, Value};

use crate::output::{ApiError, Envelope};

/// The payload field whose raw string is the natural text content for a
/// markdown-first tool — leaner than any JSON form. `None` for tools
/// whose result is structured.
fn markdown_field(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "outl_export_md" | "outl_page_render" | "outl_daily_today" | "outl_daily_get" => Some("md"),
        _ => None,
    }
}

/// Wrap an [`ApiError`] as a recoverable tool error (`isError: true`),
/// not a JSON-RPC protocol fault.
///
/// Unlike success, this keeps `structuredContent`: the text content is
/// only a `code: message` summary, so `error.data` (RFC 0255 —
/// `PAGE_MARKDOWN_AHEAD_OF_LOG` carries `path` / `lines` / `sample` /
/// `recovery_command`) would otherwise not reach the caller.
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

/// Wrap a successful tool result, content-only.
///
/// `content[0].text` carries the payload as compact JSON — or, for
/// markdown-first tools, the raw `.md`. No `structuredContent`: the text
/// already holds the full payload, so an envelope around it is a
/// duplicate. (The error path is the exception; see [`tool_error_payload`].)
/// [`prune_gui_fields`] drops fields only a GUI renderer reads before
/// serializing.
pub(crate) fn tool_success_payload(tool_name: &str, mut payload: Value) -> Value {
    let text = markdown_field(tool_name)
        .and_then(|field| {
            payload
                .get(field)
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| {
            prune_gui_fields(&mut payload);
            serde_json::to_string(&payload).unwrap_or_else(|_| payload.to_string())
        });
    json!({
        "content": [ { "type": "text", "text": text } ],
        "isError": false,
    })
}

/// Drop GUI-only fields from a payload, in place.
///
/// `tokens` (an outline node's pre-tokenized inline AST, a verbose
/// restatement of `text`) is dropped wherever it appears — the key is
/// unique to that type. `collapsed: false`, `todo: null` and an empty
/// `properties` are default noise, dropped only on an outline node
/// (identified by `text` + `children`) so a same-named field elsewhere
/// (e.g. `page_prop_list`'s `properties`, `block_get`'s `todo`) is left
/// intact.
fn prune_gui_fields(v: &mut Value) {
    match v {
        Value::Object(map) => {
            map.remove("tokens");
            if map.contains_key("text") && map.contains_key("children") {
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
            map.values_mut().for_each(prune_gui_fields);
        }
        Value::Array(arr) => arr.iter_mut().for_each(prune_gui_fields),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::codes;

    fn success_json(tool: &str, payload: Value) -> Value {
        let out = tool_success_payload(tool, payload);
        assert_eq!(out["isError"], false);
        assert!(
            out.get("structuredContent").is_none(),
            "success replies must not carry a duplicate structuredContent envelope"
        );
        let text = out["content"][0]["text"].as_str().unwrap().to_string();
        assert!(
            !text.contains('\n'),
            "content text must be compact JSON: {text}"
        );
        serde_json::from_str(&text).unwrap()
    }

    /// A structured tool's success payload round-trips through the compact
    /// content text unchanged (minus the GUI-only pruning tested below).
    #[test]
    fn success_payload_is_content_only_and_compact() {
        let payload = json!({ "meta": { "slug": "ideas", "title": "Ideas" } });
        assert_eq!(success_json("outl_page_get", payload.clone()), payload);
    }

    /// A markdown-first tool flattens to its raw `.md` string.
    #[test]
    fn markdown_first_tool_returns_raw_md() {
        let out = tool_success_payload("outl_page_render", json!({ "slug": "x", "md": "- hi" }));
        assert_eq!(out["content"][0]["text"], "- hi");
    }

    /// GUI-only outline fields are stripped; meaningful values and a
    /// same-named field on a non-outline object survive.
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
            ],
            // Not an outline node (no text+children): must be untouched.
            "prop_list": { "properties": [] }
        });
        let out = success_json("outl_page_get", payload);
        let root = &out["outline"][0];

        assert!(root.get("tokens").is_none(), "tokens must be dropped");
        assert!(root.get("collapsed").is_none(), "collapsed:false is noise");
        assert!(root.get("todo").is_none(), "todo:null is noise");
        assert!(
            root.get("properties").is_none(),
            "empty properties is noise"
        );
        assert_eq!(root["text"], "hello");

        let child = &root["children"][0];
        assert!(child.get("tokens").is_none());
        assert_eq!(child["todo"], "TODO");
        assert_eq!(child["collapsed"], true);
        assert_eq!(child["properties"], json!([["k", "v"]]));

        assert_eq!(out["prop_list"], json!({ "properties": [] }));
    }

    /// Errors keep `structuredContent` so `error.data` (RFC 0255) survives.
    #[test]
    fn error_payload_keeps_structured_content() {
        let err = ApiError::new(codes::PAGE_NOT_FOUND, "page `x` not found".to_string());
        let out = tool_error_payload(&err);

        assert_eq!(out["structuredContent"]["ok"], false);
        assert_eq!(
            out["structuredContent"]["error"]["code"],
            codes::PAGE_NOT_FOUND
        );
    }
}
