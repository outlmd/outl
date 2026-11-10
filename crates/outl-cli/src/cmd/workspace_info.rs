//! `outl workspace info` — workspace summary (path, actor, counts).
//!
//! `ops` is the persisted op-log total, read through
//! [`outl_core::workspace::Workspace::total_ops`]. That includes ops
//! folded into a snapshot boot as well as the post-cutoff delta held in
//! memory, so it matches what `outl doctor` reports without paying for a
//! full replay.
//!
//! The remaining fields are cheap projections of the materialized tree:
//! they walk indexes or in-memory maps, not the raw op log, so this stays
//! suitable for a command a user runs repeatedly to see how big their
//! graph is.

use std::collections::HashSet;
use std::path::Path;

use clap::Args;
use serde_json::{json, Value};

use outl_actions::{asset, backlinks_index::build_backlink_index, list_pages, PageKind};
use outl_core::id::NodeId;
use outl_core::property::PropValue;

use crate::output::{emit, ApiError};
use crate::ws::{self, WsCtx};

/// Args for `outl workspace info`.
#[derive(Args, Debug)]
pub struct WorkspaceInfoArgs {
    /// Force JSON output.
    #[arg(long)]
    pub json: bool,
}

/// Run `outl workspace info`.
pub fn run(args: &WorkspaceInfoArgs, path: &Path) -> i32 {
    let result = ws::open_read(path).and_then(|ctx| info(&ctx).map_err(ApiError::internal));
    emit(args.json, result, print_info)
}

/// Count every unique tag name stored in property values.
fn count_tags(tree: &outl_core::tree::Tree) -> usize {
    fn collect_tags(value: &PropValue, out: &mut HashSet<String>) {
        match value {
            PropValue::Tag(name) => {
                out.insert(name.clone());
            }
            PropValue::List(items) => {
                for item in items {
                    collect_tags(item, out);
                }
            }
            _ => {}
        }
    }

    let mut tags = HashSet::new();
    for (_, _, value) in tree.iter_properties() {
        collect_tags(value, &mut tags);
    }
    tags.len()
}

/// Count files directly inside the workspace `assets/` directory.
fn count_assets(root: &Path) -> usize {
    std::fs::read_dir(asset::assets_dir(root))
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
                .count()
        })
        .unwrap_or(0)
}

/// Count snapshot cache files and their total byte size.
fn snapshot_stats(root: &Path) -> (usize, u64) {
    let dir = root.join(".outl").join("snapshots");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return (0, 0);
    };
    let mut count = 0usize;
    let mut bytes = 0u64;
    for entry in rd.filter_map(|e| e.ok()) {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_file() {
            count += 1;
            bytes += meta.len();
        }
    }
    (count, bytes)
}

/// Format bytes as human-readable (e.g. 1.2 MB).
fn format_bytes(n: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    if n == 0 {
        return "0 B".to_string();
    }
    let exp = (n.ilog10() / 3).min(UNITS.len() as u32 - 1) as usize;
    let divisor = 10u64.pow(exp as u32 * 3);
    let value = n as f64 / divisor as f64;
    format!("{value:.1} {}", UNITS[exp])
}

