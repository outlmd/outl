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

/// Extract a string-array argument, tolerating a bare string.
///
/// A caller excluding one tag should not have to remember to wrap it
/// in an array — the DSL's repeated `not-tag:` lines have no such
/// ceremony either. Absent is an empty list, which is the same as not
/// filtering.
///
/// **Anything else is an error, not a silent drop.** This is an
/// exclusion list: dropping `[null, "someday"]` down to one entry, or
/// `5` down to none, hands back the rows the caller asked to hide and
/// reports success. The JS binding's `string_list` refuses the same
/// shapes for the same reason.
pub(crate) fn str_array(args: &Value, key: &str) -> Result<Vec<String>, ApiError> {
    let bad = |what: &str| {
        ApiError::new(
            crate::output::codes::INVALID_ARG,
            format!("`{key}` must be a string or an array of strings, got {what}"),
        )
    };
    match args.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(s)) => Ok(vec![s.clone()]),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| bad("a non-string array entry"))
            })
            .collect(),
        Some(other) => Err(bad(match other {
            Value::Number(_) => "a number",
            Value::Bool(_) => "a boolean",
            _ => "an object",
        })),
    }
}

/// Extract an optional string argument, erroring on any other type.
///
/// [`opt_str`] answers `None` for a wrong type, which is right for a
/// field whose absence means "no opinion". It is wrong for a **filter**:
/// `{"tag": ["work", "ops"]}` would drop the filter entirely and return
/// the whole workspace as a match. An LLM that has just learned
/// `not_tags` takes an array reaches for that shape first.
pub(crate) fn opt_str_strict<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, ApiError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(ApiError::new(
            crate::output::codes::INVALID_ARG,
            format!("`{key}` must be a string"),
        )),
    }
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
