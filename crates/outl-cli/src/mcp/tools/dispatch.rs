//! `tools/call` dispatcher.
//!
//! Every arm is a thin shim: extract args, ask `ServerCtx` for the
//! cached workspace (and the cached `WorkspaceIndex` when needed), call
//! the same handler the CLI subcommand uses, return the JSON
//! envelope's `data`. Workspace open + op-log replay happen once per
//! MCP session, not per tool call.
//!
//! Mutating / destructive ops include an explicit `confirm` gate to
//! avoid accidental damage from a chatty LLM. The cached
//! `WorkspaceIndex` is invalidated after any mutation so the next
//! read-only call sees fresh blocks.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::cmd::{
    asset as asset_cmd, backlinks as bl_cmd, batch as batch_cmd, block as block_cmd,
    daily as daily_cmd, doctor as doctor_cmd, export_v2 as exp_cmd, page as page_cmd,
    prop as prop_cmd, query as query_cmd, search as search_cmd, tag as tag_cmd,
    template as tpl_cmd, workspace_info as wi_cmd,
};
use crate::mcp::protocol::JsonRpcError;
use crate::mcp::{tool_error_payload, tool_success_payload, ServerCtx};
use crate::output::ApiError;

use super::{opt_params, opt_str, require_str};

/// Tool names that mutate the workspace. After a successful call we
/// invalidate the cached `WorkspaceIndex` so subsequent read-only
/// tools see the freshly written blocks / pages.
///
/// `outl_daily_today` and `outl_daily_get` look read-only by name but
/// belong here: both go through `open_journal`, which creates the
/// journal page when it doesn't exist yet (lazy materialisation
/// keeps the user's daily-note shortcut frictionless). The first
/// call of the day mutates, every call after that is effectively a
/// read — keeping them in this list errs on the safe side and pays
/// only one extra index rebuild.
const MUTATING: &[&str] = &[
    "outl_page_create",
    "outl_page_update",
    "outl_page_delete",
    "outl_page_rename",
    "outl_block_append",
    "outl_block_append_tree",
    "outl_block_insert",
    "outl_block_update",
    "outl_block_move",
    "outl_block_delete",
    "outl_block_toggle_todo",
    "outl_daily_today",
    "outl_daily_get",
    "outl_daily_append",
    "outl_asset_add",
    "outl_page_prop_set",
    "outl_batch",
    "outl_template_apply",
];

/// Dispatch a `tools/call` request to the correct handler.
pub fn call(params: Value, ctx: &Arc<ServerCtx>) -> Result<Value, JsonRpcError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| JsonRpcError::invalid_params("missing `name`"))?;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    let outcome: Result<Value, ApiError> = run_tool(name, &args, ctx);

    if outcome.is_ok() && MUTATING.contains(&name) {
        ctx.invalidate_index();
        // Wake peers now instead of on their next catch-up tick. A no-op when
        // this process didn't win the device endpoint lease — then a
        // co-resident holder pushes these ops out for us. See
        // `ServerCtx::ensure_transport`.
        ctx.announce_local_ops();
    }

    Ok(match outcome {
        Ok(v) => tool_success_payload(name, &v),
        Err(e) => tool_error_payload(&e),
    })
}

