//! Land the captures `outl capture` left in the inbox (RFC 0363).
//!
//! The terminal is where `outl capture` is run from — by hand, from a
//! shell alias, from a macOS Service or a hotkey daemon — so the TUI not
//! draining would make the command's promise false for the client most
//! likely to be open. Every GUI client drains on mount and on every
//! return to the foreground; the TUI drains once, at boot, which is the
//! only moment it has (nothing here watches the inbox).
//!
//! `outl_actions::capture` owns the landing: the journal for the day the
//! capture was *made*, in capture order, idempotent on a crash. This
//! module is the three things around it the TUI has to do itself —
//! project the pages that gained blocks, reload the open view off that
//! projection, and say so.

use std::path::Path;
use std::sync::Arc;

use outl_actions::capture::{capture_day, capture_page, drain_captures, land_capture};
use outl_actions::{commit_page, ActionError, CommitHooks, SyncTransport};
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use crate::state::{App, ToastKind};

/// The TUI's half of `commit_page` for a landing capture: guarded
/// projection (root invariant 8), the peer announce, and a flag for the
/// backlink rebuild, which needs `&mut App` and so runs after the drain.
///
/// Shared with the page merge (`actions/merge`), which commits the same
/// way: it is a projection-only commit, so a second copy would be one
/// more place to forget the guard.
pub(crate) struct CaptureCommit<'a> {
    pub(crate) root: &'a Path,
    pub(crate) hlc: &'a HlcGenerator,
    pub(crate) transport: Option<&'a Arc<dyn SyncTransport>>,
    pub(crate) backlinks_stale: bool,
    pub(crate) projection_failure: Option<ActionError>,
}

impl CommitHooks for CaptureCommit<'_> {
    fn project(&mut self, workspace: &Workspace, page: NodeId) {
        if let Err(e) = outl_actions::apply_page_md_with_sidecar_guarded(workspace, self.root, page)
        {
            self.projection_failure = Some(e);
        }
    }

    fn invalidate_backlinks(&mut self) {
        self.backlinks_stale = true;
    }

    fn announce(&mut self, workspace: &Workspace, page: NodeId) {
        let Some(transport) = self.transport else {
            return;
        };
        if let Some(meta) = outl_actions::page_meta(workspace, page) {
            transport.announce_local_ops(&meta.slug, self.hlc.next());
        }
    }
}

/// What to say about files the drain could not read.
///
/// Names the folder, because the only thing the user can do about one is
/// look at it. It deliberately does **not** point at `outl doctor`:
/// doctor has no inbox check, and sending someone to a command that
/// reports nothing is worse than saying where the files are.
fn unreadable_notice(n: usize, root: &Path) -> String {
    let dir = outl_actions::capture::inbox_in_workspace(root);
    if n == 1 {
        format!("1 capture could not be read, left in {}", dir.display())
    } else {
        format!("{n} captures could not be read, left in {}", dir.display())
    }
}

