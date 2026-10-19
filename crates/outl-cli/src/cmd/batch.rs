//! `outl batch` — execute a list of write ops sequentially in one
//! workspace session.
//!
//! Designed for the agent / scripting use case where the historical
//! "one tool call per op" surface costs latency and turn budget:
//! creating a structured page with five nested bullets used to mean
//! six MCP round-trips. With `outl_batch` the agent ships a single
//! payload and gets back the ids of every node it touched.
//!
//! Semantics:
//!
//! - Ops apply in array order, sharing one open workspace + one
//!   `HlcGenerator`.
//! - Failure is **stop-on-first-error**. Earlier ops stay in the op
//!   log (they're already CRDT ops; we don't roll them back).
//! - The response always echoes `results` for every applied op, plus
//!   `failed_at` / `failed_op` / `error` when the run stopped early.
//!   That way an agent or script can recover (or retry only the
//!   suffix) instead of guessing what landed.

use std::io::Read;
use std::ops::{Deref, DerefMut};
use std::path::Path;

use clap::Args;
use serde_json::{json, Value};

use outl_actions::BlockTreeSpec;
use outl_core::workspace::WorkspaceError;

use crate::output::{codes, emit, ApiError, EXIT_OK, EXIT_USER};
use crate::ws::{self, WsCtx};

use super::{block as block_cmd, daily as daily_cmd, page as page_cmd, prop as prop_cmd};

/// RAII batch scope over a [`WsCtx`], coalescing every op the wrapped
/// commands apply into a single storage flush.
///
/// `Workspace::begin_batch` borrows the `workspace` field, which
/// collides with the `&mut WsCtx` each per-op command handler takes.
/// This guard sidesteps that: it borrows the whole `WsCtx`, derefs back
/// to it (so the handlers keep their `&mut WsCtx` unchanged), and drives
/// the workspace's manual `enter_batch` / `end_batch` pair. Dropping
/// without `commit` still flushes the ops applied so far — best-effort,
/// matching the op-log invariant that an applied op is never silently
/// lost, only possibly not-yet-durable.
struct CtxBatch<'a> {
    ctx: &'a mut WsCtx,
    committed: bool,
}

impl<'a> CtxBatch<'a> {
    fn new(ctx: &'a mut WsCtx) -> Self {
        ctx.workspace.enter_batch();
        Self {
            ctx,
            committed: false,
        }
    }

    /// Close the batch and flush the buffered ops, propagating any
    /// storage error.
    fn commit(mut self) -> Result<(), WorkspaceError> {
        self.committed = true;
        self.ctx.workspace.end_batch()
    }
}

impl Deref for CtxBatch<'_> {
    type Target = WsCtx;

    fn deref(&self) -> &WsCtx {
        self.ctx
    }
}

impl DerefMut for CtxBatch<'_> {
    fn deref_mut(&mut self) -> &mut WsCtx {
        self.ctx
    }
}

impl Drop for CtxBatch<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        // Best-effort flush of the applied prefix: the ops already live
        // in the CRDT + in-memory log, so a storage error here means
        // "not yet durable", never "lost". It can't propagate out of
        // `Drop`, so it is logged.
        if let Err(e) = self.ctx.workspace.end_batch() {
            tracing::warn!(
                "batch flush on drop failed (ops applied in memory, not persisted): {e}"
            );
        }
    }
}

/// `outl batch` arguments.
#[derive(Args, Debug)]
pub struct BatchArgs {
    /// Ops payload as JSON: `{"ops": [{"op": "...", "args": {...}}]}`.
    /// Pass `-` (or omit) to read from stdin.
    #[arg(long)]
    pub ops: Option<String>,
    /// Force JSON output (default for `batch` since the payload is
    /// the whole point).
    #[arg(long)]
    pub json: bool,
}

/// `outl batch` entry point.
pub fn run(args: &BatchArgs, path: &Path) -> i32 {
    let payload = match read_payload(args.ops.as_deref()) {
        Ok(p) => p,
        Err(e) => return emit::<Value, _>(args.json, Err(e), |_| {}),
    };

    let result = ws::open(path).and_then(|mut ctx| apply_batch(&mut ctx, &payload));

    let stop_exit = matches!(&result, Ok(v) if v.get("failed_at").is_some());
    let code = emit(args.json, result, print_batch);
    if code == EXIT_OK && stop_exit {
        EXIT_USER
    } else {
        code
    }
}

/// Read `--ops` from arg or stdin (when arg is `-` or missing).
fn read_payload(arg: Option<&str>) -> Result<Value, ApiError> {
    let raw = match arg {
        Some(s) if s != "-" => s.to_string(),
        _ => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| ApiError::new(codes::INVALID_ARG, format!("read stdin: {e}")))?;
            buf
        }
    };
    serde_json::from_str(&raw)
        .map_err(|e| ApiError::new(codes::INVALID_ARG, format!("invalid batch JSON: {e}")))
}

