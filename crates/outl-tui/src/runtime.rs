//! TUI runtime — `pub fn run`, the event loop, terminal lifecycle, and
//! the panic-restore hook. The bits that turn a workspace path into a
//! running interactive program.
//!
//! State definitions live in [`crate::state`]; everything that touches
//! the `App` in response to a key event lives in [`crate::input`]; the
//! draw side lives in [`crate::view`].

use crate::input::{handle_insert_key, handle_normal_key, handle_overlay_key, handle_visual_key};
use crate::state::{App, Mode};
use crate::theme::{self, Theme};
use crate::view::render_app;
use anyhow::{Context, Result};
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use outl_core::id::ActorId;
use outl_core::storage::{JsonlStorage, Storage};
use outl_core::workspace::Workspace;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::fs;
use std::io::Stdout;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How often the event loop wakes up to check for external `.md`
/// edits when no keypress arrived. Short enough that `nvim :w` shows
/// up "instantly" by human standards, long enough not to thrash the
/// filesystem.
const POLL_INTERVAL: Duration = Duration::from_millis(750);

/// Shorter poll cadence used while a background workspace-index
/// rebuild is in flight. The worker finishes in tens of ms on a small
/// vault but the result only reaches the UI on the next event-loop
/// iteration — without this, the user waits up to `POLL_INTERVAL`
/// (750 ms) to see backlinks fill in after opening the app. ~60 fps
/// while we wait costs nothing on idle hardware and disappears the
/// moment the index lands.
const POLL_INTERVAL_PENDING_INDEX: Duration = Duration::from_millis(16);

/// Longest a coalesced edit may sit unpersisted while the user keeps
/// typing. The save normally drains the instant the loop goes idle (no
/// keystroke waiting), so a burst of edits persists as soon as the user
/// pauses; this cap forces a flush mid-burst so an unsaved change can't
/// linger longer than this — bounding what a crash could lose.
const MAX_SAVE_DEFER: Duration = Duration::from_millis(600);

/// Run the TUI against the workspace at `path`.
///
/// Picks the active theme from `.outl/config.toml`'s `[theme] preset`
/// field if present, falling back to the default-dark palette.
pub fn run(path: &Path) -> Result<()> {
    run_with_theme_override(path, None)
}

