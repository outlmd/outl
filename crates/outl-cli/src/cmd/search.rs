//! `outl search "<query>"` — full-text search over pages and blocks.
//!
//! Powered by `outl_md::WorkspaceIndex`. The index is rebuilt on every
//! invocation; reuse across CLI calls is not worth the daemonization
//! cost at this scale.

use std::path::Path;

use clap::Args;
use serde_json::{json, Value};

use outl_md::index::WorkspaceIndex;

use crate::output::{codes, emit, ApiError};
use crate::ws::{self, WsCtx};

/// Where the search runs.
#[derive(Debug, Clone, Copy)]
enum Scope {
    /// Search inside block text.
    Blocks,
    /// Search inside page titles / slugs.
    Pages,
    /// Search across both.
    All,
}

impl Scope {
    fn parse(s: &str) -> Result<Self, ApiError> {
        match s {
            "blocks" => Ok(Scope::Blocks),
            "pages" => Ok(Scope::Pages),
            "all" => Ok(Scope::All),
            other => Err(ApiError::new(
                codes::INVALID_ARG,
                format!("--in must be one of blocks|pages|all, got `{other}`"),
            )),
        }
    }
}

/// Args for `outl search`.
#[derive(Args, Debug)]
pub struct SearchArgs {
    /// Query string.
    pub query: String,
    /// Where to search: `blocks` | `pages` | `all` (default).
    #[arg(long, default_value = "all")]
    pub r#in: String,
    /// Maximum hits per category.
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
    /// Force JSON output.
    #[arg(long)]
    pub json: bool,
}

/// Run a `outl search` invocation.
pub fn run(args: &SearchArgs, path: &Path) -> i32 {
    let result = ws::open(path).and_then(|ctx| handler(&ctx, args));
    emit(args.json, result, print_results)
}

/// Pure handler — used by the CLI path (derives the workspace index
/// per call).
///
/// Derived from the op log, not from `pages/`: `ctx.workspace` is
/// already replayed by the time we get here, so walking every `.md`
/// and parsing it would be a second, slower derivation of state we
/// hold in memory. Measured on a 2,835-page workspace: the walk was
/// ~250 ms of the command's ~650 ms, ~160 ms of which was raw file
/// I/O.
///
/// Consequence worth knowing: a line that lives in a `.md` but in no
/// op is not searchable here. That is root `CLAUDE.md` invariant 8's
/// state, and `outl reconcile --ahead-of-log` is what turns such
/// content into ops.
pub fn handler(ctx: &WsCtx, args: &SearchArgs) -> Result<Value, ApiError> {
    let index = outl_actions::index::derive(&ctx.workspace, &ctx.root);
    handler_with_index(&index, args)
}

/// Variant that consumes a pre-built [`WorkspaceIndex`]. Used by the
/// MCP shim so it can amortise the index across read-only calls.
pub fn handler_with_index(index: &WorkspaceIndex, args: &SearchArgs) -> Result<Value, ApiError> {
    let scope = Scope::parse(&args.r#in)?;
    let mut data = serde_json::Map::new();

    if matches!(scope, Scope::Blocks | Scope::All) {
        let hits = index.search_block_text(&args.query, args.limit);
        let payload: Vec<Value> = hits
            .into_iter()
            .map(|entry| {
                json!({
                    "id": entry.id.to_string(),
                    "handle": entry.ref_handle,
                    "text": entry.text,
                    "page": entry.source_slug,
                    "path": entry.source_path.display().to_string(),
                })
            })
            .collect();
        data.insert("blocks".to_string(), Value::Array(payload));
    }

    if matches!(scope, Scope::Pages | Scope::All) {
        let hits = index.pages_by_title_prefix(&args.query, args.limit);
        let payload: Vec<Value> = hits
            .into_iter()
            .map(|p| {
                json!({
                    "slug": p.slug,
                    "title": p.title,
                    "icon": p.icon,
                    "is_journal": p.is_journal,
                    "path": p.path.display().to_string(),
                })
            })
            .collect();
        data.insert("pages".to_string(), Value::Array(payload));
    }

    Ok(Value::Object(data))
}

fn print_results(v: &Value) {
    if let Some(blocks) = v.get("blocks").and_then(Value::as_array) {
        if !blocks.is_empty() {
            println!("# blocks");
            for b in blocks {
                let page = b.get("page").and_then(Value::as_str).unwrap_or("?");
                let handle = b.get("handle").and_then(Value::as_str).unwrap_or("?");
                let text = b.get("text").and_then(Value::as_str).unwrap_or("");
                println!("  [{page}] (({handle}))  {text}");
            }
        }
    }
    if let Some(pages) = v.get("pages").and_then(Value::as_array) {
        if !pages.is_empty() {
            println!("# pages");
            for p in pages {
                let slug = p.get("slug").and_then(Value::as_str).unwrap_or("?");
                let title = p.get("title").and_then(Value::as_str).unwrap_or("?");
                println!("  {slug}  {title}");
            }
        }
    }
}
