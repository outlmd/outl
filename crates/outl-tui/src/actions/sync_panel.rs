//! The sync panel: what each paired device has not received yet, and the
//! concurrent edits Yrs merged, with a way back. RFC 0362, "Trust you can
//! see"; the terminal's counterpart to the desktop Trust section and the
//! mobile Devices sheet.
//!
//! Both halves read the same shared state the GUI clients read:
//! `outl_sync_iroh::pending_by_peer` for the per-device floors and
//! `outl_actions::merges` for the log, both persisted under
//! `.outl/state/`. Undo is an ordinary `edit_text` back to this device's
//! pre-merge text, projected to the page's `.md`, so it syncs and can be
//! stepped back like any edit.

use outl_actions::merges::{self, MergedEditDto};

use crossterm::event::{KeyCode, KeyEvent};

use crate::actions::node_id;
use crate::state::{App, Overlay, ToastKind};

/// What the panel shows. Rows are the merged edits; the pending list is
/// a header above them.
#[derive(Debug, Default)]
pub(crate) struct SyncPanelState {
    /// `(device id, ops it has not received)`, most behind first.
    pub(crate) pending: Vec<(String, u64)>,
    /// Merged edits, newest first.
    pub(crate) merged: Vec<MergedEditDto>,
    /// Highlighted merged edit.
    pub(crate) selected: usize,
}

impl SyncPanelState {
    /// Read both halves for `root` as this device (`actor`).
    pub(crate) fn load(root: &std::path::Path, actor: outl_core::id::ActorId) -> Self {
        let mut pending: Vec<_> = outl_sync_iroh::pending_by_peer(root, actor)
            .map(|m| m.into_iter().collect())
            .unwrap_or_default();
        pending.sort_by(|a: &(String, u64), b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Self {
            pending,
            merged: merges::list(root, actor),
            selected: 0,
        }
    }

    /// Clamped cursor move.
    pub(crate) fn step(&mut self, delta: i32) {
        let last = self.merged.len().saturating_sub(1) as i32;
        self.selected = (self.selected as i32 + delta).clamp(0, last) as usize;
    }
}

impl App {
    /// `:sync` — open the panel.
    pub(crate) fn open_sync_panel(&mut self) {
        let state = SyncPanelState::load(&self.workspace_root, self.hlc.actor());
        self.overlay = Some(Overlay::SyncPanel(state));
    }

    pub(crate) fn move_sync_cursor(&mut self, delta: i32) {
        if let Some(Overlay::SyncPanel(p)) = self.overlay.as_mut() {
            p.step(delta);
        }
    }

    fn selected_merge(&self) -> Option<MergedEditDto> {
        match &self.overlay {
            Some(Overlay::SyncPanel(p)) => p.merged.get(p.selected).cloned(),
            _ => None,
        }
    }

    /// `Enter` — jump to the merged block's page.
    pub(crate) fn open_selected_merge(&mut self) {
        let Some(entry) = self.selected_merge() else {
            return;
        };
        let Some(slug) = page_slug(&self.workspace, &entry.page_id) else {
            self.toast(ToastKind::Warning, "that page is gone");
            return;
        };
        self.overlay = None;
        if let Err(e) = self.open_page_by_slug(&slug) {
            self.toast(ToastKind::Error, format!("could not open {slug}: {e}"));
        }
    }

    /// `u` — put this device's pre-merge text back and drop the entry.
    pub(crate) fn undo_selected_merge(&mut self) {
        let Some(entry) = self.selected_merge() else {
            return;
        };
        match self.restore_merged_text(&entry) {
            Ok(()) => {
                let actor = self.hlc.actor();
                merges::take(&self.workspace_root, actor, &entry.id);
                self.toast(ToastKind::Info, "merge undone; your text is back");
            }
            Err(e) => self.toast(ToastKind::Error, format!("could not undo: {e}")),
        }
        let keep = match &self.overlay {
            Some(Overlay::SyncPanel(p)) => p.selected,
            _ => 0,
        };
        let mut state = SyncPanelState::load(&self.workspace_root, self.hlc.actor());
        state.selected = keep;
        state.step(0);
        self.overlay = Some(Overlay::SyncPanel(state));
    }

    fn restore_merged_text(&mut self, entry: &MergedEditDto) -> Result<(), String> {
        let page = node_id(&entry.page_id).ok_or("bad page id")?;
        let node = node_id(&entry.block_id).ok_or("bad block id")?;
        // A pending local save would be overwritten by the reload below.
        self.flush_pending_save();
        let hlc = self.hlc.clone();
        outl_actions::edit_text(&mut self.workspace, &hlc, node, &entry.ours)
            .map_err(|e| e.to_string())?;
        outl_actions::apply_page_md_with_sidecar_guarded(
            &self.workspace,
            &self.workspace_root,
            page,
        )
        .map_err(|e| e.to_string())?;
        self.load_current();
        Ok(())
    }
}

/// Sync panel keys: move, open, undo a merge.
pub(crate) fn handle_key(app: &mut App, key: KeyEvent) -> anyhow::Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.overlay = None,
        KeyCode::Enter => app.open_selected_merge(),
        KeyCode::Char('u') => app.undo_selected_merge(),
        KeyCode::Up | KeyCode::Char('k') => app.move_sync_cursor(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_sync_cursor(1),
        _ => {}
    }
    Ok(false)
}

fn page_slug(ws: &outl_core::workspace::Workspace, page_id: &str) -> Option<String> {
    outl_actions::page_meta(ws, node_id(page_id)?).map(|m| m.slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: u64) -> MergedEditDto {
        MergedEditDto {
            id: n.to_string(),
            page_id: String::new(),
            page_title: String::new(),
            block_id: String::new(),
            ours: String::new(),
            merged: String::new(),
            peer_actor: String::new(),
            at_ms: n,
        }
    }

    #[test]
    fn the_panel_reads_the_shared_log_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let actor = outl_core::id::ActorId::new();
        merges::record(dir.path(), actor, vec![entry(1), entry(2)]);
        let panel = SyncPanelState::load(dir.path(), actor);
        let ids: Vec<_> = panel.merged.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["2", "1"]);
        assert!(panel.pending.is_empty(), "never synced, nothing pending");
    }

    #[test]
    fn the_cursor_clamps_including_on_an_empty_log() {
        let mut p = SyncPanelState::default();
        p.step(1);
        assert_eq!(p.selected, 0);
        p.merged = vec![entry(1), entry(2)];
        p.step(5);
        assert_eq!(p.selected, 1);
        p.step(-9);
        assert_eq!(p.selected, 0);
    }
}
