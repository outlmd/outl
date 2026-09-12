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
///
/// **A tool only belongs here when the rest of its payload is
/// recoverable by the caller.** Flattening to one field discards every
/// sibling, so the test is not "is markdown the useful part" but "does
/// the caller already have what I am about to drop".
///
/// `outl_page_render` and `outl_export_md` pass: their payload is
/// `{slug, md}` and the caller sent that slug in.
///
/// `outl_daily_today` / `outl_daily_get` do **not**, which is why they
/// are absent despite being journal reads. Their payload is
/// `{date, meta, outline, md}` (`cmd/daily.rs`), and `outline` is the
/// only place a block's id appears — a rendered `.md` carries no ids,
/// they live in the sidecar. Every write tool that targets a block
/// (`outl_block_update`, `_move`, `_delete`, `_toggle_todo`) requires
/// that id, so flattening these two breaks "read today's journal, tick
/// a task" in the one place it is most used. `date` matters too:
/// `outl_daily_today` takes no argument, so its reply is the only thing
/// telling the caller which journal was opened.
fn markdown_field(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "outl_export_md" | "outl_page_render" => Some("md"),
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
/// Pruning is keyed on the signature of [`outl_actions::outline::OutlineNode`]
/// — `id` **and** `text` **and** `children` — and applies to nothing else.
/// On a node that matches: `tokens` (a pre-tokenized inline AST that
/// restates `text` for the Tauri renderers) always goes, and
/// `collapsed: false` / `todo: null` / empty `properties` go as default
/// noise.
///
/// **`id` is in the guard to tell the two `OutlineNode` types apart, and
/// it is load-bearing.** `outl_md::ast::OutlineNode` is
/// `{text, properties, children}` — the same `text` + `children` shape,
/// with **no** `id`. `outl_export_json` serializes those nodes as its
/// `blocks` (`cmd/export_v2.rs`), and the field has no
/// `#[serde(default)]`, so dropping an empty `properties` there makes
/// the payload fail to deserialize back into the type it came from
/// (`missing field 'properties'`) on every block that has no property.
/// A `text` + `children` guard silently broke the one tool whose whole
/// job is being an interchange format; pinned by
/// `pruning_leaves_the_parser_ast_alone`.
fn prune_gui_fields(v: &mut Value) {
    match v {
        Value::Object(map) => {
            if map.contains_key("id") && map.contains_key("text") && map.contains_key("children") {
                map.remove("tokens");
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
        serde_json::from_str(&text).unwrap_or_else(|e| {
            panic!("{tool} did not return JSON ({e}) — is it wrongly markdown-first? text: {text}")
        })
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

    /// The journal reads are **not** markdown-first: flattening them to
    /// `md` would drop `outline`, the only carrier of the block ids that
    /// every block-targeting write tool requires, plus the `date` that
    /// argument-less `outl_daily_today` has no other way to report.
    ///
    /// Do not add them back to [`markdown_field`] — a rendered `.md` has
    /// no ids in it, so "read the journal, tick a task" stops closing in
    /// one round-trip the moment this test is deleted.
    #[test]
    fn journal_reads_keep_their_ids_and_date() {
        for tool in ["outl_daily_today", "outl_daily_get"] {
            let payload = json!({
                "date": "2026-09-12",
                "meta": { "slug": "2026-09-12", "kind": "journal" },
                "md": "- ship it",
                "outline": [ {
                    "id": "01ABC",
                    "text": "ship it",
                    "todo": null,
                    "collapsed": false,
                    "properties": [],
                    "tokens": [ { "type": "plain", "text": "ship it" } ],
                    "children": []
                } ],
            });
            let out = success_json(tool, payload);

            assert_eq!(
                out["date"], "2026-09-12",
                "{tool} must report the journal it opened"
            );
            assert_eq!(
                out["outline"][0]["id"], "01ABC",
                "{tool} must keep the block id a write tool needs"
            );
            assert_eq!(
                out["md"], "- ship it",
                "{tool} must still carry the markdown"
            );
            assert!(
                out["outline"][0].get("tokens").is_none(),
                "{tool} still prunes the GUI-only AST"
            );
        }
    }

    /// `outl_export_json` returns `outl_md::ast::OutlineNode`s, which are
    /// `{text, properties, children}` with no `id` and no
    /// `#[serde(default)]` on `properties`. Pruning an empty
    /// `properties` there would make the export fail to deserialize back
    /// into the type it came from — on every block without a property,
    /// which is most of them.
    ///
    /// This is why [`prune_gui_fields`] keys on `id` too. Widening that
    /// guard back to `text` + `children` breaks the one tool whose
    /// purpose is being an interchange format.
    #[test]
    fn pruning_leaves_the_parser_ast_alone() {
        let blocks = json!([
            { "text": "no props", "properties": [], "children": [
                { "text": "nested", "properties": [], "children": [] }
            ] }
        ]);
        let out = success_json(
            "outl_export_json",
            json!({ "meta": { "slug": "x" }, "properties": [], "blocks": blocks.clone() }),
        );
        assert_eq!(
            out["blocks"], blocks,
            "the parser AST must round-trip byte for byte"
        );
        assert_eq!(
            out["properties"],
            json!([]),
            "page properties are not outline noise"
        );
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
