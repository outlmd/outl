//! Undo / redo commands — thin wrappers over
//! `outl_tauri_shared::commands::history`. The body lives in the shared
//! crate (RFC 0254 phase 1) so mobile registers the same two commands
//! instead of `AppHost::history()`'s `None` default silently skipping
//! snapshot recording forever.

use tauri::State;

use crate::state::{AppState, PageView};

/// Revert the last committed mutation on `page_id`. Errors with
/// `"nothing to undo"` when the stack is empty so the frontend can
/// surface it as a status message.
#[tauri::command]
pub(crate) fn undo_page(page_id: String, state: State<'_, AppState>) -> Result<PageView, String> {
    outl_tauri_shared::commands::history::undo_page(state.inner(), page_id)
}

/// Re-apply the mutation the last `undo_page` reverted.
#[tauri::command]
pub(crate) fn redo_page(page_id: String, state: State<'_, AppState>) -> Result<PageView, String> {
    outl_tauri_shared::commands::history::redo_page(state.inner(), page_id)
}

#[cfg(test)]
mod tests {
    //! Written **before** the command body moved to `outl-tauri-shared`
    //! (RFC 0254 phase 1) against the desktop's then-local `step_history`.
    //! `outl_actions::history` already pins the underlying `HistoryStacks`
    //! and `restore_page_md` semantics, but nothing tested that this
    //! crate's `AppState` wiring recorded through the same path a real
    //! `edit_block` commit uses, keyed its stacks by page, or returned
    //! the exact error strings the frontend matches on. Re-pointed at the
    //! moved `outl_tauri_shared::commands::history` functions post-move
    //! with the same assertions — still green is the proof the move
    //! didn't change desktop behaviour.
    use std::collections::HashMap;
    use std::sync::Arc;

    use outl_actions::{
        append_block, open_or_create_page as open_or_create, render_page_md, PageKind,
    };
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::{ActorId, NodeId};
    use outl_core::workspace::Workspace;
    use outl_exec::RuntimeRegistry;
    use outl_tauri_shared::commands::block::edit_block as shared_edit_block;
    use outl_tauri_shared::commands::history::{redo_page, undo_page};
    use parking_lot::Mutex;
    use tempfile::TempDir;

    use crate::settings::Settings;
    use crate::state::AppState;

    /// A real `AppState` wired the same way `lib.rs::run`'s `setup` wires
    /// one — an in-memory workspace with a single page/block — so the
    /// undo / redo commands run against the exact object graph a
    /// production command does. The returned `TempDir` must stay alive
    /// for as long as `AppState` (the storage root points into it, and
    /// the projection writer's background thread holds it too).
    fn test_state() -> (TempDir, AppState, NodeId, String) {
        let tmp = TempDir::new().expect("tempdir");
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).expect("open_in_memory");
        let page = open_or_create(&mut ws, &hlc, "ideas", "Ideas", PageKind::Page)
            .expect("open_or_create");
        let block = append_block(&mut ws, &hlc, Some(page), Some("one")).expect("append_block");

        let workspace = Arc::new(Mutex::new(Some(ws)));
        let storage_root = Arc::new(Mutex::new(Some(tmp.path().to_path_buf())));
        let projection_writer =
            outl_tauri_shared::ProjectionWriter::spawn(workspace.clone(), storage_root.clone());

