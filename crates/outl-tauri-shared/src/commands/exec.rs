//! `run_code_block` command body — the Tauri side of
//! `outl_actions::exec::run_code_block`.
//!
//! All the real work — walking the outline, resolving the page's `.md`
//! path, calling `outl_exec::run_block_at_index`, building the
//! Serde-friendly outcome — happens in `outl-actions` so every client
//! shares the same flow. This body owns only:
//!
//! - argument parsing (`String` → `NodeId`)
//! - the `AppHost` lookup (workspace + storage_root + hlc + registry)
//! - composing the refreshed [`PageView`] into the wire reply
//!
//! Synchronous — Tauri serves sync commands from its own multi-threaded
//! worker pool, so a long-running runtime doesn't park the JS-side event
//! loop, but it **does** hold the workspace mutex for the whole call.
//! Making this async + `spawn_blocking` to release the mutex earlier is
//! tracked in the desktop polish backlog.

use outl_actions::{
    page_meta as page_meta_action, read_page_outline_with_workspace,
    run_code_block as action_run_code_block, ExecOutputDto, RunCodeBlockOutcome,
};
use outl_exec::{extract_fence, run_block_at_index};
use outl_md::lang;
use serde::Serialize;
use std::collections::HashMap;
use tracing::warn;

use crate::helpers::{build_page_view, parse_node_id, with_ws, with_ws_mut};
use crate::host::AppHost;
use crate::state::PageView;

/// Wire reply for `run_code_block`. Adds the [`PageView`] to the shared
/// [`RunCodeBlockOutcome`] so the frontend re-renders in a single
/// round-trip.
#[derive(Debug, Clone, Serialize)]
pub struct RunCodeBlockReply {
    /// The detected language (`"python"`, `"lisp"`, …).
    pub language: String,
    /// Successful execution payload, or `None` when the runtime bailed
    /// before producing output.
    pub result_ok: Option<ExecOutputDto>,
    /// Infrastructure / runtime-not-found message, when applicable.
    pub error: Option<String>,
    /// Refreshed view of the page so the frontend re-renders without a
    /// follow-up `open_page_by_slug` round-trip — `outl-exec` already
    /// wrote the result subblock and reconciled with the op log.
    pub view: PageView,
}

pub fn run_code_block<S: AppHost>(
    state: &S,
    page_id: String,
    block_id: String,
) -> Result<RunCodeBlockReply, String> {
    let root = state.storage_root()?;
    let page = parse_node_id(&page_id)?;
    let block = parse_node_id(&block_id)?;
    let registry = state.exec_registry();

    with_ws_mut(state, |ws| {
        let RunCodeBlockOutcome {
            language,
            result_ok,
            error,
        } = action_run_code_block(ws, state.hlc(), &root, &registry, page, block)
            .map_err(|e| e.to_string())?;

        if let Some(msg) = error.as_ref() {
            warn!("run_code_block runtime error: {msg}");
        }

        let view = build_page_view(ws, &root, page).map_err(|e| e.to_string())?;
        Ok(RunCodeBlockReply {
            language,
            result_ok,
            error,
            view,
        })
    })
}

/// Wire reply for `run_auto_run_blocks`.
#[derive(Debug, Clone, Serialize)]
pub struct AutoRunReply {
    /// How many blocks were executed.
    pub ran: usize,
    /// Refreshed view after running query blocks.
    pub view: PageView,
}

/// Resolve a batch of block-ref handles (`blk-XXXXXX`) to their source
/// content. One reply serves both surfaces the handle can appear in:
/// `((blk-…))` inline refs render `text`; `!((blk-…))` embeds render
/// `text` plus the `children` subtree, mirroring the TUI.
/// Wire reply for `resolve_embeds`.
#[derive(Debug, Clone, Serialize)]
pub struct EmbedContent {
    pub handle: String,
    pub text: String,
    pub page_slug: String,
    /// `"todo"`, `"doing"`, `"done"`, or `None` when the block is not a task.
    pub status: Option<String>,
    /// The source block's subtree, projected with inline tokens so an
    /// embed surface can expand it exactly as the source page would.
    /// Empty for a leaf block. Ids are transient (see
    /// [`outl_actions::project_parsed_subtree`]) — the subtree is a
    /// read-only, re-resolved-on-navigation view, never a mutation target.
    pub children: Vec<outl_actions::OutlineNode>,
}

