//! MCP tool surface — schema registry + dispatcher.
//!
//! Split into two siblings so the file-size guard stays quiet and so
//! each concern has one place to land:
//!
//! - [`registry`] — pure schema list returned by `tools/list`.
//! - [`dispatch`] — `tools/call` router. Every handler delegates to
//!   the same code path the CLI subcommands use.
//!
//! Shared helpers (`tool_def`, `require_str`, `opt_str`) live here so
//! both siblings can reuse them without crossing each other.

use serde_json::{json, Value};

use crate::output::ApiError;

mod dispatch;
mod payload;
mod registry;

pub use dispatch::call;
pub use registry::list;

/// Build one entry in the `tools/list` shape.
pub(crate) fn tool_def(name: &str, description: &str, schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": schema,
    })
}

/// Extract a required string argument or return `INVALID_ARG`.
pub(crate) fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ApiError> {
    args.get(key).and_then(Value::as_str).ok_or_else(|| {
        ApiError::new(
            crate::output::codes::INVALID_ARG,
            format!("missing required string argument `{key}`"),
        )
    })
}

/// Extract an optional string argument.
pub(crate) fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

/// Flatten a `params` JSON object (`{ "k": "v" }`) into the `["k=v", …]`
/// shape the CLI `template run` handler consumes, so the MCP tool and
/// the CLI subcommand share one param parser. Missing / non-object
/// `params` yields an empty list.
pub(crate) fn opt_params(args: &Value) -> Vec<String> {
    args.get("params")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| format!("{k}={s}")))
                .collect()
        })
        .unwrap_or_default()
}
