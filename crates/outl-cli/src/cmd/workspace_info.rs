//! `outl workspace info` — workspace summary (path, actor, counts).
//!
//! **`ops` counts the resident log, which is not always the whole log.**
//! A workspace that booted from a snapshot replays only the delta above
//! the cutoff, so `Workspace::log()` holds that delta and nothing below
//! it. Printing it as `ops: 332` for a workspace whose `ops/` holds
//! 228,943 is not a rounding error — it is three orders of magnitude,
//! on the one command a user runs to ask how big their graph is, and it
//! reads as data loss.
//!
//! `outl_actions::activity` already wrote the rule this broke: a partial
//! count is never presented as a total, it is labelled "since the last
//! snapshot". `ops_complete` carries that verdict here, from
//! `Workspace::booted_from_snapshot`, so the human line and the JSON say
//! the same thing.
//!
//! It stays a resident count rather than becoming a true one on purpose.
//! The real total needs `Storage::all_ops()` — the doctor pays that
//! (a full parse of every `.jsonl`) because integrity is its job, and
//! making a summary command parse 134 MB to print one number is a cost
//! this command has no reason to pay. `outl doctor` is where the exact
//! figure lives, and the label now points at the difference instead of
//! hiding it.

use std::path::Path;

use clap::Args;
use serde_json::{json, Value};

use outl_actions::{list_pages, PageKind};

use crate::output::emit;
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
    let result = ws::open_read(path).map(|ctx| info(&ctx));
    emit(args.json, result, print_info)
}

/// Pure handler for the MCP shim.
pub fn info(ctx: &WsCtx) -> Value {
    let pages = list_pages(&ctx.workspace);
    let mut journals = 0usize;
    let mut regular = 0usize;
    for p in &pages {
        match p.kind {
            PageKind::Journal => journals += 1,
            PageKind::Page => regular += 1,
        }
    }
    json!({
        "root": ctx.root.display().to_string(),
        "actor": ctx.actor.to_string(),
        "ephemeral_actor": ctx.ephemeral_actor,
        "ops_dir": ctx.paths.ops.display().to_string(),
        "pages_dir": ctx.paths.pages.display().to_string(),
        "journals_dir": ctx.paths.journals.display().to_string(),
        "pages": regular,
        "journals": journals,
        "ops": ctx.workspace.log().len(),
        // False means `ops` covers only what was replayed above the
        // snapshot cutoff. See this module's doc.
        "ops_complete": !ctx.workspace.booted_from_snapshot(),
        "tree_nodes": ctx.workspace.tree().node_count(),
    })
}

fn print_info(v: &Value) {
    let root = v.get("root").and_then(Value::as_str).unwrap_or("?");
    let actor = v.get("actor").and_then(Value::as_str).unwrap_or("?");
    let pages = v.get("pages").and_then(Value::as_u64).unwrap_or(0);
    let journals = v.get("journals").and_then(Value::as_u64).unwrap_or(0);
    let ops = v.get("ops").and_then(Value::as_u64).unwrap_or(0);
    // Absent reads as complete: an older payload had no such field, and
    // a missing flag must not turn a full count into a hedge.
    let ops_complete = v
        .get("ops_complete")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let ephemeral = v
        .get("ephemeral_actor")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    println!("root:     {root}");
    if ephemeral {
        println!("actor:    {actor} (ephemeral — another outl owns the config actor)");
    } else {
        println!("actor:    {actor}");
    }
    println!("pages:    {pages}");
    println!("journals: {journals}");
    if ops_complete {
        println!("ops:      {ops}");
    } else {
        println!("ops:      {ops} since the last snapshot — `outl doctor` counts the whole log");
    }
}
