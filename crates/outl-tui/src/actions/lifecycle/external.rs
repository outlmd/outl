//! Detect that an external process (vim, vscode, `outl serve`) edited
//! the currently open `.md` and pull the new content in.
//!
//! Lives apart from `peer_sync` because the trigger is different: a
//! peer write lands in `ops-<actor>.jsonl` and we react via the
//! channel; an external editor writes the `.md` directly and we
//! detect it by `mtime` polling the open file. Same downstream
//! effect (workspace re-sync), but different mechanism and different
//! mode-handling (peer reload is deferred during Insert; external
//! reload toasts a warning instead).

use crate::state::{App, Mode};

use super::file_mtime;

impl App {
    /// Detect that the current `.md` was edited by another process
    /// (vim, vscode, `outl serve`) since we last loaded or saved it,
    /// and pull the new content in.
    ///
    /// Behaviour:
    /// - No mtime change → returns `false`, nothing happens.
    /// - Changed and we're in Insert mode → returns `true` and writes
    ///   a warning to the status line. We refuse to clobber the
    ///   user's in-flight edit; they decide how to resolve.
    /// - Changed and we're in Normal/Visual → silently reload + reset
    ///   the selection clamp + rebuild the workspace index. Returns
    ///   `true` so the caller knows a redraw is in order.
    pub(crate) fn check_external_changes(&mut self) -> bool {
        let path = self.current_path();
        let Some(disk) = file_mtime(&path) else {
            return false;
        };
        let Some(last) = self.last_mtime else {
            // First time seeing the file — record and move on.
            self.last_mtime = Some(disk);
            return false;
        };
        if disk <= last {
            return false;
        }

        if matches!(self.mode, Mode::Insert { .. }) {
            let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("file");
            // Toast the warning instead of the status line: this is a
            // conflict the user needs to acknowledge, but the footer
            // hint is more useful for the chord prompt next to it.
            self.toast(
                crate::state::ToastKind::Warning,
                format!("external edit on {fname} — Ctrl+L to reload"),
            );
            // Don't update last_mtime — we'll keep warning until the
            // user explicitly resolves it.
            return true;
        }

        self.load_current();
        // External edit changes one file — incremental patch is enough
        // to bring the index in sync. (A full rebuild is the wrong
        // tool: it would block on rescanning every other page that
        // didn't change.)
        let cur_path = self.current_path();
        self.index.patch_page(&cur_path, &self.page);
        self.bump_render_epoch();
        self.toast(crate::state::ToastKind::Info, "reloaded from disk");
        true
    }
}
