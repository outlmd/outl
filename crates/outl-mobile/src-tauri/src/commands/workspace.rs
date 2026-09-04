//! Workspace lifecycle commands: reload + stats.

use outl_actions::open_today;
use tauri::State;

use crate::state::{AppState, WorkspaceSummary};

#[tauri::command]
pub(crate) fn workspace_stats(state: State<'_, AppState>) -> WorkspaceSummary {
    let guard = state.workspace.lock();
    let storage_root = state.storage_root.to_string_lossy().into_owned();
    match guard.as_ref() {
        Some(ws) => WorkspaceSummary {
            blocks: ws.tree().node_count(),
            ops: ws.log().len(),
            actor: ws.actor.to_string(),
            storage_root,
            ready: true,
        },
        None => WorkspaceSummary {
            blocks: 0,
            ops: 0,
            actor: String::new(),
            storage_root,
            ready: false,
        },
    }
}

#[tauri::command]
pub(crate) async fn reload_workspace(state: State<'_, AppState>) -> Result<(), String> {
    // A reload replays the WHOLE op log (`Workspace::open_with_storage`) —
    // O(all ops), which on a freshly-synced workspace is 200k+ ops. This is
    // CPU-bound and runs for seconds. A synchronous `#[tauri::command]`
    // executes on the Tauri IPC/main worker, so doing the replay inline
    // holds that thread through the whole rebuild and iOS fires the
    // scene-update watchdog (>10s → SIGKILL) — the "app freezes forever
    // after pairing" bug. Offload the replay to a blocking pool thread
    // (mirrors the background boot opener, which is why boot never trips
    // the watchdog while this path did) so the WebView keeps painting.
    let storage_root = state.storage_root.clone();
    let hlc = state.hlc.clone();
    let workspace = state.workspace.clone();
    let backlink_index = state.backlink_index.clone();
    let fresh = tauri::async_runtime::spawn_blocking(
        move || -> Result<outl_core::workspace::Workspace, String> {
            let engine = outl_actions::SyncEngine::new(storage_root, hlc.actor());
            let mut fresh = engine
                .reload_workspace()
                .map_err(|e| format!("reload workspace: {e}"))?;
            // NOTE: orphan-`.md` reconcile is a BOOT/recovery concern (it runs
            // md → ops and desync recovery, both of which MUTATE the op log). It
            // used to run here inline on every 3s poll, which — on a page being
            // edited concurrently on two devices while sync ingests peer ops —
            // turned the routine reload into a projection↔op-log feedback loop
            // and made the page flip-flop between the two devices' states. iroh
            // peers ship OPS (not `.md`), so a routine reload only needs to
            // re-materialize the op log; orphan `.md` recovery already runs once
            // at boot (`workspace_open`). Keep the reload a pure re-read.
            // Resolve today's journal *in the fresh workspace* so the page
            // id reflects the merged op log. `open_today` is idempotent —
            // when the page already exists it just returns the id; when it
            // doesn't, it creates one with the deterministic slug-derived
            // id, which both peers will agree on.
            let today_id = open_today(&mut fresh, &hlc).map_err(|e| e.to_string())?;
            // Guarded (root `CLAUDE.md` invariant 8) — can refuse when
            // today's `.md` holds content the merge never saw. Not
            // propagated with `?`: that would abort the reload before
            // `fresh` (which already holds every peer's merged ops) gets
            // swapped in below, turning one page's refusal into every
            // page failing to converge. Frontend's `pullAndReload` always
            // re-opens the current page right after this command returns
            // (`open_today_journal` / `open_journal_for` /
            // `open_page_by_slug`), and that open independently re-runs
            // the equivalent guarded check and sets `PageView.md_ahead_of_log`
            // — so the refusal still reaches the banner, just one
            // round-trip later rather than from this call directly.
            if let Err(e) = engine.reproject_page(&fresh, today_id) {
                tracing::warn!("reload_workspace: today's page stopped syncing: {e}");
            }
            Ok(fresh)
        },
    )
    .await
    .map_err(|e| format!("reload task join: {e}"))??;
    // Surgical undo invalidation (mirrors the desktop's `reload_workspace`):
    // only pages whose projection actually changed across the reload lose
    // their stacks. Since mobile's `AppHost::history()` started recording
    // snapshots (RFC 0254 phase 1), a blanket keep here would let `undo_page`
    // restore a pre-reload `.md` over a page a peer just changed — silently
    // reverting their edit.
    {
        let old_guard = workspace.lock();
        let mut history = state.history.lock();
        outl_tauri_shared::helpers::invalidate_changed_history(
            old_guard.as_ref(),
            &fresh,
            &mut history,
        );
    }
    *workspace.lock() = Some(fresh);
    // Peer ops replaced the workspace, so the cached backlinks index
    // is stale — drop it; the next `page_backlinks` rebuilds it.
    *backlink_index.lock() = None;
    Ok(())
}