/// Variant of [`run`] that accepts a `--theme` override from the CLI.
/// Pass `Some(name)` to force a particular preset; `None` defers to the
/// config file (or default).
pub fn run_with_theme_override(path: &Path, theme_override: Option<&str>) -> Result<()> {
    if !is_tty() {
        return Err(anyhow::anyhow!(
            "outl-tui requires an interactive terminal (stdout is not a TTY)"
        ));
    }

    // Cage every dependency that uses `tracing` (Steel, wasmtime,
    // notify, ...). Without this they print INFO lines straight onto
    // the TUI canvas, which looks like the terminal exploded. We send
    // everything to a per-workspace log file so debugging is still
    // possible — and silence the terminal entirely.
    install_silent_log_subscriber(path);

    let workspace_root = path.to_path_buf();
    // `_lock` and `_actor_lock` live through the entire TUI run:
    // - `_lock` is the shared workspace flock on `<root>/.outl/.lock`.
    // - `_actor_lock` is the exclusive per-actor write flock on
    //   `<root>/ops/.lock-<actor>` and keeps another `outl` from
    //   stealing this process's actor mid-session.
    //
    // The underscore prefix only silences the unused-binding lint —
    // both are RAII guards, dropped at the end of this function. If
    // a future refactor moves `event_loop` (or anything else that
    // mutates the workspace) into a different function, the locks
    // have to move with it. `open_workspace`'s doc spells out the
    // ownership contract.
    let (workspace, actor, cfg, _lock, _actor_lock) = open_workspace(&workspace_root)?;

    // No boot-time `apply_all_pages_md` here. The op log remains the
    // source of truth; `.md` is its projection. We deliberately skip
    // re-projecting at boot because peers and external editors (vim,
    // VS Code) may have written `.md` content the op log doesn't
    // know about yet — re-projecting blindly would clobber those
    // edits before the orphan scanner has a chance to fold them in
    // via `reconcile_md`.
    // Global config (`~/.config/outl/config.toml`) — shared with the
    // desktop client. We read both the theme fallback and the `[sync]`
    // transport selection from it; loading once keeps the two reads
    // consistent within a single launch.
    let global_cfg = outl_config::load();
    // Resolve the journal/clock timezone once for the whole process,
    // before the first `clock::today()` builds the initial journal view
    // (issue #107). No `[calendar] timezone` → OS local, as before.
    outl_actions::clock::init(global_cfg.calendar.timezone.as_deref());
    // Automatic local backups (`[backup]`, on by default). A detached
    // background thread, never the event loop: a snapshot walks the
    // whole workspace and forks `git`, so on a large graph it is
    // seconds — nothing that may sit between a keystroke and its frame.
    // It also creates the repository on first run, which is safe
    // precisely because that repository lives outside the workspace
    // (`outl_actions::backup`), so it can't turn the user's notes folder
    // into a git repo behind their back. Missing a snapshot at quit is
    // deliberate: the interval floor is read back out of git, so the
    // next launch takes it.
    outl_actions::backup::spawn_auto_pass(
        workspace_root.clone(),
        global_cfg.backup.enabled,
        global_cfg.backup.interval_minutes,
    );
    let theme = resolve_theme(theme_override, &cfg, &global_cfg);
    // Backlinks list direction (issue #142). Read once at boot; the
    // `Ctrl+O` toggle persists changes back to `config.toml`.
    let backlinks_newest_first = global_cfg.display.backlinks_order.newest_first();
    // `shared_workspace` gates the peer-sync threads (iroh transport + the
    // filesystem poller). JsonlStorage is the ONLY persistent backend
    // (sqlite was removed in 0.5.0), so a workspace is shareable unless its
    // config *explicitly* pins a non-jsonl storage. A GUI/sync-created
    // workspace — and the CLI/TUI lazy-seeded config — omit the `storage`
    // key entirely; treat that absence as the jsonl default, NOT as "not
    // shared". The old `== Some("jsonl")` check made the TUI silently run
    // with NO peer sync on exactly those workspaces (the "TUI ↔ mobile
    // doesn't sync" bug: `~/outl-p2p/.outl/config.toml` has no `storage`
    // line, so the TUI never started a transport or a poller).
    let shared_workspace = cfg
        .get("workspace")
        .and_then(|w| w.get("storage"))
        .and_then(|s| s.as_str())
        .is_none_or(|s| s == "jsonl");

    // Install the panic hook BEFORE switching to raw mode. If
    // anything panics from here on — bug in the render path, OOM —
    // the hook runs first, restores the terminal, then chains to the
    // default handler so the user still sees the panic message.
    install_panic_restore_hook();

    enable_raw_mode().context("enabling raw mode")?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen).context("entering alt screen")?;
    // Ask the terminal to deliver pastes as a single `Event::Paste`
    // instead of streaming the clipboard contents as keystrokes (which
    // would interleave `\n` with the user's keymap and produce
    // surprise commits + new blocks). Best-effort: terminals without
    // bracketed-paste support silently ignore the CSI sequence.
    let _ = execute!(stdout, EnableBracketedPaste);

    // Opt-in mouse capture (`[tui] mouse_capture`). When on, the app
    // owns the mouse — drag-select copies markdown, the wheel moves the
    // selection — at the cost of the terminal's native text selection.
    // Default off, so a normal launch leaves the terminal's selection
    // untouched. Best-effort: terminals without mouse support ignore it.
    let mouse_capture = global_cfg.tui.mouse_capture;
    if mouse_capture {
        let _ = execute!(stdout, EnableMouseCapture);
    }

    // Ask the terminal to report enhanced key events (kitty keyboard
    // protocol). When supported, this lets us distinguish `Shift+Enter`
    // from `Enter`, `Ctrl+Enter` from `Enter`, and so on — essential
    // for multi-line editing inside a single block. Terminals that
    // don't support it (Terminal.app, plain xterm) silently ignore the
    // CSI sequence; we still have Alt+Enter as a portable fallback.
    let enhanced_keys = supports_keyboard_enhancement().unwrap_or(false);
    if enhanced_keys {
        let _ = execute!(
            stdout,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS,
            )
        );
    }

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("creating terminal")?;

    let result = event_loop(
        &mut terminal,
        workspace_root,
        workspace,
        actor,
        theme,
        shared_workspace,
        backlinks_newest_first,
    );

    if enhanced_keys {
        let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    }
    if mouse_capture {
        let _ = execute!(terminal.backend_mut(), DisableMouseCapture);
    }
    let _ = execute!(terminal.backend_mut(), DisableBracketedPaste);
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    result
}

pub(crate) fn is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

/// Install a process-wide panic hook that restores the terminal before
/// chaining to the previous handler.
///
/// Without this, a panic mid-render leaves the user staring at a
/// garbled terminal in raw mode with no cursor — they have to `reset`
/// blind to recover. Calling this is idempotent in spirit (only the
/// first call chains the real default hook).
fn install_panic_restore_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Restore the terminal state. Errors are ignored — we're
        // already panicking; nothing useful to do with a second
        // failure.
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        let _ = execute!(std::io::stdout(), crossterm::cursor::Show);
        previous(info);
    }));
}

