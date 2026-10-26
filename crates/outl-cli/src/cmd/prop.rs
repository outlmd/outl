//! `outl page prop …` — page-level property reads/writes.
//!
//! Properties live on the page node as `SetProp` ops, which the
//! markdown projection renders as `key:: value` lines at the top of
//! the file. Block-level properties (children with `key:: value` text)
//! still flow through the normal block path.

use std::path::Path;

use clap::Subcommand;
use serde_json::{json, Value};

use outl_actions::{find_by_slug, set_property};
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
    /// Remove a page property: `outl page prop delete <page> <key>`.
    Delete {
        /// Page slug.
        page: String,
        /// Property key.
        key: String,
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
                let val = v.get("value").and_then(Value::as_str).unwrap_or("");
                if val.is_empty() {
                    println!("deleted: {key}");
                } else {
                    println!("set: {key} = {val}");
                }
            })
        }
        PropCommand::Get { page, key, json } => {
            let result = ws::open_read(path).and_then(|ctx| get(&ctx, page, key));
            emit(*json, result, |v| {
                if let Some(val) = v.get("value") {
                    println!("{val}");
                }
            })
        }
        PropCommand::List { page, json } => {
            let result = ws::open_read(path).and_then(|ctx| list(&ctx, page));
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
        PropCommand::Delete { page, key, json } => {
            let result = ws::open(path).and_then(|mut ctx| delete(&mut ctx, page, key));
            emit(*json, result, |v| {
                let key = v.get("key").and_then(Value::as_str).unwrap_or("?");
                println!("deleted: {key}");
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
/// Used by the MCP shim and by `page_prop_set` in a batch, so we don't
/// have to `format!("{k}={v}")`.
///
/// An empty or whitespace-only value removes the key
/// (`set_property(..., None)`), the same rule as `set_page_property`.
/// Storing `Text("")` would leave the log and the `.md` disagreeing:
/// `prop get` would return `""` while the projection omits the line,
/// and a peer that rebuilds from the file would drop the key.
pub fn set_kv(ctx: &mut WsCtx, page: &str, key: &str, value: &str) -> Result<Value, ApiError> {
    let value = value.trim();
    if value.is_empty() {
        let mut cleared = delete(ctx, page, key)?;
        cleared["value"] = json!("");
        return Ok(cleared);
    }
    let id = resolve_page(ctx, page)?;
    let hlc = ctx.hlc.clone();
    let owned = value.to_string();
    ctx.commit_with(id, |ws| {
        set_property(ws, &hlc, id, key, Some(PropValue::Text(owned)))
    })?;
    Ok(json!({ "page": page, "key": key, "value": value }))
}

/// Remove `key` from `page`.
///
/// This is `Op::SetProp { value: None }`, which `do_op` already treats
/// as removal. The op is appended even when this device does not
/// currently hold the key, so a peer that set it later still converges.
/// A blank `set_kv` calls this too.
pub fn delete(ctx: &mut WsCtx, page: &str, key: &str) -> Result<Value, ApiError> {
    let key = outl_actions::property::normalize_key(key);
    if let Some(why) = outl_actions::property::key_rejection(&key) {
        return Err(ApiError::new(codes::INVALID_ARG, why));
    }
    let id = resolve_page(ctx, page)?;
    let hlc = ctx.hlc.clone();
    let key_owned = key.clone();
    ctx.commit_with(id, |ws| set_property(ws, &hlc, id, &key_owned, None))?;
    Ok(json!({ "page": page, "key": key }))
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
/// `outl_actions::property::page_properties` is the owner: the page's
/// real property set, minus the page model's book-keeping
/// (`outl_actions::tree::is_page_model_key`), in
/// `sort_in_presentation_order`.
///
/// It used to probe a hardcoded list of nine "well-known" keys, which
/// got both halves wrong at once: a page's actual properties were all
/// absent, and `page-slug` / `page-kind` were listed — the two keys
/// `is_page_model_key` exists to hide, because `outl page prop set`
/// refuses them and showing them invites an edit that rewrites the
/// page's identity on the next projection. Both directions are pinned
/// by `page_prop_list_shows_the_users_properties_and_hides_the_page_model`.
pub fn list(ctx: &WsCtx, page: &str) -> Result<Value, ApiError> {
    let id = resolve_page(ctx, page)?;
    let props = outl_actions::property::page_properties(&ctx.workspace, id);
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

/// A property value as this binary prints it, shared with
/// [`super::block_prop`] so `outl page prop get` and `outl block prop
/// get` cannot read the same stored value two ways.
///
/// **Not** `outl_actions::outline::prop_value_to_string`, which joins a
/// `List` with a space instead of `", "`. Both are reachable from *this*
/// command — `get` renders through here, `list` through
/// [`outl_actions::property::page_properties`], which uses that one — so
/// the two are not merely neighbours, they answer for the same stored
/// value on two subcommands. Nothing in the binary constructs a `List`
/// today, so they agree on every reachable value; the separator is pinned
/// by `crates/outl-actions/tests/render_properties_characterization.rs`,
/// and collapsing them would change that rendering rather than dedupe it.
/// Which separator is right is the query DSL's call, and merging these two
/// is that decision, not a cleanup.
pub(super) fn stringify(value: &PropValue) -> String {
    match value {
        PropValue::Text(s) | PropValue::PageRef(s) | PropValue::Tag(s) => s.clone(),
        PropValue::List(items) => items.iter().map(stringify).collect::<Vec<_>>().join(", "),
    }
}
