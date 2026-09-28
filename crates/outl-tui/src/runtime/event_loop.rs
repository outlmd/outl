//! The event loop: poll the background workers, draw, drain the
//! coalesced save, then route one keystroke.
//!
//! Everything here runs once per iteration, so the ordering between
//! those four is load-bearing and commented where it is not obvious.
//! The key routing itself belongs to [`crate::input`]; this module only
//! decides which handler a keystroke reaches, and which keys never get
//! that far (`Ctrl+C`, `Ctrl+S`, `Ctrl+L`, the help popup's close keys).

use super::{MAX_SAVE_DEFER, POLL_INTERVAL, POLL_INTERVAL_PENDING_INDEX};
use crate::input::{handle_insert_key, handle_normal_key, handle_overlay_key, handle_visual_key};
use crate::state::{App, Mode};
use crate::theme::Theme;
use crate::view::render_app;
use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use outl_core::id::ActorId;
use outl_core::workspace::Workspace;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::Stdout;
use std::path::PathBuf;
use std::time::Duration;

// Private startup orchestrator — the "too many arguments" ergonomics
// lint doesn't apply to a one-call setup seam; each arg is a distinct
// boot input threaded from `run`.
#[allow(clippy::too_many_arguments)]
pub(super) fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    workspace_root: PathBuf,
    workspace: Workspace,
    actor: ActorId,
    theme: Theme,
    shared_workspace: bool,
    backlinks_newest_first: bool,
    icon_style: outl_config::TuiIconStyle,
    config_warning: Option<String>,
) -> Result<()> {
    let mut app = App::new(
        workspace_root,
        workspace,
        actor,
        theme,
        shared_workspace,
        icon_style,
    )?;
    // Apply the persisted backlinks direction (issue #142); the field
    // only feeds the render path, so setting it post-construction is
    // enough and keeps it out of `App::new`'s already-long signature.
    app.backlinks_newest_first = backlinks_newest_first;
    // An unreadable global config, on the first frame. Set post-construction
    // for the same reason as the line above, and last so it wins over
    // whatever `App::new` left in the status line.
    if let Some(warning) = config_warning {
        app.status = warning;
    }
    loop {
        // Pick up the background index build if it finished since the
        // last frame. Non-blocking; costs ~one channel try_recv.
        app.poll_index_updates();
        // Same for the background backlink-index build: swap it in when
        // the worker finishes so the "Linked from" panel + footer count
        // fill in without blocking the open.
        app.poll_backlink_index_updates();
        // Pick up any peer ops the jsonl poller saw arrive via iCloud
        // (or another sync transport). Reopens the workspace from
        // disk so the merged op log shows up in the next render.
        app.poll_jsonl_updates();
        // Reconcile `.md` files dropped in by importers (Roam, Logseq)
        // or edited externally (vim, VS Code). The scanner picks them
        // up in the background; we emit Create/Move/Edit ops here on
        // the main thread.
        app.poll_orphan_md_updates();
        // Fire any `remind::` that came due. Runs after the peer-ops
        // and orphan polls on purpose: a rule (or a snooze) that just
        // arrived from another device should be honoured on this tick,
        // not the next one.
        app.deliver_due_reminders();
        // Sweep expired toasts so they don't linger on screen past
        // their lifetime. Cheap O(n) over the small toast stack.
        app.prune_toasts();

        terminal.draw(|f| render_app(f, &mut app)).context("draw")?;
        // Render-first coalesced save: the edit is already on screen.
        // Now drain the persist (`render → write → reconcile_md →
        // fsync`) — but only when it won't stall the user's next
        // keystroke. If a key is already waiting in the terminal buffer
        // (the user is mid-burst), skip it and let the burst flow;
        // MAX_SAVE_DEFER forces the flush once the edit has waited too
        // long so it can't linger unpersisted.
        if app.has_pending_save() {
            let input_waiting = event::poll(Duration::ZERO).unwrap_or(false);
            let overdue = app
                .pending_save_age()
                .is_some_and(|age| age >= MAX_SAVE_DEFER);
            if overdue || !input_waiting {
                app.flush_pending_save();
            }
        }
        // Wait for a keystroke for up to POLL_INTERVAL. If nothing
        // arrives, take that opportunity to check whether the `.md`
        // changed under us (external editor saved). This is the
        // simplest hot-reload path — no filesystem watcher, no
        // background thread — and good enough at human latency.
        //
        // While a background index rebuild is in flight, shorten the
        // timeout so the freshly-built `WorkspaceIndex` shows up in
        // the UI within ~16 ms of arriving (instead of waiting up to
        // 750 ms for the next external-edit poll).
        let poll_timeout = if app.has_pending_index() || app.has_pending_backlink_index() {
            POLL_INTERVAL_PENDING_INDEX
        } else {
            POLL_INTERVAL
        };
        if !event::poll(poll_timeout).unwrap_or(false) {
            app.check_external_changes();
            continue;
        }
        let key = match event::read()? {
            Event::Key(k) => k,
            Event::Paste(text) => {
                // Bracketed-paste payload — convert markdown bullets
                // to outl blocks via the shared paste pipeline.
                app.check_external_changes();
                app.paste_external(text);
                continue;
            }
            Event::Mouse(m) => {
                // Only delivered when `[tui] mouse_capture` is on. Drives
                // click-select, wheel-scroll, and drag-select-then-copy.
                app.handle_mouse(m);
                continue;
            }
            _ => continue,
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        // Before processing the keystroke, sync with disk. If a
        // background editor wrote between polls, we want the keystroke
        // to act on the *new* page state, not the stale one.
        app.check_external_changes();
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            // Commit any pending insert before exiting, then persist the
            // coalesced save so a quit never drops the last edit.
            if matches!(app.mode, Mode::Insert { .. }) {
                app.commit_insert();
            }
            app.flush_pending_save();
            return Ok(());
        }

        // Universal: if the help popup is up, intercept the obvious
        // "close it" keys before they reach any mode-specific handler.
        // The popup is a `bool` flag (not an `Overlay`) so the overlay
        // close path doesn't catch it. Also resets `help_scroll` so
        // reopening starts from the top instead of a stale offset.
        if app.show_help
            && matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
            )
        {
            app.show_help = false;
            app.help_scroll = 0;
            continue;
        }

        // Universal `Ctrl+S` = save. Works in any mode.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
            if matches!(app.mode, Mode::Insert { .. }) {
                app.commit_insert();
            } else {
                app.save();
            }
            // Explicit `Ctrl+S` means "persist now" — drain the coalesced
            // save instead of leaving it for the idle drain.
            app.flush_pending_save();
            app.toast(crate::state::ToastKind::Success, "saved");
            continue;
        }

        // Universal `Ctrl+L` = refresh workspace (re-read from disk).
        // Useful when another editor changes files behind us.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('l') {
            if matches!(app.mode, Mode::Insert { .. }) {
                app.commit_insert();
            }
            app.refresh_workspace();
            app.status = "refreshed".into();
            continue;
        }

        // Overlays steal the keystream while open.
        if app.overlay.is_some() {
            if handle_overlay_key(&mut app, key)? {
                app.flush_pending_save();
                return Ok(());
            }
            continue;
        }

        match app.mode {
            Mode::Normal => {
                if handle_normal_key(&mut app, key)? {
                    // Quitting — persist the coalesced save first.
                    app.flush_pending_save();
                    return Ok(());
                }
            }
            Mode::Insert { .. } => handle_insert_key(&mut app, key)?,
            Mode::Visual { .. } => handle_visual_key(&mut app, key)?,
        }

        // Single post-mutation point: a keystroke may have appended ops
        // to the log (edit, indent, TODO toggle, …). Dispatch them to
        // plugins' `onOp` hooks. Cheap + idempotent when nothing
        // changed — the host short-circuits on an unchanged log length,
        // so this is safe to call after every key. A hook that mutates
        // the workspace re-projects `.md` and reparses inside
        // `run_plugin_op_hooks`.
        app.run_plugin_op_hooks();
    }
}