/// Resolve the active theme.
///
/// Precedence (first hit wins):
/// 1. `--theme <preset>` CLI override.
/// 2. `[theme] preset = "..."` in the **per-workspace**
///    `.outl/config.toml`.
/// 3. `[theme] preset = "..."` in the **global**
///    `~/.config/outl/config.toml` (shared with the desktop client
///    via the `outl-config` crate).
/// 4. [`theme::default_theme`].
///
/// An unknown name falls through silently to the next level. The
/// caller can surface the choice via the status line if it cares.
fn resolve_theme(
    cli_override: Option<&str>,
    cfg: &toml::Value,
    global: &outl_config::Config,
) -> Theme {
    if let Some(name) = cli_override {
        if let Some(t) = theme::by_name(name) {
            return t;
        }
    }
    if let Some(preset) = cfg
        .get("theme")
        .and_then(|t| t.get("preset"))
        .and_then(|v| v.as_str())
    {
        if let Some(t) = theme::by_name(preset) {
            return t;
        }
    }
    // Global fallback — same TOML file the desktop reads / writes,
    // so changing the theme in the desktop's Settings modal
    // propagates to the next `outl-tui` launch automatically (and
    // vice versa). Passed in pre-loaded so the launch reads the file
    // once for both theme and `[sync]`.
    if let Some(t) = theme::by_name(&global.theme.preset) {
        return t;
    }
    theme::default_theme()
}

/// Open the workspace at `root` and return everything the TUI needs
/// to run for the rest of its lifetime.
///
/// **Lock ownership is on the caller.** The returned tuple's last
/// two fields are RAII guards (`WorkspaceLock` is shared,
/// `ActorWriteLock` is exclusive per actor). They must outlive every
/// write against the workspace — that is, the entire TUI session.
/// In practice that means binding them in
/// [`run_with_theme_override`]'s top scope so they drop after the
/// event loop returns. **Don't** pass the workspace into another
/// function without forwarding the locks, and **don't** rebind the
/// guards in a tighter scope.
///
/// Returns `(workspace, actor, cfg, workspace_lock, actor_write_lock)`.
fn open_workspace(
    root: &Path,
) -> Result<(
    Workspace,
    ActorId,
    toml::Value,
    outl_core::WorkspaceLock,
    outl_core::ActorWriteLock,
)> {
    // Shared workspace lock — every well-behaved `outl` opener piles
    // on. Concurrent TUI + MCP server + sink-outl plugin is the
    // supported case; per-actor write isolation comes from the
    // ActorWriteLock below.
    let lock = outl_core::WorkspaceLock::acquire(root)
        .with_context(|| format!("could not acquire workspace lock at {}", root.display()))?;

    let paths = outl_ws::layout::Paths::at(root.to_path_buf());
    if !paths.dot_outl.is_dir() {
        anyhow::bail!(
            "no outl workspace at {} — run `outl init` first",
            root.display()
        );
    }
    // `read_or_init_config` seeds a config when the `.outl/` dir exists
    // but `config.toml` doesn't — a workspace created by a GUI client or
    // by P2P sync, which only seed `.outl/workspace-id`. Going through
    // `outl-ws` instead of parsing the TOML here is what keeps the TUI
    // and the CLI on one schema (the hand-rolled seed used to omit
    // `created_at`, which `outl-ws` then refused to deserialize).
    let cfg = outl_ws::layout::read_or_init_config(&paths)?;
    // Which actor this DEVICE owns. Read from the device store, never
    // from `.outl/config.toml`: that file rides the file-sync surface,
    // and two devices reading one actor id is silent op loss. See
    // `outl_ws::actor`.
    let device_actor = outl_ws::actor::resolve_device_actor(
        &paths,
        &cfg,
        &outl_core::device::DeviceStore::open_default(),
    )?;
    // `outl_ws::layout::Config` models only the `[workspace]` section;
    // `resolve_theme` reads the optional per-workspace `[theme]` off the
    // raw document. A malformed file degrades to "no override" — the
    // theme falls back to the global config, never a failed open.
    let cfg_raw: toml::Value = fs::read_to_string(&paths.config)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_else(|| toml::Value::Table(Default::default()));

    let ops_dir = paths.ops.clone();
    fs::create_dir_all(&ops_dir)
        .with_context(|| format!("creating ops dir at {}", ops_dir.display()))?;

    // Exclusive per-actor write lock. Falls back to an ephemeral
    // actor when another `outl` on this machine already owns the device
    // actor — that's how a TUI + MCP server share the same workspace
    // without racing on `ops-<device_actor>.jsonl`.
    let (actor_lock, actor) = outl_core::resolve_write_actor(&ops_dir, device_actor)
        .with_context(|| format!("acquiring per-actor write lock at {}", ops_dir.display()))?;
    if actor != device_actor {
        tracing::info!(
            "another outl process owns the device actor {device_actor}; this TUI writes under ephemeral actor {actor}"
        );
    }

    // JsonlStorage is the only persistent backend (see CHANGELOG
    // 0.5.0). The directory is created above because sync transports
    // sometimes garbage-collect empty dirs between runs. Not named
    // `.ops/` because iCloud skips dotted paths.
    let storage: Box<dyn Storage> = Box::new(
        JsonlStorage::open(ops_dir.clone(), actor)
            .with_context(|| format!("opening jsonl storage at {}", ops_dir.display()))?,
    );
    let mut ws = Workspace::open_with_storage(actor, storage, Some(root.to_path_buf()))?;
    let lru_cap = outl_config::load().storage.lru_cap;
    // Register per-page shards BEFORE applying the LRU cap. Shards
    // open unbounded (cap = 0) so the reboot replays the full history.
    outl_actions::storage_scope::register_per_page_storages(&mut ws, &ops_dir, actor, root);
    if ws.has_page_storages() {
        ws.reboot_with_all_storages()?;
    }
    // Shed cold history AFTER the materialized tree is complete.
    ws.apply_lru_cap(lru_cap);
    // Snapshot boot-cache policy (#128/#109): a long-lived client writes
    // background snapshots so the next open boots from one instead of
    // replaying the whole op log. `Drop for App` also flushes a final
    // snapshot on exit. Defaults (enabled, 10k) unless `[snapshot]`
    // overrides them.
    let snap_cfg = outl_config::load().snapshot;
    ws.set_snapshot_policy(snap_cfg.enabled, snap_cfg.op_threshold);
    Ok((ws, actor, cfg_raw, lock, actor_lock))
}

