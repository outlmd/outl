//! `outl tag …` — tag listing and lookup.
//!
//! Tags are extracted from block text by tokenizing on the
//! `#identifier` form (single token, ASCII-safe characters). We avoid
//! parsing markdown beyond that — `outl_md::inline::tokenize` already
//! distinguishes `#tag` from `#` headings inside the workspace index,
//! so we route through it.

use std::collections::HashMap;
use std::path::Path;

use clap::Subcommand;
use serde_json::{json, Value};

use outl_actions::{list_pages, walk_subtree, PageMeta};
use outl_core::id::NodeId;
use outl_md::inline::{tokenize, InlineTok};

use crate::output::emit;
use crate::ws::{self, WsCtx};

/// `outl tag …` subcommands.
#[derive(Subcommand, Debug)]
pub enum TagCommand {
    /// List every tag in the workspace with usage counts.
    List {
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// List pages whose subtree contains `#<tag>`.
    Pages {
        /// Tag name without the leading `#`.
        tag: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
}

/// Run a `outl tag …` invocation.
pub fn run(cmd: &TagCommand, path: &Path) -> i32 {
    match cmd {
        TagCommand::List { json } => {
            let result = ws::open_read(path).map(|ctx| list(&ctx));
            emit(*json, result, print_tag_list)
        }
        TagCommand::Pages { tag, json } => {
            let result = ws::open_read(path).map(|ctx| pages(&ctx, tag));
            emit(*json, result, print_pages)
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Map of tag → (count, pages) across the whole workspace.
pub fn list(ctx: &WsCtx) -> Value {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut tag_pages: HashMap<String, Vec<String>> = HashMap::new();
    for meta in list_pages(&ctx.workspace) {
        let Some(id) = ulid::Ulid::from_string(&meta.id).ok().map(NodeId) else {
            continue;
        };
        let found = collect_tags_under(&ctx.workspace, id);
        for tag in &found {
            *counts.entry(tag.clone()).or_default() += 1;
            tag_pages
                .entry(tag.clone())
                .or_default()
                .push(meta.slug.clone());
        }
    }
    let mut tags: Vec<Value> = counts
        .into_iter()
        .map(|(tag, count)| {
            json!({
                "tag": tag,
                "count": count,
                "pages": tag_pages.get(&tag).cloned().unwrap_or_default(),
            })
        })
        .collect();
    tags.sort_by(|a, b| {
        b.get("count")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .cmp(&a.get("count").and_then(Value::as_u64).unwrap_or(0))
    });
    json!({ "tags": tags })
}

/// All pages whose subtree contains `#tag`.
pub fn pages(ctx: &WsCtx, tag: &str) -> Value {
    let needle = strip_leading_hash(tag);
    let mut hits: Vec<PageMeta> = Vec::new();
    for meta in list_pages(&ctx.workspace) {
        let Some(id) = ulid::Ulid::from_string(&meta.id).ok().map(NodeId) else {
            continue;
        };
        if collect_tags_under(&ctx.workspace, id).contains(&needle) {
            hits.push(meta);
        }
    }
    json!({
        "tag": needle,
        "pages": hits,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn collect_tags_under(workspace: &outl_core::workspace::Workspace, parent: NodeId) -> Vec<String> {
    let mut out = Vec::new();
    walk_subtree(workspace, parent, |id| {
        if let Some(text) = workspace.block_text(id) {
            for tok in tokenize(&text) {
                if let InlineTok::Tag { name } = tok {
                    out.push(name.to_string());
                }
            }
        }
        true
    });
    out.sort();
    out.dedup();
    out
}

fn strip_leading_hash(tag: &str) -> String {
    tag.trim_start_matches('#').to_string()
}

// ---------------------------------------------------------------------------
// Human formatters
// ---------------------------------------------------------------------------

fn print_tag_list(v: &Value) {
    if let Some(tags) = v.get("tags").and_then(Value::as_array) {
        for t in tags {
            let tag = t.get("tag").and_then(Value::as_str).unwrap_or("?");
            let count = t.get("count").and_then(Value::as_u64).unwrap_or(0);
            println!("#{tag:30}  {count}");
        }
    }
}

fn print_pages(v: &Value) {
    let tag = v.get("tag").and_then(Value::as_str).unwrap_or("?");
    println!("#{tag}");
    if let Some(pages) = v.get("pages").and_then(Value::as_array) {
        for p in pages {
            let slug = p.get("slug").and_then(Value::as_str).unwrap_or("?");
            let title = p.get("title").and_then(Value::as_str).unwrap_or("?");
            println!("  {slug}  {title}");
        }
    }
}
