//! `outl template …` — template operations.
//!
//! Each handler returns a `serde_json::Value` so the same body
//! powers both the CLI (`emit` → human or `--json`) and the MCP
//! shim (raw payload returned as the tool result).

use std::path::Path;

use chrono::NaiveDate;
use clap::Subcommand;
use serde_json::{json, Value};

use outl_actions::{
    instantiate_template, list_templates, resolve_call, run_callable_block, ExecOutputDto,
};
use outl_core::id::NodeId;
use outl_exec::RuntimeRegistry;

use crate::output::{codes, emit, ApiError};
use crate::ws::{self, WsCtx};

/// `outl template …` subcommands.
#[derive(Subcommand, Debug)]
pub enum TemplateCommand {
    /// List every template in the workspace.
    List {
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Instantiate a structural template under a target block.
    Apply {
        /// Template name (the value of `template::` on the template page).
        name: String,
        /// Target page slug (where the template will be instantiated).
        #[arg(long)]
        page: String,
        /// Target block id to instantiate under. When omitted, the
        /// template is appended at the end of the page. Must belong to
        /// `--page`; a block on another page is rejected.
        #[arg(long)]
        block: Option<String>,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Resolve a callable template (show its code block + params).
    Resolve {
        /// Template name.
        name: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Execute a callable template and write its `> **result:**` subtree
    /// under the target block.
    Run {
        /// Template name (the value of `template::` on the template page).
        name: String,
        /// Page slug that owns the anchor block.
        #[arg(long)]
        page: String,
        /// Anchor block id: the `> **result:**` subtree is written under
        /// it. Must belong to `--page`.
        #[arg(long)]
        block: String,
        /// Template parameter as `key=value`. Repeatable.
        #[arg(long = "params", value_name = "KEY=VALUE")]
        params: Vec<String>,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
}

/// Run an `outl template …` invocation.
pub fn run(cmd: &TemplateCommand, path: &Path) -> i32 {
    match cmd {
        TemplateCommand::List { json } => {
            let result = ws::open(path).and_then(|ctx| list(&ctx));
            emit(*json, result, print_template_list)
        }
        TemplateCommand::Apply {
            name,
            page,
            block,
            json,
        } => {
            let result =
                ws::open(path).and_then(|mut ctx| apply(&mut ctx, name, page, block.as_deref()));
            emit(*json, result, print_template_apply)
        }
        TemplateCommand::Resolve { name, json } => {
            let result = ws::open(path).and_then(|ctx| resolve(&ctx, name));
            emit(*json, result, print_template_resolve)
        }
        TemplateCommand::Run {
            name,
            page,
            block,
            params,
            json,
        } => {
            let result = ws::open(path)
                .and_then(|mut ctx| run_template(&mut ctx, name, page, block, params));
            emit(*json, result, print_template_run)
        }
    }
}

/// List all templates.
pub fn list(ctx: &WsCtx) -> Result<Value, ApiError> {
    let templates = list_templates(&ctx.workspace);
    Ok(json!({
        "templates": templates,
        "count": templates.len(),
    }))
}

/// Instantiate a template.
pub fn apply(
    ctx: &mut WsCtx,
    name: &str,
    page_slug: &str,
    block_id: Option<&str>,
) -> Result<Value, ApiError> {
    let page = outl_actions::find_by_slug(&ctx.workspace, page_slug).ok_or_else(|| {
        ApiError::new(
            codes::PAGE_NOT_FOUND,
            format!("page `{page_slug}` not found"),
        )
    })?;

    let target = match block_id {
        Some(id) => resolve_block_on_page(&ctx.workspace, id, page, page_slug)?,
        None => page,
    };

    let page_date = NaiveDate::parse_from_str(page_slug, "%Y-%m-%d").ok();

    let new_ids = instantiate_template(
        &mut ctx.workspace,
        &ctx.hlc,
        name,
        target,
        page_slug,
        page_date,
    )?;

    let id_strings: Vec<String> = new_ids.iter().map(|id| id.to_string()).collect();

    // Re-render the page projection so the new blocks appear on disk.
    // Guarded, and propagated: `page_slug` is an *existing* page here
    // (resolved above via `find_by_slug`), so its `.md` can already hold
    // content the op log never recorded. The unconditional writer used to
    // sit here and would have silently deleted that content instead of
    // refusing (RFC 0255) — the same bug the CLI/MCP write paths had, one
    // file over. A refusal must reach the caller: the ops just applied are
    // safe in the log, but the user asked for a projection and needs to
    // know it did not land on disk.
    outl_actions::apply_page_md_with_sidecar_guarded(&ctx.workspace, &ctx.root, page)?;

    Ok(json!({
        "template": name,
        "page": page_slug,
        "created_blocks": id_strings,
        "count": new_ids.len(),
    }))
}

/// Resolve a callable template.
pub fn resolve(ctx: &WsCtx, name: &str) -> Result<Value, ApiError> {
    let resolution = resolve_call(&ctx.workspace, name)?;
    Ok(json!({
        "template_slug": resolution.template_slug,
        "language": resolution.language,
        "source": resolution.source,
        "params": resolution.params,
    }))
}

/// Execute a callable template under an anchor block on `page_slug`.
pub fn run_template(
    ctx: &mut WsCtx,
    name: &str,
    page_slug: &str,
    block_id: &str,
    params: &[String],
) -> Result<Value, ApiError> {
    let page = outl_actions::find_by_slug(&ctx.workspace, page_slug).ok_or_else(|| {
        ApiError::new(
            codes::PAGE_NOT_FOUND,
            format!("page `{page_slug}` not found"),
        )
    })?;
    let anchor = resolve_block_on_page(&ctx.workspace, block_id, page, page_slug)?;
    let parsed = parse_params(params)?;

    let registry = RuntimeRegistry::with_builtins();
    let out = run_callable_block(
        &mut ctx.workspace,
        &ctx.hlc,
        &registry,
        name,
        &parsed,
        anchor,
    )
    .map_err(|e| ApiError::new(codes::INVALID_ARG, e.to_string()))?;

    // Re-render the page projection so the `> **result:**` subtree lands on
    // disk. Guarded + propagated for the same reason as `apply` above: this
    // is an existing page's `.md`, and a frozen one must refuse rather than
    // silently drop unlogged content (RFC 0255).
    outl_actions::apply_page_md_with_sidecar_guarded(&ctx.workspace, &ctx.root, page)?;

    let dto = ExecOutputDto::from(&out);
    Ok(json!({
        "template": name,
        "page": page_slug,
        "block": block_id,
        "result": dto,
    }))
}

/// Parse a block id, ensure it resolves to a real node, and confirm the
/// node lives on `page` (`page_slug`). Cross-page targets are rejected
/// with `INVALID_ARG` — instantiating under a block on another page then
/// reprojecting only `--page` silently drops the new blocks from disk.
fn resolve_block_on_page(
    workspace: &outl_core::workspace::Workspace,
    block_id: &str,
    page: NodeId,
    page_slug: &str,
) -> Result<NodeId, ApiError> {
    let ulid = ulid::Ulid::from_string(block_id)
        .map_err(|_| ApiError::new(codes::INVALID_ARG, format!("invalid block id `{block_id}`")))?;
    let target = NodeId(ulid);

    // The page node itself is a valid anchor (append at page level).
    if target == page {
        return Ok(target);
    }
    let owner = outl_actions::enclosing_page_id(workspace, target).ok_or_else(|| {
        ApiError::new(
            codes::BLOCK_NOT_FOUND,
            format!("block `{block_id}` not found"),
        )
    })?;
    if owner != page {
        return Err(ApiError::new(
            codes::INVALID_ARG,
            format!("block `{block_id}` does not belong to page `{page_slug}`"),
        ));
    }
    Ok(target)
}

/// Parse repeated `key=value` params into ordered pairs.
fn parse_params(raw: &[String]) -> Result<Vec<(String, String)>, ApiError> {
    raw.iter()
        .map(|kv| {
            kv.split_once('=')
                .map(|(k, v)| (k.trim().to_string(), v.to_string()))
                .filter(|(k, _)| !k.is_empty())
                .ok_or_else(|| {
                    ApiError::new(
                        codes::INVALID_ARG,
                        format!("invalid param `{kv}` — expected `key=value`"),
                    )
                })
        })
        .collect()
}

fn print_template_list(v: &Value) {
    if let Some(templates) = v.get("templates").and_then(Value::as_array) {
        if templates.is_empty() {
            println!("No templates found.");
            println!("Create one by adding `template:: <name>` to any page.");
            return;
        }
        for t in templates {
            let name = t.get("name").and_then(Value::as_str).unwrap_or("?");
            let slug = t.get("slug").and_then(Value::as_str).unwrap_or("?");
            let params = t.get("params").and_then(Value::as_array);
            match params {
                Some(p) if !p.is_empty() => {
                    let pnames: Vec<&str> = p.iter().filter_map(Value::as_str).collect();
                    println!("  {name:<20} {slug:<30} params: {pnames:?}");
                }
                _ => println!("  {name:<20} {slug}"),
            }
        }
        if let Some(count) = v.get("count").and_then(Value::as_u64) {
            println!("\n  {count} template(s)");
        }
    }
}

fn print_template_apply(v: &Value) {
    let name = v.get("template").and_then(Value::as_str).unwrap_or("?");
    let page = v.get("page").and_then(Value::as_str).unwrap_or("?");
    let count = v.get("count").and_then(Value::as_u64).unwrap_or(0);
    println!("Instantiated template `{name}` into `{page}` ({count} block(s) created)");
}

fn print_template_run(v: &Value) {
    let name = v.get("template").and_then(Value::as_str).unwrap_or("?");
    let page = v.get("page").and_then(Value::as_str).unwrap_or("?");
    println!("Ran template `{name}` on `{page}`");
    if let Some(result) = v.get("result") {
        let stdout = result.get("stdout").and_then(Value::as_str).unwrap_or("");
        let exit = result.get("exit").and_then(Value::as_str).unwrap_or("?");
        println!("Exit:   {exit}");
        if !stdout.trim().is_empty() {
            println!("Result:");
            for line in stdout.trim().lines() {
                println!("  {line}");
            }
        }
    }
}

fn print_template_resolve(v: &Value) {
    let slug = v
        .get("template_slug")
        .and_then(Value::as_str)
        .unwrap_or("?");
    let lang = v.get("language").and_then(Value::as_str).unwrap_or("?");
    let params = v.get("params").and_then(Value::as_array);
    println!("Template: {slug}");
    println!("Language: {lang}");
    if let Some(p) = params {
        if !p.is_empty() {
            let pnames: Vec<&str> = p.iter().filter_map(Value::as_str).collect();
            println!("Params:   {pnames:?}");
        }
    }
}
