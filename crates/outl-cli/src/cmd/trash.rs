//! `outl trash list` / `outl trash restore` — reading deletions back.
//!
//! Invariant 6 makes delete a `Move(node, TRASH_ROOT)`, so nothing is
//! ever physically removed. Until this command existed, that bought the
//! user nothing they could act on: the doctor could count the trash and
//! no surface could touch it (issue #287).
//!
//! `outl_actions::trash` owns what an entry *is* and whether it can be
//! restored; this module owns only how it reads and the JSON envelope.
//!
//! **There is no `trash empty`.** It is the one operation here that
//! actually destroys, and it belongs with op-log compaction
//! ([#110](https://github.com/outlmd/outl/issues/110)) rather than as an
//! `rm`. Retention today is "forever, until that lands" — a policy, and
//! `docs/cli.md` states it rather than leaving the user to infer it.

use std::path::Path;

use clap::Subcommand;
use serde_json::{json, Value};

use outl_actions::trash;
use outl_actions::ActionError;

use crate::cmd::block::parse_id;
use crate::output::{codes, emit, ApiError};
use crate::ws::{self, WsCtx};

/// `outl trash …` subcommands.
#[derive(Subcommand, Debug)]
pub enum TrashCommand {
    /// List every deletion still in the trash.
    List {
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Put a deleted block back where it was deleted from.
    Restore {
        /// Block id (full ULID), as `outl trash list` prints it.
        id: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
}

/// Run an `outl trash …` invocation.
pub fn run(cmd: &TrashCommand, path: &Path) -> i32 {
    match cmd {
        TrashCommand::List { json } => {
            let result = ws::open(path).and_then(|ctx| list(&ctx));
            emit(*json, result, print_list)
        }
        TrashCommand::Restore { id, json } => {
            let result = ws::open(path).and_then(|mut ctx| restore(&mut ctx, id));
            emit(*json, result, print_restored)
        }
    }
}

/// Every top-level deletion, with whether it can be restored.
pub fn list(ctx: &WsCtx) -> Result<Value, ApiError> {
    let entries: Vec<Value> = trash::list(&ctx.workspace)
        .into_iter()
        .map(|entry| {
            json!({
                "id": entry.node.to_string(),
                "preview": entry.preview,
                "blocks": entry.subtree_len,
                "page": entry.page,
                "parent": entry.parent_at_deletion.map(|p| p.to_string()),
                // The verdict comes from `outl_actions::trash::refusal_for`,
                // never recomputed here — a listing that decided this on
                // its own would offer a restore that then fails.
                "restorable": entry.refusal.is_none(),
                "refusal": entry.refusal,
            })
        })
        .collect();

    Ok(json!({ "entries": entries }))
}

/// Restore one block.
pub fn restore(ctx: &mut WsCtx, id_str: &str) -> Result<Value, ApiError> {
    let id = parse_id(id_str)?;
    let entry = trash::restore(&mut ctx.workspace, &ctx.hlc, id).map_err(refusal_error)?;

    // The page the block landed back on has to be re-projected, or the
    // `.md` keeps saying the block is gone.
    if let Some(page) = outl_actions::enclosing_page_id(&ctx.workspace, id) {
        ctx.commit(page)?;
    }

    Ok(json!({
        "id": entry.node.to_string(),
        "preview": entry.preview,
        "blocks": entry.subtree_len,
        "parent": entry.parent_at_deletion.map(|p| p.to_string()),
    }))
}

/// Map a refusal onto a stable code, so an agent can tell them apart
/// without parsing prose. The message stays the one
/// `outl_actions::error` wrote — a second wording here would be a
/// second owner of the explanation.
///
/// Deliberately no count in this sentence: it said "the three" while
/// the match had four arms, and `docs/cli.md` said "Three refusals"
/// over a four-row table. A number written beside a list is a second
/// copy of the list's length.
fn refusal_error(err: ActionError) -> ApiError {
    let code = match &err {
        ActionError::NotTrashed(_) => codes::NOT_TRASHED,
        ActionError::TrashPageRestoreUnsupported { .. } => codes::TRASH_PAGE_UNSUPPORTED,
        ActionError::TrashParentTrashed { .. } => codes::TRASH_PARENT_TRASHED,
        ActionError::TrashParentMissing { .. } => codes::TRASH_PARENT_MISSING,
        ActionError::TrashOriginUnknown { .. } => codes::TRASH_ORIGIN_UNKNOWN,
        _ => return ApiError::internal(err),
    };
    ApiError::new(code, err.to_string())
}

fn print_list(value: &Value) {
    let entries = value["entries"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if entries.is_empty() {
        println!("trash is empty — nothing has been deleted in this workspace");
        return;
    }

    println!("{} deletion(s) in the trash", entries.len());
    for entry in entries {
        let id = entry["id"].as_str().unwrap_or("?");
        let preview = entry["preview"].as_str().unwrap_or("");
        let blocks = entry["blocks"].as_u64().unwrap_or(1);
        let carried = if blocks > 1 {
            format!(" (+{} block(s) under it)", blocks - 1)
        } else {
            String::new()
        };
        match entry["page"].as_str() {
            Some(slug) => println!("  {id}  page `{slug}`{carried}"),
            None => println!("  {id}  {preview}{carried}"),
        }
        // Printed verbatim: `ActionError::Display` owns the whole
        // sentence, "cannot restore" included. Prefixing it here stated
        // it twice (`the_human_listing_states_each_refusal_once`).
        if let Some(refusal) = entry["refusal"].as_str() {
            println!("      {refusal}");
        }
    }
    println!();
    println!("restore one with `outl trash restore <id>`");
}

fn print_restored(value: &Value) {
    let id = value["id"].as_str().unwrap_or("?");
    let preview = value["preview"].as_str().unwrap_or("");
    println!("restored {id}: {preview}");
}
