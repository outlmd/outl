//! TUI runtime — `pub fn run` and the boot sequence it drives. The
//! bits that turn a workspace path into a running interactive program,
//! and put the terminal back the way it was found afterwards.
//!
//! State definitions live in [`crate::state`]; everything that touches
//! the `App` in response to a key event lives in [`crate::input`]; the
//! draw side lives in [`crate::view`].
//!
//! ## Module layout
//!
//! | Submodule       | What's in it                                                  |
//! |-----------------|---------------------------------------------------------------|
//! | `mod.rs` (here) | `run`, the alt-screen bracket around it, the loop's constants |
//! | `workspace`     | Locks, device actor, storage backend, snapshot + LRU policy   |
//! | `preset`        | Which theme preset this launch resolves to                    |
//! | `event_loop`    | Poll, draw, drain the coalesced save, route one keystroke     |
//! | `terminal`      | Is stdout a terminal, and the panic-time restore hook         |
//! | `logging`       | Send every dependency's `tracing` output to a file, not here  |

use anyhow::{Context, Result};
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::path::Path;
use std::time::Duration;

mod event_loop;
mod logging;
mod preset;
mod terminal;
mod workspace;

use event_loop::event_loop;
use logging::install_silent_log_subscriber;
use preset::resolve_theme;
use terminal::{install_panic_restore_hook, is_tty};
use workspace::open_workspace;

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
    // `load_result`, not `load`: a `config.toml` that failed to parse means
    // every preference below is a default the user never chose, and a
    // `tracing::warn!` in a log file is not a way to tell them (issue #284).
    // The verdict rides down to the status line.
    // The sentence itself belongs to `outl-config`, not here: `outl doctor`
    // prints the same one, and two copies of it drift.
    let global = outl_config::load_result();
    let config_warning = global.notice();
    let global_cfg = &global.config;
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
    let theme = resolve_theme(theme_override, &cfg, global_cfg);
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
        global_cfg.tui.icons,
        config_warning,
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
