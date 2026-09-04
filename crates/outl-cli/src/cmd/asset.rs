//! `outl asset …` — file/asset import.
//!
//! Glue only: the copy + content-hash + link generation live in
//! `outl_actions::import_asset`, and the block append routes through
//! `outl-actions` so the op log stays source of truth. This module just
//! resolves the target (daily / page), calls the shared `add_asset`
//! helper, and JSON-envelopes the result. The same `add_asset` runs
//! behind the `outl_asset_add` MCP tool so the two surfaces can't drift.

use std::path::Path;
use std::path::PathBuf;

use clap::Subcommand;
use serde_json::{json, Value};

use outl_actions::{
    append_block, apply_page_md_with_sidecar_guarded, find_by_slug, import_asset, journal_slug,
    open_today, today, ActionError,
};

use crate::output::{codes, emit, ApiError};
use crate::ws::{self, WsCtx};

/// `outl asset …` subcommands.
#[derive(Subcommand, Debug)]
pub enum AssetCommand {
    /// Import a file into the workspace and append its markdown link as
    /// a new block on the daily (default) or a page.
    Add {
        /// Path to the file to import (PDF, image, …).
        file: PathBuf,
        /// Append the link to this page (by slug) instead of the daily.
        #[arg(long)]
        page: Option<String>,
        /// Append the link to today's journal (the default target).
        #[arg(long)]
        daily: bool,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
}

/// Where the imported asset's link block lands.
#[derive(Debug)]
pub enum AssetTarget {
    /// Today's journal (the journal-first default).
    Daily,
    /// A page identified by slug.
    Page(String),
}

/// Run an `outl asset …` invocation.
pub fn run(cmd: &AssetCommand, path: &Path) -> i32 {
    match cmd {
        AssetCommand::Add {
            file,
            page,
            daily,
            json,
        } => {
            let result = resolve_target(page.as_deref(), *daily).and_then(|target| {
                ws::open(path).and_then(|mut ctx| {
                    let max_bytes = outl_config::load().assets.max_bytes;
                    add_asset(&mut ctx, file, &target, max_bytes)
                })
            });
            emit(*json, result, print_added)
        }
    }
}

// ---------------------------------------------------------------------------
// Shared handler (CLI + MCP)
// ---------------------------------------------------------------------------

/// Resolve the CLI/MCP `--page` / `--daily` pair to a target. Mutually
/// exclusive; defaults to the daily when neither points at a page.
pub(crate) fn resolve_target(page: Option<&str>, daily: bool) -> Result<AssetTarget, ApiError> {
    match (page, daily) {
        (Some(_), true) => Err(ApiError::new(
            codes::INVALID_ARG,
            "use either --page or --daily, not both".to_string(),
        )),
        (Some(slug), false) => Ok(AssetTarget::Page(slug.to_string())),
        (None, _) => Ok(AssetTarget::Daily),
    }
}

/// Import `source` into `<workspace>/assets/` and append its markdown
/// link as a new block on the target page/daily. Shared by the CLI
/// handler and the `outl_asset_add` MCP tool so both stay in lockstep.
pub fn add_asset(
    ctx: &mut WsCtx,
    source: &Path,
    target: &AssetTarget,
    max_bytes: u64,
) -> Result<Value, ApiError> {
    // Resolve the target page FIRST, before importing: a missing `--page`
    // must fail without leaving an unreferenced blob on disk (which would
    // waste storage and replicate to peers).
    let (page_id, target_json) = match target {
        AssetTarget::Daily => {
            let id = open_today(&mut ctx.workspace, &ctx.hlc).map_err(ApiError::internal)?;
            (
                id,
                json!({ "kind": "daily", "date": journal_slug(today()) }),
            )
        }
        AssetTarget::Page(slug) => {
            let id = find_by_slug(&ctx.workspace, slug).ok_or_else(|| {
                ApiError::new(codes::PAGE_NOT_FOUND, format!("page `{slug}` not found"))
            })?;
            (id, json!({ "kind": "page", "page": slug }))
        }
    };

    // Copy + content-hash + link generation is entirely upstream; this
    // never touches the op log (an asset's bytes are not workspace state).
    let imported = import_asset(&ctx.root, source, max_bytes).map_err(map_import_error)?;

    // The link is ordinary workspace state — it goes through the op log
    // as a plain block append, never a hand-written `.md` edit.
    let block_id = append_block(
        &mut ctx.workspace,
        &ctx.hlc,
        Some(page_id),
        Some(&imported.markdown),
    )
    .map_err(ApiError::internal)?;

    apply_page_md_with_sidecar_guarded(&ctx.workspace, &ctx.root, page_id)?;

    Ok(json!({
        "block_id": block_id.to_string(),
        "rel_path": imported.rel_path,
        "display_name": imported.display_name,
        "is_image": imported.is_image,
        "markdown": imported.markdown,
        "target": target_json,
    }))
}

/// Translate an import failure into a stable error code. A too-large
/// file or an unreadable/absent source is user input (`INVALID_ARG`);
/// anything else is internal.
fn map_import_error(err: ActionError) -> ApiError {
    match err {
        ActionError::AssetTooLarge { .. } => ApiError::new(codes::INVALID_ARG, err.to_string()),
        ActionError::Io(_) => ApiError::new(codes::INVALID_ARG, err.to_string()),
        other => ApiError::internal(other),
    }
}

// ---------------------------------------------------------------------------
// Human formatter
// ---------------------------------------------------------------------------

fn print_added(v: &Value) {
    let rel = v.get("rel_path").and_then(Value::as_str).unwrap_or("?");
    let md = v.get("markdown").and_then(Value::as_str).unwrap_or("");
    let target = v
        .get("target")
        .and_then(|t| t.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or("?");
    println!("imported {rel} → {target}");
    println!("  {md}");
}
