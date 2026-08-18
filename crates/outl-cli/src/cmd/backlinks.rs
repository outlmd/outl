//! `outl backlinks …` — who references what.
//!
//! Page-level: `[[slug]]` and `[[title]]` mentions across the workspace.
//! Block-level: `((blk-XXXXXX))` references resolved via the workspace
//! block index.

use std::path::Path;

use clap::Subcommand;
use serde_json::{json, Value};

use outl_actions::{backlinks_for_page, find_by_slug, page_meta};
use outl_md::index::WorkspaceIndex;

use crate::cmd::block as block_cmd;
use crate::human::print_outline_node;
use crate::output::{codes, emit, ApiError};
use crate::ws::{self, WsCtx};

/// `outl backlinks …` subcommands.
#[derive(Subcommand, Debug)]
pub enum BacklinksCommand {
    /// Pages that mention `[[<slug>]]` or `[[<title>]]`.
    Page {
        /// Page slug.
        slug: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Blocks that reference `((blk-XXXXXX))`.
    Block {
        /// Block id (full ULID).
        id: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Resolve `!((blk-XXXXXX))` recursively — source block + children.
    Embed {
        /// Block id or short handle (`blk-XXXXXX`).
        id_or_handle: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
}

/// Run a `outl backlinks …` invocation.
pub fn run(cmd: &BacklinksCommand, path: &Path) -> i32 {
    match cmd {
        BacklinksCommand::Page { slug, json } => {
            let result = ws::open(path).and_then(|ctx| page(&ctx, slug));
            emit(*json, result, print_page)
        }
        BacklinksCommand::Block { id, json } => {
            let result = ws::open(path).and_then(|ctx| block(&ctx, id));
            emit(*json, result, print_block)
        }
        BacklinksCommand::Embed { id_or_handle, json } => {
            let result = ws::open(path).and_then(|ctx| embed(&ctx, id_or_handle));
            emit(*json, result, print_embed)
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Page-level backlinks.
pub fn page(ctx: &WsCtx, slug: &str) -> Result<Value, ApiError> {
    let id = find_by_slug(&ctx.workspace, slug)
        .ok_or_else(|| ApiError::new(codes::PAGE_NOT_FOUND, format!("page `{slug}` not found")))?;
    let meta = page_meta(&ctx.workspace, id)
        .ok_or_else(|| ApiError::new(codes::INTERNAL, "page meta missing".to_string()))?;
    let links = backlinks_for_page(&ctx.workspace, &ctx.root, &meta);
    Ok(json!({
        "page": meta,
        "backlinks": links,
    }))
}

/// Block-level references — CLI path (derives the index per call).
///
/// Derived from the op log rather than walked off disk; see
/// [`crate::cmd::search::handler`] for the reasoning and the cost.
pub fn block(ctx: &WsCtx, id_str: &str) -> Result<Value, ApiError> {
    let index = outl_actions::index::derive(&ctx.workspace, &ctx.root);
    block_with_index(&index, id_str)
}

/// Variant taking a pre-built [`WorkspaceIndex`]. Used by MCP.
pub fn block_with_index(index: &WorkspaceIndex, id_str: &str) -> Result<Value, ApiError> {
    let id = block_cmd::parse_id(id_str)?;
    let refs = index.block_refs_to(id);
    Ok(json!({
        "block_id": id.to_string(),
        "references": refs
            .iter()
            .map(|r| json!({
                "source_page": r.source_slug,
                "source_block_path": r.source_block_path,
            }))
            .collect::<Vec<_>>(),
    }))
}

/// Resolve an embed (`!((…))`) — CLI path.
pub fn embed(ctx: &WsCtx, id_or_handle: &str) -> Result<Value, ApiError> {
    let index = outl_actions::index::derive(&ctx.workspace, &ctx.root);
    embed_with_index(&index, id_or_handle)
}

/// Variant taking a pre-built [`WorkspaceIndex`]. Used by MCP.
pub fn embed_with_index(index: &WorkspaceIndex, id_or_handle: &str) -> Result<Value, ApiError> {
    let entry = if id_or_handle.starts_with("blk-") {
        index.resolve_block_ref(id_or_handle)
    } else {
        let id = block_cmd::parse_id(id_or_handle)?;
        index.block_by_id(id)
    }
    .ok_or_else(|| {
        ApiError::new(
            codes::BLOCK_NOT_FOUND,
            format!("block `{id_or_handle}` not indexed"),
        )
    })?;
    Ok(json!({
        "id": entry.id.to_string(),
        "handle": entry.ref_handle,
        "page": entry.source_slug,
        "text": entry.text,
        "children": serde_json::to_value(&entry.children).map_err(ApiError::internal)?,
    }))
}

// ---------------------------------------------------------------------------
// Human formatters
// ---------------------------------------------------------------------------

fn print_page(v: &Value) {
    let slug = v
        .pointer("/page/slug")
        .and_then(Value::as_str)
        .unwrap_or("?");
    println!("backlinks → {slug}");
    if let Some(links) = v.get("backlinks").and_then(Value::as_array) {
        for link in links {
            let src = link
                .pointer("/source_page/slug")
                .and_then(Value::as_str)
                .unwrap_or("?");
            let text = link.get("block_text").and_then(Value::as_str).unwrap_or("");
            println!("  [{src}] {text}");
        }
    }
}

fn print_block(v: &Value) {
    let id = v.get("block_id").and_then(Value::as_str).unwrap_or("?");
    println!("refs → {id}");
    if let Some(refs) = v.get("references").and_then(Value::as_array) {
        for r in refs {
            let page = r.get("source_page").and_then(Value::as_str).unwrap_or("?");
            println!("  [{page}]");
        }
    }
}

fn print_embed(v: &Value) {
    let id = v.get("id").and_then(Value::as_str).unwrap_or("?");
    let text = v.get("text").and_then(Value::as_str).unwrap_or("");
    println!("{id}  {text}");
    if let Some(children) = v.get("children").and_then(Value::as_array) {
        for child in children {
            print_outline_node(child, 1);
        }
    }
}