fn run_tool(name: &str, args: &Value, ctx: &Arc<ServerCtx>) -> Result<Value, ApiError> {
    match name {
        // --- page ---
        "outl_page_get" => {
            let slug = require_str(args, "slug")?.to_string();
            ctx.with_workspace(|wc| page_cmd::get(wc, &slug))
        }
        "outl_page_create" => {
            let slug = require_str(args, "slug")?.to_string();
            let title = opt_str(args, "title").map(str::to_string);
            let icon = opt_str(args, "icon").map(str::to_string);
            let content_specs: Option<Vec<outl_actions::BlockTreeSpec>> = match args.get("content")
            {
                None | Some(Value::Null) => None,
                Some(v) => Some(serde_json::from_value(v.clone()).map_err(|e| {
                    ApiError::new(
                        crate::output::codes::INVALID_ARG,
                        format!("invalid `content` shape: {e}"),
                    )
                })?),
            };
            ctx.with_workspace(|wc| {
                page_cmd::create(
                    wc,
                    &slug,
                    title.as_deref(),
                    icon.as_deref(),
                    content_specs.as_deref(),
                )
            })
        }
        "outl_page_update" => {
            let slug = require_str(args, "slug")?.to_string();
            let title = opt_str(args, "title").map(str::to_string);
            let icon = opt_str(args, "icon").map(str::to_string);
            ctx.with_workspace(|wc| page_cmd::update(wc, &slug, title.as_deref(), icon.as_deref()))
        }
        "outl_page_delete" => {
            let slug = require_str(args, "slug")?.to_string();
            let confirm = args
                .get("confirm")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !confirm {
                return Err(ApiError::new(
                    crate::output::codes::CONFIRM_REQUIRED,
                    format!("refusing to delete `{slug}` without confirm:true"),
                ));
            }
            ctx.with_workspace(|wc| page_cmd::delete(wc, &slug))
        }
        "outl_page_list" => {
            let filter = opt_str(args, "filter").map(str::to_string);
            ctx.with_workspace(|wc| page_cmd::list(wc, filter.as_deref()))
        }
        "outl_page_rename" => {
            let old = require_str(args, "old_slug")?.to_string();
            let new = require_str(args, "new_slug")?.to_string();
            ctx.with_workspace(|wc| page_cmd::rename(wc, &old, &new))
        }
        "outl_page_render" => {
            let slug = require_str(args, "slug")?.to_string();
            ctx.with_workspace(|wc| page_cmd::render(wc, &slug))
        }

        // --- block ---
        "outl_block_get" => {
            let id = require_str(args, "id")?.to_string();
            ctx.with_workspace(|wc| block_cmd::get(wc, &id))
        }
        "outl_block_append" => {
            let page = opt_str(args, "page").map(str::to_string);
            let parent = opt_str(args, "parent").map(str::to_string);
            let text = require_str(args, "text")?.to_string();
            ctx.with_workspace(|wc| {
                block_cmd::append(wc, page.as_deref(), parent.as_deref(), &text)
            })
        }
        "outl_block_append_tree" => {
            let page = opt_str(args, "page").map(str::to_string);
            let parent = opt_str(args, "parent").map(str::to_string);
            let tree_value = args.get("tree").cloned().ok_or_else(|| {
                ApiError::new(
                    crate::output::codes::INVALID_ARG,
                    "missing required `tree`".to_string(),
                )
            })?;
            let spec: outl_actions::BlockTreeSpec =
                serde_json::from_value(tree_value).map_err(|e| {
                    ApiError::new(
                        crate::output::codes::INVALID_ARG,
                        format!("invalid `tree` shape: {e}"),
                    )
                })?;
            ctx.with_workspace(|wc| {
                block_cmd::append_tree_h(wc, page.as_deref(), parent.as_deref(), &spec)
            })
        }
        "outl_block_insert" => {
            let after = require_str(args, "after")?.to_string();
            let text = require_str(args, "text")?.to_string();
            ctx.with_workspace(|wc| block_cmd::insert(wc, &after, &text))
        }
        "outl_block_update" => {
            let id = require_str(args, "id")?.to_string();
            let text = require_str(args, "text")?.to_string();
            ctx.with_workspace(|wc| block_cmd::update(wc, &id, &text))
        }
        "outl_block_move" => {
            let id = require_str(args, "id")?.to_string();
            let parent = opt_str(args, "parent").map(str::to_string);
            let after = opt_str(args, "after").map(str::to_string);
            ctx.with_workspace(|wc| {
                block_cmd::move_block(wc, &id, parent.as_deref(), after.as_deref())
            })
        }
        "outl_block_delete" => {
            let id = require_str(args, "id")?.to_string();
            let confirm = args
                .get("confirm")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !confirm {
                return Err(ApiError::new(
                    crate::output::codes::CONFIRM_REQUIRED,
                    format!("refusing to delete `{id}` without confirm:true"),
                ));
            }
            ctx.with_workspace(|wc| block_cmd::delete(wc, &id))
        }
        "outl_block_toggle_todo" => {
            let id = require_str(args, "id")?.to_string();
            ctx.with_workspace(|wc| block_cmd::toggle_todo(wc, &id))
        }
        "outl_block_tree" => {
            let id = require_str(args, "id")?.to_string();
            ctx.with_workspace(|wc| block_cmd::tree(wc, &id))
        }

        // --- daily ---
        "outl_daily_today" => ctx.with_workspace(daily_cmd::today_handler),
        "outl_daily_get" => {
            let date = require_str(args, "date")?.to_string();
            ctx.with_workspace(|wc| daily_cmd::get(wc, &date))
        }
        "outl_daily_append" => {
            let text = require_str(args, "text")?.to_string();
            let date = opt_str(args, "date").map(str::to_string);
            ctx.with_workspace(|wc| daily_cmd::append(wc, date.as_deref(), &text))
        }
        "outl_daily_range" => {
            let from = require_str(args, "from")?.to_string();
            let to = require_str(args, "to")?.to_string();
            ctx.with_workspace(|wc| daily_cmd::range(wc, &from, &to))
        }

        // --- asset ---
        "outl_asset_add" => {
            let path = require_str(args, "path")?.to_string();
            let page = opt_str(args, "page").map(str::to_string);
            let daily = args.get("daily").and_then(Value::as_bool).unwrap_or(false);
            let target = asset_cmd::resolve_target(page.as_deref(), daily)?;
            let max_bytes = outl_config::load().assets.max_bytes;
            ctx.with_workspace(|wc| {
                asset_cmd::add_asset(wc, std::path::Path::new(&path), &target, max_bytes)
            })
        }

        // --- search / query ---
        "outl_search" => {
            let query = require_str(args, "query")?.to_string();
            let in_str = opt_str(args, "in").unwrap_or("all").to_string();
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .map(|n| n as usize)
                .unwrap_or(50);
            let search_args = search_cmd::SearchArgs {
                query,
                r#in: in_str,
                limit,
                json: true,
            };
            ctx.with_index(|idx| search_cmd::handler_with_index(idx, &search_args))
        }
        "outl_query" => {
            let q = query_cmd::QueryArgs {
                tag: opt_str(args, "tag").map(str::to_string),
                priority: opt_str(args, "priority").map(str::to_string),
                props: args
                    .get("props")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                since: opt_str(args, "since").map(str::to_string),
                kind: opt_str(args, "kind").map(str::to_string),
                raw: opt_str(args, "raw").map(str::to_string),
                json: true,
            };
            ctx.with_workspace(|wc| query_cmd::handler(wc, &q))
        }

        // --- backlinks / refs ---
        "outl_backlinks" => {
            let slug = require_str(args, "slug")?.to_string();
            ctx.with_workspace(|wc| bl_cmd::page(wc, &slug))
        }
        "outl_block_refs" => {
            let id = require_str(args, "id")?.to_string();
            ctx.with_index(|idx| bl_cmd::block_with_index(idx, &id))
        }
        "outl_block_embed" => {
            let h = require_str(args, "id_or_handle")?.to_string();
            ctx.with_index(|idx| bl_cmd::embed_with_index(idx, &h))
        }

        // --- tag ---
        "outl_tag_list" => ctx.with_workspace(|wc| Ok(tag_cmd::list(wc))),
        "outl_tag_pages" => {
            let tag = require_str(args, "tag")?.to_string();
            ctx.with_workspace(|wc| Ok(tag_cmd::pages(wc, &tag)))
        }

        // --- page prop ---
        "outl_page_prop_set" => {
            let page = require_str(args, "page")?.to_string();
            let key = require_str(args, "key")?.to_string();
            let value = require_str(args, "value")?.to_string();
            ctx.with_workspace(|wc| prop_cmd::set_kv(wc, &page, &key, &value))
        }
        "outl_page_prop_get" => {
            let page = require_str(args, "page")?.to_string();
            let key = require_str(args, "key")?.to_string();
            ctx.with_workspace(|wc| prop_cmd::get(wc, &page, &key))
        }
        "outl_page_prop_list" => {
            let page = require_str(args, "page")?.to_string();
            ctx.with_workspace(|wc| prop_cmd::list(wc, &page))
        }

        // --- export ---
        "outl_export_hugo" => {
            let slug = require_str(args, "slug")?.to_string();
            let out_dir = require_str(args, "out_dir")?.to_string();
            ctx.with_workspace(|wc| exp_cmd::hugo(wc, &slug, std::path::Path::new(&out_dir)))
        }
        "outl_export_md" => {
            let slug = require_str(args, "slug")?.to_string();
            ctx.with_workspace(|wc| exp_cmd::md(wc, &slug))
        }
        "outl_export_json" => {
            let slug = require_str(args, "slug")?.to_string();
            ctx.with_workspace(|wc| exp_cmd::json_ast(wc, &slug))
        }

        // --- batch ---
        "outl_batch" => ctx.with_workspace(|wc| batch_cmd::apply_batch(wc, args)),

        // --- workspace ---
        "outl_workspace_info" => ctx.with_workspace(|wc| Ok(wi_cmd::info(wc))),
        "outl_workspace_doctor" => doctor_cmd::collect_in_session_json(&ctx.workspace_path),

        // --- templates ---
        "outl_template_list" => ctx.with_workspace(|wc| tpl_cmd::list(wc)),
        "outl_template_apply" => {
            let name = require_str(args, "name")?.to_string();
            let page = require_str(args, "page")?.to_string();
            let block = opt_str(args, "block").map(str::to_string);
            ctx.with_workspace(|wc| tpl_cmd::apply(wc, &name, &page, block.as_deref()))
        }
        "outl_template_resolve" => {
            let name = require_str(args, "name")?.to_string();
            ctx.with_workspace(|wc| tpl_cmd::resolve(wc, &name))
        }
        "outl_template_run" => {
            let name = require_str(args, "name")?.to_string();
            let page = require_str(args, "page")?.to_string();
            let block = require_str(args, "block")?.to_string();
            let params = opt_params(args);
            ctx.with_workspace(|wc| tpl_cmd::run_template(wc, &name, &page, &block, &params))
        }

        other => Err(ApiError::new(
            crate::output::codes::INVALID_ARG,
            format!("unknown tool `{other}`"),
        )),
    }
}