/// Sweep the current page for blocks whose fence language has
/// `auto_run() == true` (query blocks) and execute them. Returns the
/// refreshed [`PageView`].
pub fn run_auto_run_blocks<S: AppHost>(state: &S, page_id: String) -> Result<AutoRunReply, String> {
    let root = state.storage_root()?;
    let page = parse_node_id(&page_id)?;
    let registry = state.exec_registry();

    let auto_run_langs: Vec<String> = registry
        .languages()
        .filter(|lang| registry.get(lang).map(|r| r.auto_run()).unwrap_or(false))
        .map(String::from)
        .collect();

    // Does any auto-run runtime actually read `ExecContext::index`?
    // Only `query` does today; the answer gates the derivation below.
    let needs_index = auto_run_langs
        .iter()
        .filter_map(|lang| registry.get(lang))
        .any(|r| r.needs_workspace_index());

    if auto_run_langs.is_empty() {
        let view = with_ws(state, |ws| {
            build_page_view(ws, &root, page).map_err(|e| e.to_string())
        })?;
        return Ok(AutoRunReply { ran: 0, view });
    }

    let ran = with_ws_mut(state, |ws| {
        let meta =
            page_meta_action(ws, page).ok_or_else(|| format!("page not found: {page_id}"))?;
        let outline =
            read_page_outline_with_workspace(&root, &meta, ws).map_err(|e| e.to_string())?;

        let mut targets: Vec<usize> = Vec::new();
        let mut cursor = 0usize;
        collect_auto_run_flat_indices(&outline.nodes, &auto_run_langs, &mut cursor, &mut targets);

        let md_path = outl_actions::journal::page_md_path(&root, &meta);
        let hlc = state.hlc();
        // Derive once for the whole sweep, not per block — this loop
        // runs every auto-run block on the page, and a runtime handed
        // no index builds its own off disk. Skipped entirely when no
        // target's runtime reads one, so a page of `auto-run::` python
        // blocks does not pay for a facility only `query` uses.
        let index =
            (needs_index && !targets.is_empty()).then(|| outl_actions::index::derive(ws, &root));
        let mut count = 0usize;
        for idx in targets {
            if let Err(e) = run_block_at_index(
                ws,
                hlc,
                &md_path,
                idx,
                &registry,
                Some(&root.join(".outl").join("orphans.log")),
                index.as_ref(),
            ) {
                warn!("auto-run block {idx} failed: {e}");
            } else {
                count += 1;
            }
        }
        Ok(count)
    })?;

    let view = with_ws(state, |ws| {
        build_page_view(ws, &root, page).map_err(|e| e.to_string())
    })?;
    Ok(AutoRunReply { ran, view })
}

/// How deep an embed's subtree is projected into the reply. Mirror of
/// the frontend's `EmbeddedSubtree` `MAX_DEPTH` (and the TUI's
/// `EMBED_MAX_DEPTH`): the client never renders past this, so shipping
/// deeper levels is pure IPC weight. Keep the three in step.
const EMBED_SUBTREE_MAX_DEPTH: usize = 4;

/// Batch-resolve embed handles to their source content.
pub fn resolve_embeds<S: AppHost>(
    state: &S,
    handles: Vec<String>,
) -> Result<HashMap<String, EmbedContent>, String> {
    let root = state.storage_root()?;
    let index = outl_md::index::WorkspaceIndex::build(&root);
    let mut result = HashMap::new();
    for handle in &handles {
        if let Some(entry) = index.resolve_block_ref(handle) {
            let (status, text) = split_todo_prefix(&entry.text);
            let mut children = outl_actions::project_parsed_subtree(&entry.children);
            truncate_children_depth(&mut children, EMBED_SUBTREE_MAX_DEPTH);
            result.insert(
                handle.clone(),
                EmbedContent {
                    handle: entry.ref_handle.clone(),
                    text,
                    page_slug: entry.source_slug.clone(),
                    status,
                    children,
                },
            );
        }
    }
    Ok(result)
}

/// Drop subtree levels the client won't render (beyond
/// [`EMBED_SUBTREE_MAX_DEPTH`]). `nodes` is level 1; once `depth_left`
/// hits 1, the current level keeps its text but loses its children.
fn truncate_children_depth(nodes: &mut [outl_actions::OutlineNode], depth_left: usize) {
    if depth_left <= 1 {
        for node in nodes {
            node.children.clear();
        }
        return;
    }
    for node in nodes {
        truncate_children_depth(&mut node.children, depth_left - 1);
    }
}

/// Split `"TODO body"` / `"DOING body"` / `"DONE body"` into
/// `(Some("todo"|"doing"|"done"), "body")`, reusing the canonical
/// `outl_actions::split_todo` so the marker vocabulary stays
/// consistent — a new state reaches the GUI without a change here.
fn split_todo_prefix(raw: &str) -> (Option<String>, String) {
    let (state, body) = outl_actions::split_todo(raw);
    let status = state.map(|s| s.as_str().to_lowercase());
    (status, body.into())
}

fn collect_auto_run_flat_indices(
    blocks: &[outl_actions::OutlineNode],
    auto_run_langs: &[String],
    cursor: &mut usize,
    out: &mut Vec<usize>,
) {
    for b in blocks {
        if let Some(parts) = extract_fence(&b.text) {
            let canonical = lang::canonical(&parts.language).unwrap_or(&parts.language);
            if auto_run_langs.iter().any(|l| l == canonical) {
                out.push(*cursor);
            }
        }
        *cursor += 1;
        collect_auto_run_flat_indices(&b.children, auto_run_langs, cursor, out);
    }
}