/// Group a count into thousands (`228943` → `228,944`), matching the
/// `docs/cli.md` example a user copies these numbers out of.
fn fmt_count(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// Pure handler for the MCP shim.
pub fn info(ctx: &WsCtx) -> Result<Value, outl_core::workspace::WorkspaceError> {
    let pages = list_pages(&ctx.workspace);
    let mut journals = 0usize;
    let mut regular = 0usize;
    for p in &pages {
        match p.kind {
            PageKind::Journal => journals += 1,
            PageKind::Page => regular += 1,
        }
    }
    let total_ops = ctx.workspace.total_ops()?;
    let ops_since_snapshot = ctx.workspace.log().len();
    let tree_nodes = ctx.workspace.tree().node_count();
    let blocks = tree_nodes.saturating_sub(regular + journals);
    let trashed = ctx
        .workspace
        .tree()
        .iter_nodes()
        .filter(|(_, parent, _)| *parent == NodeId::trash())
        .count();
    let (snapshot_count, snapshot_bytes) = snapshot_stats(&ctx.root);
    let last_modified = ctx.workspace.last_op_time()?.map(|t| {
        chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    });
    Ok(json!({
        "root": ctx.root.display().to_string(),
        "actor": ctx.actor.to_string(),
        "ephemeral_actor": ctx.ephemeral_actor,
        "ops_dir": ctx.paths.ops.display().to_string(),
        "pages_dir": ctx.paths.pages.display().to_string(),
        "journals_dir": ctx.paths.journals.display().to_string(),
        "pages": regular,
        "journals": journals,
        "blocks": blocks,
        "tree_nodes": tree_nodes,
        "properties": ctx.workspace.tree().property_count(),
        "tags": count_tags(ctx.workspace.tree()),
        "backlinks": build_backlink_index(&ctx.workspace, &ctx.root).len(),
        "assets": count_assets(&ctx.root),
        "trashed": trashed,
        "snapshots": snapshot_count,
        "snapshot_bytes": snapshot_bytes,
        "last_modified": last_modified,
        "ops": total_ops,
        "ops_since_snapshot": ops_since_snapshot,
    }))
}

fn print_info(v: &Value) {
    let root = v.get("root").and_then(Value::as_str).unwrap_or("?");
    let actor = v.get("actor").and_then(Value::as_str).unwrap_or("?");
    let pages = v.get("pages").and_then(Value::as_u64).unwrap_or(0);
    let journals = v.get("journals").and_then(Value::as_u64).unwrap_or(0);
    let blocks = v.get("blocks").and_then(Value::as_u64).unwrap_or(0);
    let tree_nodes = v.get("tree_nodes").and_then(Value::as_u64).unwrap_or(0);
    let properties = v.get("properties").and_then(Value::as_u64).unwrap_or(0);
    let tags = v.get("tags").and_then(Value::as_u64).unwrap_or(0);
    let backlinks = v.get("backlinks").and_then(Value::as_u64).unwrap_or(0);
    let assets = v.get("assets").and_then(Value::as_u64).unwrap_or(0);
    let trashed = v.get("trashed").and_then(Value::as_u64).unwrap_or(0);
    let snapshot_count = v.get("snapshots").and_then(Value::as_u64).unwrap_or(0);
    let snapshot_size = v
        .get("snapshot_bytes")
        .and_then(Value::as_u64)
        .map(format_bytes)
        .unwrap_or_else(|| "0 B".to_string());
    let last_modified = v.get("last_modified").and_then(Value::as_str);
    let ops = v.get("ops").and_then(Value::as_u64).unwrap_or(0);
    let ops_since_snapshot = v
        .get("ops_since_snapshot")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let ephemeral = v
        .get("ephemeral_actor")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    println!("root:      {root}");
    if ephemeral {
        println!("actor:     {actor} (ephemeral — another outl owns the config actor)");
    } else {
        println!("actor:     {actor}");
    }
    println!("pages:     {}", fmt_count(pages));
    println!("journals:  {}", fmt_count(journals));
    println!(
        "blocks:    {} ({} total nodes)",
        fmt_count(blocks),
        fmt_count(tree_nodes)
    );
    println!("props:     {}", fmt_count(properties));
    println!("tags:      {}", fmt_count(tags));
    println!("backlinks: {}", fmt_count(backlinks));
    println!("assets:    {}", fmt_count(assets));
    println!("trashed:   {}", fmt_count(trashed));
    println!("snapshots: {} ({snapshot_size})", fmt_count(snapshot_count));
    if let Some(last_modified) = last_modified {
        println!("modified:  {last_modified}");
    }
    print!("ops:       {} total", fmt_count(ops));
    if ops_since_snapshot != ops {
        println!(
            ", {} since the last snapshot",
            fmt_count(ops_since_snapshot)
        );
    } else {
        println!();
    }
}