/// Workhorse used by both CLI and MCP dispatcher.
pub fn apply_batch(ctx: &mut WsCtx, payload: &Value) -> Result<Value, ApiError> {
    let ops = payload
        .get("ops")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::new(codes::INVALID_ARG, "missing `ops` array".to_string()))?;

    // Run every op under one batch so the N ops share a single storage
    // flush instead of paying a per-op fsync. Each op still flows through
    // the CRDT one at a time (the materialized tree the next op reads
    // stays correct); only the persist is deferred to `commit`. On a
    // stop-on-first-error the guard is dropped *before* the error
    // response is built, flushing the already-applied prefix — so
    // `applied` / `failed_at` describe the exact same on-disk state as
    // the pre-batch per-op path.
    let mut batch = CtxBatch::new(ctx);

    let mut results: Vec<Value> = Vec::with_capacity(ops.len());
    for (idx, entry) in ops.iter().enumerate() {
        let op_name = entry
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ApiError::new(
                    codes::INVALID_ARG,
                    format!("op at index {idx} missing `op` field"),
                )
            })?
            .to_string();
        let default_args = json!({});
        let args = entry.get("args").unwrap_or(&default_args);

        match apply_op(&mut batch, &op_name, args) {
            Ok(data) => results.push(json!({ "op": op_name, "data": data })),
            Err(err) => {
                // Flush the applied prefix before reporting the failure,
                // so the on-disk state matches `applied` / `failed_at`.
                drop(batch);
                return Ok(json!({
                    "results": results,
                    "applied": idx,
                    "failed_at": idx,
                    "failed_op": op_name,
                    "error": { "code": err.code, "message": err.message },
                }));
            }
        }
    }

    batch.commit().map_err(ApiError::internal)?;

    let applied = results.len();
    Ok(json!({
        "results": results,
        "applied": applied,
    }))
}

fn apply_op(ctx: &mut WsCtx, op: &str, args: &Value) -> Result<Value, ApiError> {
    match op {
        "page_create" => {
            let slug = require_str(args, "slug")?;
            let title = opt_str(args, "title");
            let icon = opt_str(args, "icon");
            let content = match args.get("content") {
                None | Some(Value::Null) => None,
                Some(v) => Some(deserialize_forest(v)?),
            };
            page_cmd::create(ctx, slug, title, icon, content.as_deref())
        }
        "page_update" => {
            let slug = require_str(args, "slug")?;
            let title = opt_str(args, "title");
            let icon = opt_str(args, "icon");
            page_cmd::update(ctx, slug, title, icon)
        }
        "page_delete" => {
            let slug = require_str(args, "slug")?;
            require_confirm(args, &format!("page `{slug}`"))?;
            page_cmd::delete(ctx, slug)
        }
        "page_rename" => {
            let old = require_str(args, "old_slug")?;
            let new = require_str(args, "new_slug")?;
            page_cmd::rename(ctx, old, new)
        }
        "block_append" => {
            let page = opt_str(args, "page");
            let parent = opt_str(args, "parent");
            let text = require_str(args, "text")?;
            block_cmd::append(ctx, page, parent, text)
        }
        "block_append_tree" => {
            let page = opt_str(args, "page");
            let parent = opt_str(args, "parent");
            let spec_val = args
                .get("tree")
                .cloned()
                .ok_or_else(|| ApiError::new(codes::INVALID_ARG, "missing `tree`".to_string()))?;
            let spec: BlockTreeSpec = serde_json::from_value(spec_val)
                .map_err(|e| ApiError::new(codes::INVALID_ARG, format!("invalid `tree`: {e}")))?;
            block_cmd::append_tree_h(ctx, page, parent, &spec)
        }
        "block_insert" => {
            let after = require_str(args, "after")?;
            let text = require_str(args, "text")?;
            block_cmd::insert(ctx, after, text)
        }
        "block_update" => {
            let id = require_str(args, "id")?;
            let text = require_str(args, "text")?;
            block_cmd::update(ctx, id, text)
        }
        "block_move" => {
            let id = require_str(args, "id")?;
            let parent = opt_str(args, "parent");
            let after = opt_str(args, "after");
            block_cmd::move_block(ctx, id, parent, after)
        }
        "block_delete" => {
            let id = require_str(args, "id")?;
            require_confirm(args, &format!("block `{id}`"))?;
            block_cmd::delete(ctx, id)
        }
        "block_toggle_todo" => {
            let id = require_str(args, "id")?;
            block_cmd::toggle_todo(ctx, id)
        }
        "daily_append" => {
            let text = require_str(args, "text")?;
            let date = opt_str(args, "date");
            daily_cmd::append(ctx, date, text)
        }
        "page_prop_set" => {
            let page = require_str(args, "page")?;
            let key = require_str(args, "key")?;
            let value = require_str(args, "value")?;
            prop_cmd::set_kv(ctx, page, key, value)
        }
        "page_prop_delete" => {
            let page = require_str(args, "page")?;
            let key = require_str(args, "key")?;
            prop_cmd::delete(ctx, page, key)
        }
        other => Err(ApiError::new(
            codes::INVALID_ARG,
            format!("unknown batch op `{other}`"),
        )),
    }
}

fn deserialize_forest(value: &Value) -> Result<Vec<BlockTreeSpec>, ApiError> {
    serde_json::from_value(value.clone())
        .map_err(|e| ApiError::new(codes::INVALID_ARG, format!("invalid `content`: {e}")))
}

fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ApiError> {
    args.get(key).and_then(Value::as_str).ok_or_else(|| {
        ApiError::new(
            codes::INVALID_ARG,
            format!("missing required string `{key}`"),
        )
    })
}

fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn require_confirm(args: &Value, label: &str) -> Result<(), ApiError> {
    if args
        .get("confirm")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        Ok(())
    } else {
        Err(ApiError::new(
            codes::CONFIRM_REQUIRED,
            format!("refusing to delete {label} without confirm:true"),
        ))
    }
}

fn print_batch(v: &Value) {
    let applied = v.get("applied").and_then(Value::as_u64).unwrap_or(0);
    if let Some(fail_idx) = v.get("failed_at").and_then(Value::as_u64) {
        let op = v.get("failed_op").and_then(Value::as_str).unwrap_or("?");
        let msg = v
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("?");
        eprintln!("batch stopped at op {fail_idx} (`{op}`): {msg}");
        eprintln!("applied {applied} op(s) before failure");
    } else {
        println!("batch applied {applied} op(s)");
    }
}
