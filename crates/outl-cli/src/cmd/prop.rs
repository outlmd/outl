//! `outl page prop …` — page-level property reads/writes.
//!
//! Properties live on the page node as `SetProp` ops, which the
//! markdown projection renders as `key:: value` lines at the top of
//! the file. Block-level properties (children with `key:: value` text)
//! still flow through the normal block path.

use std::path::Path;

use clap::Subcommand;
use serde_json::{json, Value};

use outl_actions::{apply_page_md_with_sidecar_guarded, find_by_slug, set_property};
use outl_core::id::NodeId;
use outl_core::property::PropValue;

use crate::output::{codes, emit, ApiError};
use crate::ws::{self, WsCtx};

/// `outl page prop …` subcommands.
#[derive(Subcommand, Debug)]
pub enum PropCommand {
    /// Set a page property: `outl page prop set <page> key=value`.
    Set {
        /// Page slug.
        page: String,
        /// `key=value`. The value is stored as plain text.
        assignment: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Get a page property by key.
    Get {
        /// Page slug.
        page: String,
        /// Property key.
        key: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// List every property on a page.
    List {
        /// Page slug.
        page: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
}

/// Run a `outl page prop …` invocation.
pub fn run(cmd: &PropCommand, path: &Path) -> i32 {
    match cmd {
        PropCommand::Set {
            page,
            assignment,
            json,
        } => {
            let result = ws::open(path).and_then(|mut ctx| set(&mut ctx, page, assignment));
            emit(*json, result, |v| {
                let key = v.get("key").and_then(Value::as_str).unwrap_or("?");
                let val = v.get("value").and_then(Value::as_str).unwrap_or("?");
                println!("set: {key} = {val}");
            })
        }
        PropCommand::Get { page, key, json } => {
            let result = ws::open(path).and_then(|ctx| get(&ctx, page, key));
            emit(*json, result, |v| {
                if let Some(val) = v.get("value") {
                    println!("{val}");
                }
            })
        }
        PropCommand::List { page, json } => {
            let result = ws::open(path).and_then(|ctx| list(&ctx, page));
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
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Set a `key=value` property on a page (CLI shape — parses the
/// `key=value` shorthand). MCP and other typed callers should use
/// [`set_kv`] directly to avoid round-tripping through string
/// concatenation.
pub fn set(ctx: &mut WsCtx, page: &str, assignment: &str) -> Result<Value, ApiError> {
    let (key, value) = assignment.split_once('=').ok_or_else(|| {
        ApiError::new(
            codes::INVALID_ARG,
            format!("expected `key=value`, got `{assignment}`"),
        )
    })?;
    set_kv(ctx, page, key.trim(), value.trim())
}

/// Typed entry point — write `key = value` to `page` and reproject.
/// Used by the MCP shim so we don't have to `format!("{k}={v}")`.
pub fn set_kv(ctx: &mut WsCtx, page: &str, key: &str, value: &str) -> Result<Value, ApiError> {
    let id = resolve_page(ctx, page)?;
    set_property(
        &mut ctx.workspace,
        &ctx.hlc,
        id,
        key,
        Some(PropValue::Text(value.to_string())),
    )
    .map_err(ApiError::internal)?;

    apply_page_md_with_sidecar_guarded(&ctx.workspace, &ctx.root, id)?;
    Ok(json!({ "page": page, "key": key, "value": value }))
}

/// Get a property by key. Returns `null` value when unset.
pub fn get(ctx: &WsCtx, page: &str, key: &str) -> Result<Value, ApiError> {
    let id = resolve_page(ctx, page)?;
    match ctx.workspace.tree().property(id, key) {
        Some(value) => Ok(json!({
            "page": page,
            "key": key,
            "value": stringify(value),
        })),
        None => Err(ApiError::new(
            codes::PROP_NOT_FOUND,
            format!("page `{page}` has no property `{key}`"),
        )),
    }
}

/// List every property on a page.
///
/// The tree map does not expose a per-node property iterator yet, so
/// we probe a list of well-known keys. Extend [`well_known_property_keys`]
/// when a new surface lands.
pub fn list(ctx: &WsCtx, page: &str) -> Result<Value, ApiError> {
    let id = resolve_page(ctx, page)?;
    let mut props: Vec<(String, String)> = Vec::new();
    for key in well_known_property_keys() {
        if let Some(value) = ctx.workspace.tree().property(id, key) {
            props.push((key.to_string(), stringify(value)));
        }
    }
    Ok(json!({
        "page": page,
        "properties": props
            .into_iter()
            .map(|(k, v)| json!({ "key": k, "value": v }))
            .collect::<Vec<_>>(),
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_page(ctx: &WsCtx, slug: &str) -> Result<NodeId, ApiError> {
    find_by_slug(&ctx.workspace, slug)
        .ok_or_else(|| ApiError::new(codes::PAGE_NOT_FOUND, format!("page `{slug}` not found")))
}

fn stringify(value: &PropValue) -> String {
    match value {
        PropValue::Text(s) | PropValue::PageRef(s) | PropValue::Tag(s) => s.clone(),
        PropValue::List(items) => items.iter().map(stringify).collect::<Vec<_>>().join(", "),
    }
}

/// Property keys we know the workspace surface uses. We can't iterate
/// the whole property map directly from the tree today, so the `list`
/// subcommand probes this list — extend it when a new well-known key
/// lands.
fn well_known_property_keys() -> &'static [&'static str] {
    &[
        "title",
        "icon",
        "tags",
        "priority",
        "status",
        "auto-run",
        "pinned",
        "page-slug",
        "page-kind",
    ]
}