        let state = AppState {
            workspace,
            storage_root,
            hlc,
            settings: Arc::new(Mutex::new(Settings::default())),
            app_config_dir: tmp.path().to_path_buf(),
            registry: Arc::new(RuntimeRegistry::with_builtins()),
            fs_watcher: Arc::new(Mutex::new(None)),
            iroh_transport: Arc::new(Mutex::new(None)),
            iroh_pairing: Arc::new(Mutex::new(None)),
            history: Mutex::new(HashMap::new()),
            backlink_index: Arc::new(Mutex::new(None)),
            projection_writer,
        };
        (tmp, state, page, block.to_string())
    }

    fn page_text(state: &AppState, page: NodeId) -> String {
        let ws = state.workspace.lock();
        render_page_md(ws.as_ref().expect("workspace must be open"), page)
    }

    #[test]
    fn undo_page_restores_the_previous_snapshot() {
        let (_tmp, state, page, block_id) = test_state();
        let page_id = page.to_string();
        assert_eq!(page_text(&state, page), "title:: Ideas\n\n- one\n");

        shared_edit_block(&state, page_id.clone(), block_id, "two".into()).expect("edit_block");
        assert_eq!(page_text(&state, page), "title:: Ideas\n\n- two\n");

        // No explicit flush: `undo_page` itself must drain the queued
        // background projection write before reconciling against the
        // sidecar (see `step_history`'s flush call). Without that,
        // reconcile can find no sidecar to match "two" against and
        // create a second block for "one" instead of replacing it.
        let view = undo_page(&state, page_id).expect("undo_page");
        assert_eq!(page_text(&state, page), "title:: Ideas\n\n- one\n");
        assert_eq!(view.outline.len(), 1);
        assert_eq!(view.outline[0].text, "one");
    }

    #[test]
    fn undo_page_errors_with_nothing_to_undo_when_the_stack_is_empty() {
        let (_tmp, state, page, _block_id) = test_state();
        let err = undo_page(&state, page.to_string()).unwrap_err();
        assert_eq!(err, "nothing to undo");
    }

    #[test]
    fn redo_page_errors_with_nothing_to_redo_when_the_stack_is_empty() {
        let (_tmp, state, page, _block_id) = test_state();
        let err = redo_page(&state, page.to_string()).unwrap_err();
        assert_eq!(err, "nothing to redo");
    }

    #[test]
    fn redo_page_replays_the_mutation_undo_reverted() {
        let (_tmp, state, page, block_id) = test_state();
        let page_id = page.to_string();
        shared_edit_block(&state, page_id.clone(), block_id, "two".into()).expect("edit_block");

        // No explicit flush here either — see `undo_page_restores_the_previous_snapshot`.
        undo_page(&state, page_id.clone()).expect("undo_page");
        assert_eq!(page_text(&state, page), "title:: Ideas\n\n- one\n");

        redo_page(&state, page_id).expect("redo_page");
        assert_eq!(page_text(&state, page), "title:: Ideas\n\n- two\n");
    }

    #[test]
    fn a_new_edit_after_undo_clears_the_redo_stack() {
        // Vim semantics: a fresh committed mutation branches history, so
        // the "two" undo reverted must not come back on redo.
        let (_tmp, state, page, block_id) = test_state();
        let page_id = page.to_string();
        shared_edit_block(&state, page_id.clone(), block_id.clone(), "two".into())
            .expect("edit_block");
        undo_page(&state, page_id.clone()).expect("undo_page");

        shared_edit_block(&state, page_id.clone(), block_id, "three".into()).expect("edit_block");
        let err = redo_page(&state, page_id).unwrap_err();
        assert_eq!(err, "nothing to redo");
    }

    #[test]
    fn history_is_keyed_per_page_not_shared_globally() {
        let (_tmp, state, page_a, block_a) = test_state();
        let hlc = state.hlc.clone();
        let page_b = {
            let mut guard = state.workspace.lock();
            let ws = guard.as_mut().expect("workspace open");
            open_or_create(ws, &hlc, "other", "Other", PageKind::Page).expect("open_or_create")
        };
        let page_a_id = page_a.to_string();
        let page_b_id = page_b.to_string();

        shared_edit_block(&state, page_a_id.clone(), block_a, "changed".into())
            .expect("edit_block on page A");

        // Page A has a mutation to undo; page B was never touched.
        assert!(undo_page(&state, page_a_id).is_ok());
        let err = undo_page(&state, page_b_id).unwrap_err();
        assert_eq!(err, "nothing to undo");
    }
}
