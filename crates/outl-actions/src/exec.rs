//! Shared "run a fenced code block" glue.
//!
//! Every outl client that lets the user execute a code block (`outl-tui`'s
//! `g x` chord, `outl-desktop`'s `Cmd+X` / Run button, `outl-mobile`'s
//! long-press → "Run code") needs the same orchestration:
//!
//! 1. Resolve the block's flat-DFS index inside the page outline.
//! 2. Resolve the page's `.md` path on disk (journals vs. pages folder).
//! 3. Call [`outl_exec::run_block_at_index`] which executes, persists
//!    the `> **result:**` sibling subblock, and reconciles with the op
//!    log.
//! 4. Surface the language, the runtime payload (when it ran), and the
//!    infrastructure error (when it didn't) as a Serde-friendly DTO so
//!    each client adds a refreshed [`crate::OutlineNode`]/`PageView`-
//!    shaped layer on top.
//!
//! This module owns steps 1–4. Clients own only the AppState lookup
//! and the `view` field of the response so the per-client `PageView`
//! type stays in the client.
//!
//! Why it sits in `outl-actions` (and not in each client's
//! `commands::exec`): the mobile and desktop shims used to be
//! bit-for-bit copies of the same flow, and the path-resolution code
//! was also reinventing [`crate::page_md_path`] — exactly the kind of
//! drift the workspace-level "Reuse-first" policy in the root
//! `CLAUDE.md` exists to prevent.

use std::path::Path;

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_exec::{run_block_at_index, ExecOutput, RuntimeRegistry};
use serde::Serialize;

use crate::error::ActionError;
use crate::journal::page_md_path;
use crate::outline::{flat_index_for_block, project_outline};
use crate::page::page_meta;

/// Serializable mirror of [`outl_exec::ExecOutput`].
///
/// `Duration` doesn't serialise cleanly to JSON, so we flatten it to
/// milliseconds; `ExitStatus` is rendered via `Debug` for forward-compat
/// (`"Ok"`, `"NonZero(1)"`, `"Trap(\"…\")"`).
#[derive(Debug, Clone, Serialize)]
pub struct ExecOutputDto {
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
    /// Wall-clock runtime of the runtime call, in milliseconds.
    pub duration_ms: u128,
    /// Stringified Rust `ExitStatus`.
    pub exit: String,
}

impl From<&ExecOutput> for ExecOutputDto {
    fn from(o: &ExecOutput) -> Self {
        Self {
            stdout: o.stdout.clone(),
            stderr: o.stderr.clone(),
            duration_ms: o.duration.as_millis(),
            exit: format!("{:?}", o.exit),
        }
    }
}

/// Outcome of [`run_code_block`], without the per-client `view`
/// payload. Clients wrap this with their refreshed `PageView` before
/// shipping it down the Tauri bridge.
///
/// `result_ok` and `error` are mutually exclusive — the runtime
/// either ran (and produced output) or never started (unknown
/// language, timeout, sandbox crash).
#[derive(Debug, Clone, Serialize)]
pub struct RunCodeBlockOutcome {
    /// Detected fence language (`"python"`, `"lisp"`, …).
    pub language: String,
    /// Successful execution payload, or `None` when the runtime
    /// bailed before producing output.
    pub result_ok: Option<ExecOutputDto>,
    /// Infrastructure / runtime-not-found message, when applicable.
    pub error: Option<String>,
}

/// Run the fenced code block at `block_id` inside `page_id`.
///
/// Resolves the block's flat-DFS index, the page's `.md` path, then
/// hands control to [`outl_exec::run_block_at_index`]. The result
/// subblock is persisted by `outl-exec` before this returns, so the
/// caller just needs to re-project the page (`read_page_view*`,
/// `build_page_view`) to surface the change in the UI.
///
/// Errors:
///
/// - [`ActionError::NotInTree`] when `page_id` doesn't resolve to a
///   page node, or when `block_id` isn't part of the projected
///   outline (foreign page, stale call).
/// - [`ActionError::Exec`] wrapping a `RunError` from `outl-exec`
///   when the orchestration itself fails (sidecar IO, op log apply,
///   `.md` reconcile). Runtime-level failures (`unknown language`,
///   timeout) come back through the `error` field of the outcome,
///   not as an `Err` — they are user-visible diagnostics, not bugs.
pub fn run_code_block(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    storage_root: &Path,
    registry: &RuntimeRegistry,
    page_id: NodeId,
    block_id: NodeId,
) -> Result<RunCodeBlockOutcome, ActionError> {
    let meta =
        page_meta(workspace, page_id).ok_or_else(|| ActionError::NotInTree(page_id.to_string()))?;

    // Callable template: a `call:<name>` fence resolves and runs the
    // named template, writing the result under this block. Handled here
    // (not in `outl-exec`) because template resolution needs the
    // workspace — so every client that calls `run_code_block` (desktop,
    // mobile) gets `call:` for free, matching the TUI's `gx` path.
    let block_text = workspace.block_text(block_id).unwrap_or_default();
    if let Some((name, params)) = crate::template::parse_call_invocation(&block_text) {
        let (result_ok, error) = match crate::template::run_callable_block(
            workspace, hlc, registry, &name, &params, block_id,
        ) {
            Ok(out) => (Some(ExecOutputDto::from(&out)), None),
            Err(e) => (None, Some(e.to_string())),
        };
        // `run_callable_block` mutates the op log; project the `.md` so
        // the on-disk page matches (the normal path's `run_block_at_index`
        // does this itself).
        let _ = crate::journal::apply_page_md_with_sidecar(workspace, storage_root, page_id);
        return Ok(RunCodeBlockOutcome {
            language: format!("call:{name}"),
            result_ok,
            error,
        });
    }

    let outline = project_outline(workspace, page_id);
    let flat_idx = flat_index_for_block(&outline, block_id)
        .ok_or_else(|| ActionError::NotInTree(block_id.to_string()))?;

    let md_path = page_md_path(storage_root, &meta);

    // Derive an index only for a runtime that reads one. `query` does;
    // `python` / `lisp` / `js` do not, and deriving over a large
    // workspace costs enough that making every fence pay for it would
    // be a regression. Without this the runtime would build its own off
    // disk anyway, which is strictly worse — hence deriving rather than
    // passing `None`.
    let index = outl_exec::extract_fence(&block_text)
        .and_then(|parts| registry.get(&parts.language))
        .is_some_and(|rt| rt.needs_workspace_index())
        .then(|| crate::index::derive(workspace, storage_root));
    let report = run_block_at_index(
        workspace,
        hlc,
        &md_path,
        flat_idx,
        registry,
        None,
        index.as_ref(),
    )
    .map_err(|e| ActionError::Exec(e.to_string()))?;

    let (result_ok, error) = match &report.result {
        Ok(out) => (Some(ExecOutputDto::from(out)), None),
        Err(e) => (None, Some(format!("{e}"))),
    };

    Ok(RunCodeBlockOutcome {
        language: report.language.clone(),
        result_ok,
        error,
    })
}
