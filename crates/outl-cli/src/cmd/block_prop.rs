//! `outl block prop …` — block-level property reads and writes.
//!
//! A block property is an `Op::SetProp` on the block node. The write is
//! [`outl_actions::set_property`], the same call `set_block_property`
//! makes, and the key is cleaned by [`outl_actions::property::normalize_key`]
//! and [`outl_actions::property::key_rejection`] before it gets there.
//!
//! An empty value removes the key (`SetProp` with `None`). Storing
//! `PropValue::Text("")` is what leaves a bare `key::` line in the
//! rendered `.md` — the page-property ghost in issue #300. `delete`
//! and `set key=` both take the removing path.
//!
//! A newline, a carriage return, or a null byte in the value is
//! rejected. `write_property` emits the value on the same line as
//! `key::`, so one of those bytes would split the line and the next
//! parse would read a different tree.

use std::path::Path;

use clap::Subcommand;
use serde_json::{json, Value};

use outl_actions::{enclosing_page_id, set_property};
use outl_core::id::NodeId;
use outl_core::property::PropValue;

use super::block::parse_id;
use super::prop::stringify;
use crate::output::{codes, emit, ApiError};
use crate::ws::{self, WsCtx};

/// `outl block prop …` subcommands.
#[derive(Subcommand, Debug)]
pub enum BlockPropCommand {
    /// Set a block property: `outl block prop set <id> key=value`.
    ///
    /// An empty value (`key=`) removes the property.
    Set {
        /// Block id.
        id: String,
        /// `key=value`. The value is stored as plain text.
        assignment: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Get a block property by key.
    Get {
        /// Block id.
        id: String,
        /// Property key.
        key: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// List every property on a block.
    List {
        /// Block id.
        id: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Remove a block property. The `key::` line is dropped from the `.md`.
    Delete {
        /// Block id.
        id: String,
        /// Property key.
        key: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
}

/// Run a `outl block prop …` invocation.
pub fn run(cmd: &BlockPropCommand, path: &Path) -> i32 {
    match cmd {
        BlockPropCommand::Set {
            id,
            assignment,
            json,
        } => {
            let result = ws::open(path).and_then(|mut ctx| set(&mut ctx, id, assignment));
            emit(*json, result, |v| {
                let key = v.get("key").and_then(Value::as_str).unwrap_or("?");
                match v.get("value").and_then(Value::as_str) {
                    Some(val) => println!("set: {key} = {val}"),
                    None => println!("deleted: {key}"),
                }
            })
        }
        BlockPropCommand::Get { id, key, json } => {
            let result = ws::open(path).and_then(|ctx| get(&ctx, id, key));
            emit(*json, result, |v| {
                if let Some(val) = v.get("value") {
                    println!("{val}");
                }
            })
        }
        BlockPropCommand::List { id, json } => {
            let result = ws::open(path).and_then(|ctx| list(&ctx, id));
            emit(*json, result, |v| {
                if let Some(props) = v.get("properties").and_then(Value::as_array) {
                    for p in props {
                        let key = p.get("key").and_then(Value::as_str).unwrap_or("?");
                        let val = p.get("value").and_then(Value::as_str).unwrap_or("?");
                        println!("{key:20}  {val}");
                    }
                }
            })
        }
        BlockPropCommand::Delete { id, key, json } => {
            let result = ws::open(path).and_then(|mut ctx| delete(&mut ctx, id, key));
            emit(*json, result, |v| {
                let key = v.get("key").and_then(Value::as_str).unwrap_or("?");
                println!("deleted: {key}");
            })
        }
    }
}

/// Set a `key=value` property on a block (CLI shape).
pub fn set(ctx: &mut WsCtx, id: &str, assignment: &str) -> Result<Value, ApiError> {
    let (key, value) = assignment.split_once('=').ok_or_else(|| {
        ApiError::new(
            codes::INVALID_ARG,
            format!("expected `key=value`, got `{assignment}`"),
        )
    })?;
    set_kv(ctx, id, key.trim(), value)
}

/// Typed entry point — write `key = value` on `id` and reproject.
///
/// An empty `value` removes the property. MCP calls this directly.
pub fn set_kv(ctx: &mut WsCtx, id: &str, key: &str, value: &str) -> Result<Value, ApiError> {
    assign(ctx, id, key, Some(value))
}

/// Get a property by key. `PROP_NOT_FOUND` when unset.
pub fn get(ctx: &WsCtx, id: &str, key: &str) -> Result<Value, ApiError> {
    let node = require_block(ctx, id)?;
    let key = prepare_key(key)?;
    match ctx.workspace.tree().property(node, &key) {
        Some(value) => Ok(json!({
            "id": id,
            "key": key,
            "value": stringify(value),
        })),
        None => Err(ApiError::new(
            codes::PROP_NOT_FOUND,
            format!("block `{id}` has no property `{key}`"),
        )),
    }
}

/// List every property on a block, in the order every outl surface
/// shows them (`outl_actions::tree::sort_in_presentation_order`).
///
/// The tree exposes [`outl_core::tree::Tree::properties_of`], so this
/// is the block's real set — not a probe of well-known keys.
pub fn list(ctx: &WsCtx, id: &str) -> Result<Value, ApiError> {
    let node = require_block(ctx, id)?;
    let mut props: Vec<(String, String)> = ctx
        .workspace
        .tree()
        .properties_of(node)
        .map(|(key, value)| (key.to_string(), stringify(value)))
        .collect();
    outl_actions::tree::sort_in_presentation_order(&mut props);
    Ok(json!({
        "id": id,
        "properties": props
            .into_iter()
            .map(|(key, value)| json!({ "key": key, "value": value }))
            .collect::<Vec<_>>(),
    }))
}

/// Remove a property. The op stores `None`, so the next projection
/// does not emit a `key::` line.
pub fn delete(ctx: &mut WsCtx, id: &str, key: &str) -> Result<Value, ApiError> {
    let node = require_block(ctx, id)?;
    let key = prepare_key(key)?;
    if ctx.workspace.tree().property(node, &key).is_none() {
        return Err(ApiError::new(
            codes::PROP_NOT_FOUND,
            format!("block `{id}` has no property `{key}`"),
        ));
    }
    assign(ctx, id, &key, None)?;
    Ok(json!({ "id": id, "key": key }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Write `value` (or remove the key, when `value` is empty / `None`)
/// and reproject the page that holds the block.
fn assign(
    ctx: &mut WsCtx,
    id: &str,
    raw_key: &str,
    value: Option<&str>,
) -> Result<Value, ApiError> {
    let node = require_block(ctx, id)?;
    let key = prepare_key(raw_key)?;
    // Reject before trim. A value that is only a newline would otherwise
    // trim to empty and take the clear path, and a value with a break
    // in the middle would render as two lines.
    if let Some(raw) = value {
        if let Some(why) = value_rejection(raw) {
            return Err(ApiError::new(codes::INVALID_ARG, why));
        }
    }
    // Same rule as `set_block_property`: trim, and an empty value is a
    // clear. `Text("")` would render as a bare `key::` line.
    let stored = value
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    let page = enclosing_page_id(&ctx.workspace, node).ok_or_else(|| {
        ApiError::new(
            codes::BLOCK_NOT_FOUND,
            format!("block `{id}` is not on a page"),
        )
    })?;
    let hlc = ctx.hlc.clone();
    let prop = stored.clone().map(PropValue::Text);
    ctx.commit_with(page, |ws| set_property(ws, &hlc, node, &key, prop))?;
    Ok(json!({
        "id": id,
        "key": key,
        "value": stored,
    }))
}

fn require_block(ctx: &WsCtx, id: &str) -> Result<NodeId, ApiError> {
    let node = parse_id(id)?;
    if !ctx.workspace.tree().contains(node) {
        return Err(ApiError::new(
            codes::BLOCK_NOT_FOUND,
            format!("block `{id}` not found"),
        ));
    }
    Ok(node)
}

/// A property is one `key:: value` line. A line break or a null byte
/// cannot live in that line.
fn value_rejection(value: &str) -> Option<&'static str> {
    if value.contains(['\n', '\r', '\0']) {
        Some(
            "property value must stay on one line and cannot contain a newline, a carriage return, or a null byte",
        )
    } else {
        None
    }
}

fn prepare_key(raw: &str) -> Result<String, ApiError> {
    let key = outl_actions::property::normalize_key(raw);
    if let Some(why) = outl_actions::property::key_rejection(&key) {
        return Err(ApiError::new(codes::INVALID_ARG, why));
    }
    Ok(key)
}
