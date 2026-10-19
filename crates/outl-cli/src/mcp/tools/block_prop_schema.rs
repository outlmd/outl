//! Schemas for the block-property tools.
//!
//! They live beside [`super::registry`] rather than inside it: that
//! file is under the file-size ratchet's ceiling, and four more
//! `tool_def`s would put it over.

use serde_json::{json, Value};

use super::tool_def;

pub(crate) fn defs() -> Vec<Value> {
    vec![
        tool_def(
            "outl_block_prop_set",
            "Set a block-level `key:: value` property. An empty value removes it. A newline, a carriage return, or a null byte in the value is rejected.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "key": { "type": "string" },
                    "value": { "type": "string" }
                },
                "required": ["id", "key", "value"]
            }),
        ),
        tool_def(
            "outl_block_prop_get",
            "Read a block property by key.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "key": { "type": "string" }
                },
                "required": ["id", "key"]
            }),
        ),
        tool_def(
            "outl_block_prop_list",
            "List every property on a block.",
            json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }),
        ),
        tool_def(
            "outl_block_prop_delete",
            "Remove a block property. Drops the `key::` line from the page.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "key": { "type": "string" }
                },
                "required": ["id", "key"]
            }),
        ),
    ]
}
