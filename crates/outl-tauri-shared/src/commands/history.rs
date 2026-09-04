//! Undo / redo command bodies.
//!
//! Thin adapters over `outl_actions::history` — the stacks live in
//! whatever [`AppHost::history`] slot the client wires (desktop's
//! `AppState::history`, one `HistoryStacks` per page; mobile's, as of
//! RFC 0254 phase 1), snapshots are the page's rendered `.md`, and the
//! restore routes through `outl_actions::restore_page_md` so every
//! undo / redo is new ops in the log — never a rewrite of it.
//! `helpers::finish_in_page_with` is the recording side of this pair.
//!
//! Moved here from `outl-desktop/src-tauri/src/commands/history.rs`
//! (RFC 0254 phase 1): the body only ever depended on [`AppHost`], so a
//! client-only home was the same defect RFC 0022 fixed for themes —
//! mobile's `AppHost::history()` returned `None` by default and had no
//! command that could ever return anything else.

use outl_actions::render_page_md;

use crate::helpers::{build_page_view, parse_node_id, storage_root_or_err, with_ws_mut};
use crate::host::AppHost;
use crate::state::PageView;

enum Direction {
    Undo,
    Redo,
}

/// Revert the last committed mutation on `page_id`. Errors with
/// `"nothing to undo"` when the stack is empty so the frontend can
/// surface it as a status message.
pub fn undo_page<S: AppHost>(state: &S, page_id: String) -> Result<PageView, String> {
    step_history(state, &page_id, Direction::Undo)
}

/// Re-apply the mutation the last `undo_page` reverted.
pub fn redo_page<S: AppHost>(state: &S, page_id: String) -> Result<PageView, String> {
    step_history(state, &page_id, Direction::Redo)
}

fn step_history<S: AppHost>(
    state: &S,
    page_id: &str,
    direction: Direction,
) -> Result<PageView, String> {
    let root = storage_root_or_err(state)?;
    let page = parse_node_id(page_id)?;
    // A host that hasn't wired a history slot (`AppHost::history`'s
    // default `None`) has no stacks to step — surface that plainly
    // instead of the caller ever seeing a panic.
    let history = state
        .history()
        .ok_or_else(|| "undo is not supported on this client".to_string())?;
    // `restore_page_md` below writes the snapshot straight to the page's
    // `.md` and reconciles it against whatever sidecar is *currently on
    // disk*. `finish_in_page_with` only ever queues that sidecar's write
    // when a `ProjectionWriter` is wired (the async-writes default on
    // both GUI clients) — it does not wait for it. Undo right after an
    // edit, with the queue not yet drained, means `reconcile_md` matches
    // against a stale (or, for a page never projected before, entirely
    // absent) sidecar: it can't find the block the snapshot's content
    // used to be, so it creates a *second* block instead of replacing
    // the first — a real duplicate-content bug, not a test artifact
    // (root `CLAUDE.md` invariant 8's family: something wrote without
    // checking what the other side had done). Flushing first makes
    // undo/redo always reconcile against a sidecar that reflects every
    // edit that happened before it.
    //
    // Must run BEFORE `with_ws_mut` takes the workspace lock below: the
    // background worker needs that same lock to drain a page write
    // queued ahead of this flush, so calling `flush()` while already
    // holding the lock would deadlock the two against each other.
    if let Some(writer) = state.projection_writer() {
        writer.flush()?;
    }
    with_ws_mut(state, |ws| {
        let current = render_page_md(ws, page);
        let restored = {
            let mut map = history.lock();
            let stacks = map.entry(page).or_default();
            let restore = |snapshot: &String| {
                outl_actions::restore_page_md(ws, state.hlc(), &root, page, snapshot)
            };
            match direction {
                Direction::Undo => stacks.try_undo(current, restore),
                Direction::Redo => stacks.try_redo(current, restore),
            }
        }
        .ok_or_else(|| {
            match direction {
                Direction::Undo => "nothing to undo",
                Direction::Redo => "nothing to redo",
            }
            .to_string()
        })?;
        restored.map_err(|e| e.to_string())?;
        build_page_view(ws, &root, page).map_err(|e| e.to_string())
    })
}