// Private startup orchestrator — the "too many arguments" ergonomics
// lint doesn't apply to a one-call setup seam; each arg is a distinct
// boot input threaded from `run`.
#[allow(clippy::too_many_arguments)]
fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    workspace_root: PathBuf,
    workspace: Workspace,
    actor: ActorId,
    theme: Theme,
    shared_workspace: bool,
    backlinks_newest_first: bool,
) -> Result<()> {
    let mut app = App::new(workspace_root, workspace, actor, theme, shared_workspace)?;
    // Apply the persisted backlinks direction (issue #142); the field
    // only feeds the render path, so setting it post-construction is
    // enough and keeps it out of `App::new`'s already-long signature.
    app.backlinks_newest_first = backlinks_newest_first;
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

/// Wire up `tracing` so nothing prints onto the TUI canvas, but logs
/// stay debuggable.
///
/// Order matters here:
/// 1. **`tracing-log` bridge** — Steel and a few other deps emit via
///    the older `log` crate. Without the bridge `tracing-subscriber`
///    never sees those records and the default `log` impl falls back
///    to printing on stderr — straight onto our TUI canvas. Install
///    the bridge *before* the subscriber so the subscriber catches
///    every record.
/// 2. **`tracing-subscriber`** with a file appender at
///    `<workspace>/.outl/tui.log`. `tail -f` it when something looks
///    off.
/// 3. Best-effort installation: if a global was already set (rare,
///    but possible when a dep front-runs us), `try_init` swallows
///    the error — the TUI keeps running with whatever subscriber
///    was registered first.
fn install_silent_log_subscriber(workspace_root: &Path) {
    use std::fs::OpenOptions;
    use tracing_log::LogTracer;
    use tracing_subscriber::{fmt, EnvFilter};

    // Bridge `log` → `tracing`. Idempotent: re-init returns Err which
    // we ignore.
    let _ = LogTracer::init();

    let log_path = workspace_root.join(".outl").join("tui.log");
    let _ = std::fs::create_dir_all(log_path.parent().unwrap_or(workspace_root));
    let file = OpenOptions::new().create(true).append(true).open(&log_path);

    // `RUST_LOG` still wins if the user wants verbose output — useful
    // when reporting a bug. Default = warn; everything noisier gets
    // dropped on the floor.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));

    let builder = fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_target(true);

    let result = match file {
        Ok(f) => builder.with_writer(f).try_init(),
        // No log file? Still install a sink so the canvas stays clean.
        Err(_) => builder.with_writer(std::io::sink).try_init(),
    };
    // Errors here mean a global subscriber was set by an earlier
    // caller — fine, just move on.
    let _ = result;
}