impl App {
    /// Drain the workspace inbox and show what landed.
    ///
    /// Called once from the runtime, **after** `clock::init`: the day a
    /// capture belongs to is resolved in the configured timezone, so
    /// draining earlier would file a late-evening capture under the
    /// wrong day.
    ///
    /// Each capture goes through `commit_page`, so its page is projected
    /// and announced before `drain_captures` deletes its file. A later
    /// failure leaves that file and the rest for the next launch, and
    /// what already landed is still refreshed and reported.
    pub(crate) fn drain_capture_inbox(&mut self) {
        let dir = outl_actions::capture::inbox_in_workspace(&self.workspace_root);
        // `unreadable` is read here and reported below. A file the drain
        // cannot parse is left in the inbox, so without a word from this
        // client the capture simply never appears and the only trace is
        // a `tracing::warn!` in a log a TUI user never opens. Returning
        // early on `waiting.is_empty()` alone was silent in exactly the
        // case where every file in the inbox is broken.
        let (waiting, unreadable) = outl_actions::capture::pending_captures(&dir);
        if waiting.is_empty() && unreadable == 0 {
            return;
        }
        if waiting.is_empty() {
            self.toast(
                ToastKind::Warning,
                unreadable_notice(unreadable, &self.workspace_root),
            );
            return;
        }
        // The open view may be the journal a capture lands in; snapshot
        // it so `u` takes the landing back like any other edit.
        self.snapshot_for_undo();

        let mut hooks = CaptureCommit {
            root: &self.workspace_root,
            hlc: &self.hlc,
            transport: self.sync_transport.as_ref(),
            backlinks_stale: false,
            projection_failure: None,
        };
        let workspace = &mut self.workspace;
        let hlc = &self.hlc;
        let mut landed = 0usize;
        let mut warnings = Vec::new();
        let result = drain_captures(&dir, capture_day, |capture, day| {
            let page = capture_page(workspace, hlc, day)?;
            commit_page(workspace, &mut hooks, page, |ws| {
                land_capture(ws, hlc, page, capture)
            })?;
            if let Some(e) = hooks.projection_failure.take() {
                // The op is durable; what failed is the projection.
                warnings.push(format!("capture projection: {e}"));
            }
            landed += 1;
            Ok::<_, ActionError>(page)
        });
        let backlinks_stale = hooks.backlinks_stale;
        if let Err(e) = result {
            warnings.push(format!("capture inbox: {e}"));
        }
        if unreadable > 0 {
            warnings.push(unreadable_notice(unreadable, &self.workspace_root));
        }

        if landed > 0 {
            if backlinks_stale {
                self.spawn_backlink_index_rebuild();
            }
            self.refresh_workspace();
        }
        for w in warnings {
            self.toast(ToastKind::Warning, w);
        }
        if landed == 0 {
            self.undo.pop();
            return;
        }
        self.toast(
            ToastKind::Info,
            if landed == 1 {
                "added 1 capture to your journal".to_string()
            } else {
                format!("added {landed} captures to your journal")
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_core::{ActorId, Workspace};
    use tempfile::TempDir;

    fn fresh_app() -> (App, TempDir) {
        let dir = TempDir::new().unwrap();
        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
        let app = App::new_for_tests(
            dir.path().to_path_buf(),
            ws,
            actor,
            crate::theme::default_theme(),
            false,
        )
        .unwrap();
        (app, dir)
    }

    fn messages(app: &App) -> Vec<&str> {
        app.toasts.iter().map(|t| t.message.as_str()).collect()
    }

    /// The three things this module's doc promises: the capture becomes a
    /// block, the page it landed in is projected to `.md` (the TUI reads
    /// a view by parsing that file, so without this the block is
    /// invisible), and the user is told.
    #[test]
    fn a_capture_lands_projects_and_is_reported() {
        let (mut app, dir) = fresh_app();
        let inbox = outl_actions::capture::inbox_in_workspace(dir.path());
        outl_actions::capture::write_capture(
            &inbox,
            "call the venue",
            "cli",
            chrono::Utc::now().timestamp_millis(),
        )
        .unwrap();

        app.drain_capture_inbox();

        assert_eq!(messages(&app), ["added 1 capture to your journal"]);
        assert_eq!(
            std::fs::read_dir(&inbox).unwrap().count(),
            0,
            "the file is deleted once its op is durable"
        );
        let projected = std::fs::read_dir(dir.path().join("journals"))
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
                    .filter_map(|e| std::fs::read_to_string(e.path()).ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        assert!(
            projected.iter().any(|md| md.contains("call the venue")),
            "the journal `.md` must hold the capture, got {projected:?}"
        );
    }

    /// The silent case this module used to have: every file in the inbox
    /// is unreadable, so there is nothing to land and the early return
    /// fired with nothing said. A capture that never appears and is never
    /// mentioned is the failure RFC 0363 exists to prevent.
    #[test]
    fn an_inbox_of_unreadable_files_still_reports() {
        let (mut app, dir) = fresh_app();
        let inbox = outl_actions::capture::inbox_in_workspace(dir.path());
        std::fs::create_dir_all(&inbox).unwrap();
        let id = ulid::Ulid::generate();
        std::fs::write(inbox.join(format!("{id}.json")), "{not json").unwrap();

        app.drain_capture_inbox();

        let said = messages(&app);
        assert_eq!(said.len(), 1, "exactly one notice, got {said:?}");
        assert!(
            said[0].contains("could not be read") && said[0].contains("captures"),
            "the notice names the folder and the count, got {said:?}"
        );
        assert_eq!(
            std::fs::read_dir(&inbox).unwrap().count(),
            1,
            "an unreadable file is kept, never dropped"
        );
    }

    #[test]
    fn an_empty_inbox_says_nothing() {
        let (mut app, _dir) = fresh_app();
        app.drain_capture_inbox();
        assert!(messages(&app).is_empty());
    }
}
